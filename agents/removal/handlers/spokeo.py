"""
Spokeo.com removal handler.

Spokeo requires the exact profile URL (not a search page) for opt-out.
The opt-out form asks for the profile URL and an email address.
"""
import asyncio

from playwright.async_api import Page

from shared.models import IdentityContext
from .base import BrokerHandler


class SpokeoHandler(BrokerHandler):
    """
    Handler for Spokeo.com opt-out requests.

    Spokeo's opt-out process:
    1. Navigate to https://www.spokeo.com/optout
    2. Enter the exact profile URL (e.g., spokeo.com/First-Last/City/ST/p12345678)
    3. Enter email address
    4. Click submit
    5. Check email for verification link (customer must complete)
    """

    async def fill_form(
        self, page: Page, identity: IdentityContext, source_url: str
    ) -> bool:
        """Fill Spokeo opt-out form with profile URL and email."""
        try:
            # Wait for form to load
            await page.wait_for_selector('input[name="url"], input[id="url"]', timeout=10000)

            # Fill profile URL field
            url_input = page.locator('input[name="url"], input[id="url"]').first
            await url_input.fill(source_url)
            self.log.debug("filled_spokeo_url", url_length=len(source_url))

            # Fill email field
            if identity.email:
                email_input = page.locator('input[name="email"], input[type="email"]').first
                await email_input.fill(identity.email)
                self.log.debug("filled_spokeo_email")

            await asyncio.sleep(1)  # Allow form validation
            return True

        except Exception as exc:
            self.log.warning("spokeo_fill_failed", error_type=type(exc).__name__)
            return False

    async def submit(self, page: Page) -> bool:
        """Submit Spokeo opt-out form."""
        try:
            # Find and click submit button
            submit_selectors = [
                'button[type="submit"]',
                'input[type="submit"]',
                'button:has-text("Remove")',
                'button:has-text("Submit")',
            ]

            for selector in submit_selectors:
                button = page.locator(selector).first
                if await button.count() > 0:
                    await button.click()
                    self.log.debug("clicked_spokeo_submit", selector=selector)

                    # Wait for navigation or result message
                    try:
                        await page.wait_for_load_state("networkidle", timeout=15000)
                    except Exception:
                        pass  # Page may not navigate, just show result

                    return True

            self.log.warning("spokeo_no_submit_button")
            return False

        except Exception as exc:
            self.log.warning("spokeo_submit_failed", error_type=type(exc).__name__)
            return False

    async def extract_confirmation(self, page: Page, page_text: str) -> str | None:
        """
        Spokeo doesn't provide an immediate confirmation reference.
        They send a verification email that the customer must click.
        """
        # Check for email verification message
        if "check your email" in page_text.lower() or "verification email" in page_text.lower():
            return "EMAIL_VERIFICATION_REQUIRED"
        return None
