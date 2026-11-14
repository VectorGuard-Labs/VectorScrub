"""
WhitePages.com removal handler.

WhitePages has a suppression request form that requires the listing URL.
"""
import asyncio

from playwright.async_api import Page

from shared.models import IdentityContext
from .base import BrokerHandler


class WhitePagesHandler(BrokerHandler):
    """
    Handler for WhitePages.com suppression requests.

    WhitePages opt-out process:
    1. Navigate to https://www.whitepages.com/suppression-requests
    2. Enter the listing URL
    3. Optionally provide contact info
    4. Submit request
    """

    async def fill_form(
        self, page: Page, identity: IdentityContext, source_url: str
    ) -> bool:
        """Fill WhitePages suppression form."""
        try:
            # Wait for form to load
            await page.wait_for_selector(
                'input[name="url"], input[name="listing_url"], textarea',
                timeout=10000
            )

            # Fill listing URL - try multiple possible field names
            url_selectors = [
                'input[name="url"]',
                'input[name="listing_url"]',
                'textarea[name="url"]',
                'input[placeholder*="URL" i]',
                'input[placeholder*="listing" i]',
            ]

            filled = False
            for selector in url_selectors:
                field = page.locator(selector).first
                if await field.count() > 0:
                    await field.fill(source_url)
                    self.log.debug("filled_whitepages_url", selector=selector)
                    filled = True
                    break

            if not filled:
                self.log.warning("whitepages_url_field_not_found")
                return False

            # Fill optional fields if present
            if identity.email:
                email_field = page.locator('input[type="email"], input[name="email"]').first
                if await email_field.count() > 0:
                    await email_field.fill(identity.email)
                    self.log.debug("filled_whitepages_email")

            if identity.full_name:
                name_field = page.locator('input[name="name"], input[placeholder*="name" i]').first
                if await name_field.count() > 0:
                    await name_field.fill(identity.full_name)
                    self.log.debug("filled_whitepages_name")

            await asyncio.sleep(1)  # Allow form validation
            return True

        except Exception as exc:
            self.log.warning("whitepages_fill_failed", error_type=type(exc).__name__)
            return False

    async def submit(self, page: Page) -> bool:
        """Submit WhitePages suppression form."""
        try:
            # Find and click submit button
            submit_selectors = [
                'button[type="submit"]',
                'input[type="submit"]',
                'button:has-text("Submit")',
                'button:has-text("Request Suppression")',
                'button:has-text("Remove")',
            ]

            for selector in submit_selectors:
                button = page.locator(selector).first
                if await button.count() > 0:
                    await button.click()
                    self.log.debug("clicked_whitepages_submit", selector=selector)

                    # Wait for result
                    try:
                        await page.wait_for_load_state("networkidle", timeout=15000)
                    except Exception:
                        await asyncio.sleep(2)  # Wait for dynamic content

                    return True

            self.log.warning("whitepages_no_submit_button")
            return False

        except Exception as exc:
            self.log.warning("whitepages_submit_failed", error_type=type(exc).__name__)
            return False
