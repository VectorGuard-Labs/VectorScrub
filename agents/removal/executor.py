"""
Removal executor handles automated submission of data removal requests.
Supports web forms (Playwright), email requests, and California DROP API.
"""

import asyncio
import hashlib
import re
from typing import Optional
from playwright.async_api import async_playwright, Page, TimeoutError as PlaywrightTimeout
import structlog

from shared.models import RemovalProcedure, IdentityContext
from shared.backend_client import BackendClient

log = structlog.get_logger()


class RemovalExecutor:
    """Executes removal requests via multiple channels (web, email, API)."""

    def __init__(self):
        self.backend = BackendClient()

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
        Submits removal request via email.

        MVP stub - Phase 3 will implement full SMTP integration.
        """
        log.warning("email_submission_not_implemented", opt_out_email=procedure.opt_out_email)
        return {
            "status": "failed",
            "confirmation_ref": None,
            "evidence_hash": None,
            "error": "Email submission not yet implemented (Phase 3)",
        }

    async def submit_drop_api(
        self, procedure: RemovalProcedure, identity: IdentityContext
    ) -> dict:
        """
        Submits removal request to California DROP API.

        MVP stub - Phase 3 will implement full DROP API integration.
        """
        log.warning("drop_api_not_implemented")
        return {
            "status": "failed",
            "confirmation_ref": None,
            "evidence_hash": None,
            "error": "DROP API submission not yet implemented (Phase 3)",
        }
