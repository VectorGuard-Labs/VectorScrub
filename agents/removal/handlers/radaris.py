"""
Radaris.com removal handler.

Radaris requires the listing URL and email verification for removal.
"""
import asyncio

from playwright.async_api import Page

from shared.models import IdentityContext
from .base import BrokerHandler


class RadarisHandler(BrokerHandler):
    """
    Handler for Radaris.com removal requests.

    Radaris opt-out process:
    1. Navigate to https://radaris.com/page/how-to-remove
    2. Enter listing URL
    3. Enter email address
    4. Submit and verify via email link
    """

    async def fill_form(
        self, page: Page, identity: IdentityContext, source_url: str
    ) -> bool:
        """Fill Radaris removal form."""
        try:
            # Wait for form to load
            await page.wait_for_selector(
                'input[name="url"], input[type="url"], textarea',
                timeout=10000
            )

            # Fill listing URL
            url_selectors = [
                'input[name="url"]',
                'input[type="url"]',
                'textarea[name="url"]',
                'input[placeholder*="URL" i]',
                'input[placeholder*="link" i]',
            ]

            filled_url = False
            for selector in url_selectors:
                field = page.locator(selector).first
                if await field.count() > 0:
                    await field.fill(source_url)
                    self.log.debug("filled_radaris_url", selector=selector)
                    filled_url = True
                    break

            if not filled_url:
                self.log.warning("radaris_url_field_not_found")
                return False

            # Fill email
            if identity.email:
                email_input = page.locator(
                    'input[type="email"], input[name="email"]'
                ).first
                if await email_input.count() > 0:
                    await email_input.fill(identity.email)
                    self.log.debug("filled_radaris_email")
                else:
                    self.log.warning("radaris_email_field_not_found")
                    return False

            # Fill name if requested
            name_input = page.locator(
                'input[name="name"], input[placeholder*="name" i]'
            ).first
            if await name_input.count() > 0 and identity.full_name:
                await name_input.fill(identity.full_name)
                self.log.debug("filled_radaris_name")

            await asyncio.sleep(1)  # Allow validation
            return True

        except Exception as exc:
            self.log.warning("radaris_fill_failed", error_type=type(exc).__name__)
            return False

    async def submit(self, page: Page) -> bool:
        """Submit Radaris removal request."""
        try:
            # Find and click submit button
            submit_selectors = [
                'button[type="submit"]',
                'input[type="submit"]',
                'button:has-text("Submit")',
                'button:has-text("Remove")',
                'button:has-text("Request Removal")',
            ]

            for selector in submit_selectors:
                button = page.locator(selector).first
                if await button.count() > 0:
                    await button.click()
                    self.log.debug("clicked_radaris_submit", selector=selector)

                    # Wait for result
                    try:
                        await page.wait_for_load_state("networkidle", timeout=15000)
                    except Exception:
                        await asyncio.sleep(2)

                    return True

            self.log.warning("radaris_no_submit_button")
            return False

        except Exception as exc:
            self.log.warning("radaris_submit_failed", error_type=type(exc).__name__)
            return False

    async def extract_confirmation(self, page: Page, page_text: str) -> str | None:
        """Extract confirmation from Radaris result page."""
        text_lower = page_text.lower()
        # Radaris sends verification email
        if "verification" in text_lower or "check your email" in text_lower:
            return "EMAIL_VERIFICATION_REQUIRED"
        # Radaris typically shows "within 24-72 hours" message
        if "24" in text_lower and "72" in text_lower and "hours" in text_lower:
            return "REMOVAL_SCHEDULED_24_72_HOURS"
        return None
