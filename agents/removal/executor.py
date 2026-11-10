"""
Removal executor handles automated submission of data removal requests.
Supports web forms (Playwright), email requests, and California DROP API.
"""

import asyncio
import hashlib
import html
import re
import smtplib
import ssl
from dataclasses import dataclass
from email.mime.multipart import MIMEMultipart
from email.mime.text import MIMEText
from email.utils import make_msgid
from typing import Optional
from urllib.parse import urlparse
from playwright.async_api import async_playwright, Page, TimeoutError as PlaywrightTimeout
import httpx
import structlog

from shared.models import RemovalProcedure, IdentityContext
from shared.backend_client import BackendClient
from shared.config import get_config

log = structlog.get_logger()


def _host(url: str | None) -> str:
    """Host only, for logs: listing and opt-out URLs can embed a person's name or a token."""
    try:
        return urlparse(url or "").hostname or "unknown"
    except ValueError:
        return "unknown"


def _why(exc: BaseException) -> str:
    """
    A reason that is safe to log and to show the customer: the exception's type only.
    Exception text from HTTP, SMTP and browser libraries routinely contains URLs, addresses
    and response bodies, and URLs embed the customer's name.
    """
    return type(exc).__name__


# ---------------------------------------------------------------------------
# Reading the page a broker shows after a form is submitted
#
# Kept as plain functions of text so the rules can be tested without a browser. The
# executor's contract is "submitted" only on positive evidence; these rules are that
# evidence, so they err toward "not confirmed".
# ---------------------------------------------------------------------------

# A labelled identifier: "Reference: ABC-12345", "Confirmation number #98765", "Case #4521",
# "Request ID: XYZ789", "Your ticket is T-20481". The label must be followed by a ':' or '#'
# (or "is"), or by an explicit qualifier (number / no. / id / code); and the value must be 4-40
# characters of letters/digits/hyphens that include at
# least one digit. Ordinary prose therefore cannot match: "in case of error", "confirmation
# email", "reference our policy" and "case closed" have no label+separator+digit-bearing value.
# Only the label part is case-insensitive.
_LABEL = r"\b(?:confirmation|reference|ticket|tracking|request|case)"
_CONFIRMATION_REF = re.compile(
    # "Reference number 12345", "Request ID XYZ789", "reference no. 5521-AA": an explicit
    # qualifier makes the separator optional...
    rf"(?:(?i:{_LABEL}\s+(?:number\b|no\b\.?|id\b|code\b))\s*(?:[:#]+|\bis\b)?"
    # ...whereas a bare label ("Reference:", "Case #", "ticket is") needs the separator.
    rf"|(?i:{_LABEL})\s*(?:[:#]+|\bis\b))"
    r"\s*#?\s*"
    r"((?=[A-Za-z0-9-]*\d)[A-Za-z0-9][A-Za-z0-9-]{3,39})\b"
)

# Wording that means the broker did NOT accept the request. Checked against the whole
# page and it wins over everything else, including a reference that happens to be present.
_FAILURE_PHRASES = (
    "please try again",
    "try again later",
    "something went wrong",
    "an error occurred",
    "error occurred",
    "invalid",
    "could not be",
    "couldn't be",
    "unable to",
    "failed",
    "not found",
    "not accepted",
    "not been submitted",
    "was not submitted",
    "captcha",
    "verify you are human",
    "access denied",
)

# Wording that says a request was received. Deliberately specific: a bare "thank you" or
# "successfully" appears on error pages ("Thank you for contacting us. An error occurred").
_SUCCESS_PHRASES = (
    "has been submitted",
    "have been submitted",
    "was submitted",
    "submitted successfully",
    "successfully submitted",
    "successfully received",
    "request has been received",
    "request has been",
    "we have received your",
    "we've received your",
    "thank you for your request",
    "thank you for submitting",
    "check your email",
    "verification email",
)


@dataclass(frozen=True)
class PageVerdict:
    """What the result page says about the submission."""
    accepted: bool
    reference: Optional[str]
    reason: str  # short, constant text for logs; never page content


def find_confirmation_reference(text: str) -> Optional[str]:
    """A labelled confirmation identifier in visible page text, or None."""
    match = _CONFIRMATION_REF.search(text or "")
    return match.group(1) if match else None


def judge_result_page(text: str) -> PageVerdict:
    """
    Decide from the visible text of the page shown after submitting whether the broker
    accepted the request.

      * Failure wording anywhere means NOT accepted, even if a reference is also present
        (error pages often echo a request or case id).
      * Otherwise a labelled reference, or specific acceptance wording, means accepted.
      * Anything else is not accepted: the customer is told it could not be confirmed.
    """
    body = (text or "").lower()
    reference = find_confirmation_reference(text or "")
    if any(phrase in body for phrase in _FAILURE_PHRASES):
        return PageVerdict(False, None, "failure_wording")
    if reference:
        return PageVerdict(True, reference, "reference")
    if any(phrase in body for phrase in _SUCCESS_PHRASES):
        return PageVerdict(True, None, "acceptance_wording")
    return PageVerdict(False, None, "no_evidence")


class RemovalExecutor:
    """Executes removal requests via multiple channels (web, email, API)."""

    def __init__(self):
        self.backend = BackendClient()
        self.config = get_config()

    async def submit_removal_request(
        self,
        removal_request_id: str,
        customer_id: str,
        procedure: RemovalProcedure,
        identity: IdentityContext,
        source_url: str,
    ) -> dict:
        """
        Dispatches removal request to appropriate submission method.

        Returns:
            {
                "status": "submitted" | "failed",
                "confirmation_ref": str | None,
                "evidence_hash": str | None,
                "error": str | None
            }
        """
        log.info(
            "submit_removal",
            removal_request_id=removal_request_id,
            procedure_type=procedure.procedure_type,
            # Host only: listing URLs commonly embed the person's name.
            source_host=urlparse(source_url).hostname,
        )

        try:
            if procedure.procedure_type == "web_form":
                result = await self.submit_web_form(procedure, identity, source_url)
            elif procedure.procedure_type == "email":
                result = await self.submit_email_request(procedure, identity, source_url)
            elif procedure.procedure_type == "drop_api":
                result = await self.submit_drop_api(procedure, identity)
            else:
                return {
                    "status": "failed",
                    "confirmation_ref": None,
                    "evidence_hash": None,
                    "error": f"Unknown procedure type: {procedure.procedure_type}",
                }

            log.info("submission_complete", status=result.get("status"), procedure_type=procedure.procedure_type)
            return result

        except Exception as exc:
            log.error("submission_failed", procedure_type=procedure.procedure_type, error_type=_why(exc))
            return {
                "status": "failed",
                "confirmation_ref": None,
                "evidence_hash": None,
                "error": f"Submission failed ({_why(exc)})",
            }

    async def submit_web_form(
        self, procedure: RemovalProcedure, identity: IdentityContext, source_url: str
    ) -> dict:
        """
        Submits removal request via web form using Playwright.

        Generic implementation for MVP - in production, use site-specific handlers.

        Reliability contract: this returns "submitted" ONLY when there is positive
        evidence the broker accepted the request (a confirmation reference, or
        success wording on the resulting page). Anything else — a form that has
        changed shape, no field or submit control matched, a CAPTCHA, an ambiguous
        result page — returns "failed" with a reason, so the customer is told the
        truth and can retry, instead of being told a request went through when it
        may not have.
        """
        log.info("submit_web_form", opt_out_host=_host(procedure.opt_out_url))

        async with async_playwright() as p:
            browser = await p.chromium.launch(headless=True)
            try:
                page = await browser.new_page()
                await page.set_viewport_size({"width": 1280, "height": 1024})

                # Navigate to opt-out page
                await page.goto(procedure.opt_out_url, timeout=30000)
                await page.wait_for_load_state("networkidle", timeout=30000)

                # Do not attempt to defeat bot protection. A CAPTCHA means the
                # broker wants a human; report it and stop.
                if await self._has_captcha(page):
                    log.warning("web_form_captcha", opt_out_host=_host(procedure.opt_out_url))
                    return self._failed(
                        "CAPTCHA challenge on the opt-out page; this source needs "
                        "to be completed manually"
                    )

                # Fill form fields using generic selectors
                # In production: use procedure.form_fields for site-specific selectors
                fields_filled = await self._fill_generic_form(page, identity, source_url)
                if fields_filled == 0:
                    return self._failed(
                        "No recognizable form fields on the opt-out page (the page "
                        "layout may have changed)"
                    )

                # Submit form
                if not await self._submit_form(page):
                    return self._failed(
                        "No submit control found on the opt-out page (the page "
                        "layout may have changed)"
                    )

                # Wait for confirmation page
                await page.wait_for_load_state("networkidle", timeout=20000)
                await asyncio.sleep(2)  # Allow dynamic content to load

                # Capture screenshot as evidence
                screenshot_bytes = await page.screenshot(full_page=True)
                evidence_hash = self.generate_evidence_hash(screenshot_bytes)

                # Judge the page from what a person would read on it. One verdict decides:
                # failure wording vetoes acceptance, even when a reference is also shown.
                try:
                    page_text = await page.inner_text("body")
                except Exception:
                    page_text = ""
                verdict = judge_result_page(page_text)

                if not verdict.accepted:
                    log.warning("web_form_unconfirmed", evidence_hash=evidence_hash, reason=verdict.reason)
                    if verdict.reason == "failure_wording":
                        message = (
                            "The opt-out page reported a problem after the form was submitted "
                            "(the request was probably not accepted)"
                        )
                    else:
                        message = (
                            "Form was submitted but no confirmation was detected (the "
                            "request may not have been accepted)"
                        )
                    result = self._failed(message)
                    result["evidence_hash"] = evidence_hash
                    return result

                confirmation_ref = verdict.reference
                log.info(
                    "web_form_submitted",
                    confirmation_ref=confirmation_ref,
                    evidence_hash=evidence_hash,
                    reason=verdict.reason,
                )

                return {
                    "status": "submitted",
                    "confirmation_ref": confirmation_ref,
                    "evidence_hash": evidence_hash,
                    "error": None,
                }

            except PlaywrightTimeout as exc:
                log.warning("web_form_timeout", opt_out_host=_host(procedure.opt_out_url), error_type=_why(exc))
                return self._failed("The opt-out page timed out")
            finally:
                await browser.close()

    @staticmethod
    def _failed(reason: str) -> dict:
        """Standard failed-submission result."""
        return {
            "status": "failed",
            "confirmation_ref": None,
            "evidence_hash": None,
            "error": reason,
        }

    async def _has_captcha(self, page: Page) -> bool:
        """True if the page presents a CAPTCHA / human-verification widget."""
        selectors = [
            'iframe[src*="recaptcha"]',
            'iframe[src*="hcaptcha"]',
            'iframe[src*="challenges.cloudflare.com"]',
            ".g-recaptcha",
            ".h-captcha",
            ".cf-turnstile",
            "[data-sitekey]",
        ]
        for selector in selectors:
            if await page.locator(selector).count() > 0:
                return True
        return False

    async def _fill_generic_form(
        self, page: Page, identity: IdentityContext, source_url: str
    ) -> int:
        """
        Fills form fields using generic selectors and returns how many were filled.

        MVP implementation - matches common patterns like:
        - input[name*="name"]
        - input[name*="url"]
        - input[name*="email"]

        In production: use site-specific selectors stored in procedure.form_fields.
        """
        full_name = (
            identity.full_name or f"{identity.first_name} {identity.last_name}"
        ).strip()
        filled = 0

        # Try to fill name field
        name_selectors = [
            'input[name*="name" i]',
            'input[id*="name" i]',
            'input[placeholder*="name" i]',
        ]
        if full_name:
            for selector in name_selectors:
                if await page.locator(selector).count() > 0:
                    await page.locator(selector).first.fill(full_name)
                    log.debug("filled_name_field", selector=selector)
                    filled += 1
                    break

        # Try to fill URL/listing field
        url_selectors = [
            'input[name*="url" i]',
            'input[name*="listing" i]',
            'input[id*="url" i]',
            'textarea[name*="url" i]',
        ]
        for selector in url_selectors:
            if not source_url:
                break
            if await page.locator(selector).count() > 0:
                await page.locator(selector).first.fill(source_url)
                log.debug("filled_url_field", selector=selector)
                filled += 1
                break

        # Try to fill email field if present
        email_selectors = [
            'input[name*="email" i]',
            'input[type="email"]',
            'input[id*="email" i]',
        ]
        if identity.email:
            for selector in email_selectors:
                if await page.locator(selector).count() > 0:
                    await page.locator(selector).first.fill(identity.email)
                    log.debug("filled_email_field", selector=selector)
                    filled += 1
                    break

        return filled

    async def _submit_form(self, page: Page) -> bool:
        """Submits the form using common submit patterns. Returns False if none matched."""
        submit_selectors = [
            'button[type="submit"]',
            'input[type="submit"]',
            'button:has-text("Submit")',
            'button:has-text("Remove")',
            'button:has-text("Opt Out")',
        ]

        for selector in submit_selectors:
            if await page.locator(selector).count() > 0:
                await page.locator(selector).first.click()
                log.debug("clicked_submit", selector=selector)
                return True

        # Fallback: press Enter on first input
        if await page.locator("input").count() > 0:
            await page.locator("input").first.press("Enter")
            log.debug("fallback_submit", method="enter_key")
            return True

        return False

    def generate_evidence_hash(self, screenshot_bytes: bytes) -> str:
        """Generates SHA-256 hash of screenshot for tamper-evidence."""
        return hashlib.sha256(screenshot_bytes).hexdigest()

    async def submit_email_request(
        self, procedure: RemovalProcedure, identity: IdentityContext, source_url: str
    ) -> dict:
        """
        Submits removal request via email using SMTP.

        Sends a properly formatted opt-out email to the data broker's
        privacy contact address with all required customer information.
        """
        if not procedure.opt_out_email:
            return {
                "status": "failed",
                "confirmation_ref": None,
                "evidence_hash": None,
                "error": "No opt-out email address provided",
            }

        if not self.config.smtp_username or not self.config.smtp_password:
            log.error("smtp_not_configured")
            return {
                "status": "failed",
                "confirmation_ref": None,
                "evidence_hash": None,
                "error": "SMTP credentials not configured",
            }

        log.info("submit_email_request", to_domain=(procedure.opt_out_email or "").rpartition("@")[2])

        try:
            # Build email content
            subject = f"Data Removal Request for {identity.first_name} {identity.last_name}"
            body_html, body_text = self._build_email_body(identity, source_url, procedure)

            # Create message
            msg = MIMEMultipart("alternative")
            msg["Subject"] = subject
            msg["From"] = self.config.smtp_from_email or self.config.smtp_username
            msg["To"] = procedure.opt_out_email
            msg["Reply-To"] = identity.email or self.config.smtp_from_email
            msg["Message-ID"] = make_msgid(domain=(self.config.smtp_from_email or "vectorscrub.local").rpartition("@")[2] or None)

            # Attach text and HTML versions
            msg.attach(MIMEText(body_text, "plain"))
            msg.attach(MIMEText(body_html, "html"))

            # Send email via SMTP
            message_id = await self._send_smtp_email(msg, procedure.opt_out_email)

            # Generate evidence hash from email content
            evidence_content = f"{subject}\n{body_text}".encode("utf-8")
            evidence_hash = hashlib.sha256(evidence_content).hexdigest()

            log.info(
                "email_submitted",
                to_domain=procedure.opt_out_email.rpartition("@")[2],
                message_id=message_id,
                evidence_hash=evidence_hash,
            )

            return {
                "status": "submitted",
                "confirmation_ref": message_id or f"email:{procedure.opt_out_email}",
                "evidence_hash": evidence_hash,
                "error": None,
            }

        except Exception as exc:
            log.error("email_submission_failed", error_type=_why(exc))
            return {
                "status": "failed",
                "confirmation_ref": None,
                "evidence_hash": None,
                "error": f"Email send failed ({_why(exc)})",
            }

    def _build_email_body(
        self, identity: IdentityContext, source_url: str, procedure: RemovalProcedure
    ) -> tuple[str, str]:
        """
        Build HTML and plain text versions of the opt-out email.

        Returns: (html_body, text_body)
        """
        full_name = f"{identity.first_name} {identity.last_name}"

        # Plain text version
        text_body = f"""Dear Privacy Team,

I am writing to request the immediate removal of my personal information from your database and website in accordance with applicable privacy laws including CCPA, GDPR, and other data protection regulations.

Personal Information to Remove:
- Full Name: {full_name}
- Current Address: {identity.current_address or 'Not provided'}
- Email: {identity.email or 'Not provided'}
- URL/Listing: {source_url}

Previous Addresses (if applicable):
{chr(10).join(f"- {addr}" for addr in (identity.previous_addresses or [])) or '- None'}

Phone Numbers (if applicable):
{chr(10).join(f"- {phone}" for phone in (identity.phone_numbers or [])) or '- None'}

I request that you:
1. Remove all my personal information from your database
2. Confirm the removal in writing
3. Ensure my information is not re-added from third-party sources

Expected timeline: {procedure.estimated_days} days as per your published opt-out procedure.

Please send confirmation to: {identity.email or 'the email address this was sent from'}

Thank you for your prompt attention to this matter.

Sincerely,
{full_name}

---
This request was submitted via VectorScrub Privacy Protection Platform
"""

        # HTML version. Every value is escaped: names, addresses and the listing URL come
        # from the customer and from scraped pages, and must not be able to inject markup.
        e = html.escape
        h_name = e(full_name)
        h_address = e(identity.current_address or "Not provided")
        h_email = e(identity.email or "Not provided")
        h_email_reply = e(identity.email or "the email address this was sent from")
        h_url = e(source_url, quote=True)
        h_prev = "".join(f"<li>{e(a)}</li>" for a in (identity.previous_addresses or []))
        h_phones = "".join(f"<li>{e(ph)}</li>" for ph in (identity.phone_numbers or []))
        html_body = f"""<!DOCTYPE html>
<html>
<head>
    <meta charset="utf-8">
    <style>
        body {{ font-family: Arial, sans-serif; line-height: 1.6; color: #333; }}
        .header {{ background: #f4f4f4; padding: 20px; border-bottom: 3px solid #0066cc; }}
        .content {{ padding: 20px; }}
        .info-block {{ background: #f9f9f9; padding: 15px; margin: 15px 0; border-left: 4px solid #0066cc; }}
        .footer {{ margin-top: 30px; padding-top: 20px; border-top: 1px solid #ddd; font-size: 0.9em; color: #666; }}
        ul {{ margin: 10px 0; padding-left: 20px; }}
    </style>
</head>
<body>
    <div class="header">
        <h2>Data Removal Request</h2>
    </div>

    <div class="content">
        <p>Dear Privacy Team,</p>

        <p>I am writing to request the immediate removal of my personal information from your database and website in accordance with applicable privacy laws including <strong>CCPA, GDPR</strong>, and other data protection regulations.</p>

        <div class="info-block">
            <h3>Personal Information to Remove:</h3>
            <ul>
                <li><strong>Full Name:</strong> {h_name}</li>
                <li><strong>Current Address:</strong> {h_address}</li>
                <li><strong>Email:</strong> {h_email}</li>
                <li><strong>URL/Listing:</strong> <a href="{h_url}">{h_url}</a></li>
            </ul>

            {'<h4>Previous Addresses:</h4><ul>' + h_prev + '</ul>' if h_prev else ''}

            {'<h4>Phone Numbers:</h4><ul>' + h_phones + '</ul>' if h_phones else ''}
        </div>

        <p><strong>I request that you:</strong></p>
        <ol>
            <li>Remove all my personal information from your database</li>
            <li>Confirm the removal in writing</li>
            <li>Ensure my information is not re-added from third-party sources</li>
        </ol>

        <p><strong>Expected timeline:</strong> {procedure.estimated_days} days as per your published opt-out procedure.</p>

        <p>Please send confirmation to: <strong>{h_email_reply}</strong></p>

        <p>Thank you for your prompt attention to this matter.</p>

        <p>Sincerely,<br>
        <strong>{h_name}</strong></p>

        <div class="footer">
            <p><em>This request was submitted via VectorScrub Privacy Protection Platform</em></p>
        </div>
    </div>
</body>
</html>"""

        return html_body, text_body

    async def _send_smtp_email(self, msg: MIMEMultipart, to_email: str) -> Optional[str]:
        """
        Send email via SMTP and return message ID.

        Runs in executor to avoid blocking the event loop.
        """
        def _send_sync():
            host, port = self.config.smtp_host, self.config.smtp_port
            if not self.config.smtp_use_tls and host not in ("localhost", "127.0.0.1", "::1"):
                # Never send the SMTP password, or a customer's identity, in clear text.
                raise ssl.SSLError("SMTP without TLS is only allowed to localhost")

            # Verify the server's certificate and hostname. A bare starttls() accepts any
            # certificate, so anyone on the path could read the credentials and the message.
            tls = ssl.create_default_context()
            if port == 465:
                server = smtplib.SMTP_SSL(host, port, timeout=30, context=tls)
            else:
                server = smtplib.SMTP(host, port, timeout=30)
            try:
                if port != 465 and self.config.smtp_use_tls:
                    server.starttls(context=tls)
                server.login(self.config.smtp_username, self.config.smtp_password)
                server.send_message(msg)
            except Exception as exc:
                log.error("smtp_send_error", error_type=_why(exc))
                raise
            finally:
                try:
                    server.quit()
                except Exception:
                    pass  # the message outcome is already decided; a failed goodbye changes nothing
            return msg["Message-ID"]

        # Run SMTP operation in thread pool to avoid blocking
        loop = asyncio.get_event_loop()
        message_id = await loop.run_in_executor(None, _send_sync)
        return message_id

    async def submit_drop_api(
        self, procedure: RemovalProcedure, identity: IdentityContext
    ) -> dict:
        """
        Submits removal request to California DROP (Delete Request and Opt-Out Platform) API.

        The DROP platform allows California residents to submit deletion requests
        to 600+ registered data brokers simultaneously through a single API call.

        Reference: https://privacy.ca.gov/data-deletion/
        """
        if not self.config.drop_api_key or not self.config.drop_api_secret:
            log.error("drop_api_not_configured")
            return {
                "status": "failed",
                "confirmation_ref": None,
                "evidence_hash": None,
                "error": "DROP API credentials not configured",
            }

        log.info("submit_drop_api", sandbox=self.config.drop_enable_sandbox)

        try:
            # Build DROP API request payload
            payload = self._build_drop_payload(identity)

            # Authenticate and submit
            api_url = self.config.drop_api_url
            if self.config.drop_enable_sandbox:
                api_url = api_url.replace("/v1", "/sandbox/v1")

            async with httpx.AsyncClient(timeout=60) as client:
                # Step 1: Authenticate
                auth_response = await client.post(
                    f"{api_url}/auth/token",
                    json={
                        "api_key": self.config.drop_api_key,
                        "api_secret": self.config.drop_api_secret,
                    },
                )
                auth_response.raise_for_status()
                access_token = auth_response.json().get("access_token")

                if not access_token:
                    raise ValueError("Failed to obtain DROP API access token")

                # Step 2: Submit deletion request
                headers = {
                    "Authorization": f"Bearer {access_token}",
                    "Content-Type": "application/json",
                }

                submission_response = await client.post(
                    f"{api_url}/deletion-requests",
                    json=payload,
                    headers=headers,
                )
                submission_response.raise_for_status()
                result = submission_response.json()

                request_id = result.get("request_id")
                status = result.get("status", "submitted")
                broker_count = result.get("brokers_notified", 0)

                log.info(
                    "drop_api_submitted",
                    request_id=request_id,
                    status=status,
                    brokers_notified=broker_count,
                )

                # Generate evidence hash from request payload
                evidence_content = f"{request_id}\n{payload}".encode("utf-8")
                evidence_hash = hashlib.sha256(evidence_content).hexdigest()

                return {
                    "status": "submitted",
                    "confirmation_ref": f"DROP-{request_id}",
                    "evidence_hash": evidence_hash,
                    "error": None,
                }

        except httpx.HTTPStatusError as exc:
            # Status only: the response body can echo back the identity that was submitted.
            log.error("drop_api_http_error", status_code=exc.response.status_code)
            return {
                "status": "failed",
                "confirmation_ref": None,
                "evidence_hash": None,
                "error": f"DROP API error: HTTP {exc.response.status_code}",
            }
        except Exception as exc:
            log.error("drop_api_submission_failed", error_type=_why(exc))
            return {
                "status": "failed",
                "confirmation_ref": None,
                "evidence_hash": None,
                "error": f"DROP API submission failed ({_why(exc)})",
            }

    def _build_drop_payload(self, identity: IdentityContext) -> dict:
        """
        Build DROP API deletion request payload.

        Format based on typical government API patterns.
        Actual format should match California DROP API specification.
        """
        payload = {
            "request_type": "deletion",
            "requester": {
                "first_name": identity.first_name,
                "last_name": identity.last_name,
                "email": identity.email,
                "current_address": {
                    "full_address": identity.current_address,
                },
            },
            "identity_verification": {
                # In production, this would include identity verification documents
                # For now, we use the provided identity fields
                "method": "self_attestation",
                "verified_at": None,  # Platform handles verification
            },
            "scope": {
                "include_all_brokers": True,  # Submit to all 600+ registered brokers
                "specific_categories": [
                    "people_search",
                    "data_aggregators",
                    "marketing_databases",
                ],
            },
            "additional_info": {
                "previous_addresses": identity.previous_addresses or [],
                "phone_numbers": identity.phone_numbers or [],
                "email_addresses": identity.email_addresses or [],
                "date_of_birth": identity.date_of_birth,
            },
        }

        return payload
