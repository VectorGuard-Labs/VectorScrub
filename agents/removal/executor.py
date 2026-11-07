"""
Removal executor handles automated submission of data removal requests.
Supports web forms (Playwright), email requests, and California DROP API.
"""

import asyncio
import hashlib
import re
import smtplib
from email.mime.multipart import MIMEMultipart
from email.mime.text import MIMEText
from typing import Optional
from playwright.async_api import async_playwright, Page, TimeoutError as PlaywrightTimeout
import httpx
import structlog

from shared.models import RemovalProcedure, IdentityContext
from shared.backend_client import BackendClient
from shared.config import get_config

log = structlog.get_logger()


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
            source_url=source_url,
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

            log.info("submission_complete", result=result)
            return result

        except Exception as exc:
            log.error("submission_failed", error=str(exc))
            return {
                "status": "failed",
                "confirmation_ref": None,
                "evidence_hash": None,
                "error": str(exc),
            }

    async def submit_web_form(
        self, procedure: RemovalProcedure, identity: IdentityContext, source_url: str
    ) -> dict:
        """
        Submits removal request via web form using Playwright.

        Generic implementation for MVP - in production, use site-specific handlers.
        """
        log.info("submit_web_form", opt_out_url=procedure.opt_out_url)

        async with async_playwright() as p:
            browser = await p.chromium.launch(headless=True)
            try:
                page = await browser.new_page()
                await page.set_viewport_size({"width": 1280, "height": 1024})

                # Navigate to opt-out page
                await page.goto(procedure.opt_out_url, timeout=30000)
                await page.wait_for_load_state("networkidle", timeout=30000)

                # Fill form fields using generic selectors
                # In production: use procedure.form_fields for site-specific selectors
                await self._fill_generic_form(page, identity, source_url)

                # Submit form
                await self._submit_form(page)

                # Wait for confirmation page
                await page.wait_for_load_state("networkidle", timeout=20000)
                await asyncio.sleep(2)  # Allow dynamic content to load

                # Capture screenshot as evidence
                screenshot_bytes = await page.screenshot(full_page=True)
                evidence_hash = self.generate_evidence_hash(screenshot_bytes)

                # Extract confirmation reference
                confirmation_ref = await self.extract_confirmation_reference(page)

                log.info(
                    "web_form_submitted",
                    confirmation_ref=confirmation_ref,
                    evidence_hash=evidence_hash,
                )

                return {
                    "status": "submitted",
                    "confirmation_ref": confirmation_ref,
                    "evidence_hash": evidence_hash,
                    "error": None,
                }

            except PlaywrightTimeout as exc:
                log.warning("web_form_timeout", error=str(exc))
                return {
                    "status": "failed",
                    "confirmation_ref": None,
                    "evidence_hash": None,
                    "error": f"Timeout: {exc}",
                }
            finally:
                await browser.close()

    async def _fill_generic_form(
        self, page: Page, identity: IdentityContext, source_url: str
    ) -> None:
        """
        Fills form fields using generic selectors.

        MVP implementation - matches common patterns like:
        - input[name*="name"]
        - input[name*="url"]
        - input[name*="email"]

        In production: use site-specific selectors stored in procedure.form_fields.
        """
        full_name = f"{identity.first_name} {identity.last_name}"

        # Try to fill name field
        name_selectors = [
            'input[name*="name" i]',
            'input[id*="name" i]',
            'input[placeholder*="name" i]',
        ]
        for selector in name_selectors:
            if await page.locator(selector).count() > 0:
                await page.locator(selector).first.fill(full_name)
                log.debug("filled_name_field", selector=selector)
                break

        # Try to fill URL/listing field
        url_selectors = [
            'input[name*="url" i]',
            'input[name*="listing" i]',
            'input[id*="url" i]',
            'textarea[name*="url" i]',
        ]
        for selector in url_selectors:
            if await page.locator(selector).count() > 0:
                await page.locator(selector).first.fill(source_url)
                log.debug("filled_url_field", selector=selector)
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
                    break

    async def _submit_form(self, page: Page) -> None:
        """Submits the form using common submit patterns."""
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
                return

        # Fallback: press Enter on first input
        await page.locator("input").first.press("Enter")
        log.debug("fallback_submit", method="enter_key")

    async def extract_confirmation_reference(self, page: Page) -> Optional[str]:
        """
        Extracts confirmation reference from page using regex patterns.

        Common patterns:
        - Reference: #12345
        - Confirmation 67890
        - Ticket: ABC-123
        - Request ID: XYZ789
        """
        content = await page.content()

        patterns = [
            r"Reference:?\s*#?([A-Z0-9-]+)",
            r"Confirmation:?\s*#?([A-Z0-9-]+)",
            r"Ticket:?\s*#?([A-Z0-9-]+)",
            r"Request\s*ID:?\s*#?([A-Z0-9-]+)",
            r"Case\s*#?([A-Z0-9-]+)",
        ]

        for pattern in patterns:
            match = re.search(pattern, content, re.IGNORECASE)
            if match:
                ref = match.group(1)
                log.debug("extracted_confirmation", ref=ref, pattern=pattern)
                return ref

        # No match - return page title or first heading as fallback
        title = await page.title()
        if "success" in title.lower() or "confirm" in title.lower():
            return f"page_title:{title[:50]}"

        return None

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

        log.info("submit_email_request", to=procedure.opt_out_email)

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
                to=procedure.opt_out_email,
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
            log.error("email_submission_failed", error=str(exc))
            return {
                "status": "failed",
                "confirmation_ref": None,
                "evidence_hash": None,
                "error": f"Email send failed: {exc}",
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

        # HTML version
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
                <li><strong>Full Name:</strong> {full_name}</li>
                <li><strong>Current Address:</strong> {identity.current_address or 'Not provided'}</li>
                <li><strong>Email:</strong> {identity.email or 'Not provided'}</li>
                <li><strong>URL/Listing:</strong> <a href="{source_url}">{source_url}</a></li>
            </ul>

            {'<h4>Previous Addresses:</h4><ul>' + ''.join(f'<li>{addr}</li>' for addr in (identity.previous_addresses or [])) + '</ul>' if identity.previous_addresses else ''}

            {'<h4>Phone Numbers:</h4><ul>' + ''.join(f'<li>{phone}</li>' for phone in (identity.phone_numbers or [])) + '</ul>' if identity.phone_numbers else ''}
        </div>

        <p><strong>I request that you:</strong></p>
        <ol>
            <li>Remove all my personal information from your database</li>
            <li>Confirm the removal in writing</li>
            <li>Ensure my information is not re-added from third-party sources</li>
        </ol>

        <p><strong>Expected timeline:</strong> {procedure.estimated_days} days as per your published opt-out procedure.</p>

        <p>Please send confirmation to: <strong>{identity.email or 'the email address this was sent from'}</strong></p>

        <p>Thank you for your prompt attention to this matter.</p>

        <p>Sincerely,<br>
        <strong>{full_name}</strong></p>

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
            try:
                if self.config.smtp_use_tls:
                    server = smtplib.SMTP(self.config.smtp_host, self.config.smtp_port, timeout=30)
                    server.starttls()
                else:
                    server = smtplib.SMTP(self.config.smtp_host, self.config.smtp_port, timeout=30)

                server.login(self.config.smtp_username, self.config.smtp_password)
                server.send_message(msg)
                message_id = msg.get("Message-ID", f"vectorscrub-{hash(msg.as_string())}")
                server.quit()
                return message_id
            except Exception as exc:
                log.error("smtp_send_error", error=str(exc))
                raise

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
            log.error(
                "drop_api_http_error",
                status_code=exc.response.status_code,
                response=exc.response.text,
            )
            return {
                "status": "failed",
                "confirmation_ref": None,
                "evidence_hash": None,
                "error": f"DROP API error: {exc.response.status_code} - {exc.response.text[:200]}",
            }
        except Exception as exc:
            log.error("drop_api_submission_failed", error=str(exc))
            return {
                "status": "failed",
                "confirmation_ref": None,
                "evidence_hash": None,
                "error": f"DROP API submission failed: {exc}",
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
