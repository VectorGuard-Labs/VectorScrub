"""
TruePeopleSearch.com removal handler.

TruePeopleSearch has a straightforward removal process with email verification.
"""
import asyncio

from playwright.async_api import Page

from shared.models import IdentityContext
from .base import BrokerHandler


class TruePeopleSearchHandler(BrokerHandler):
    """
    Handler for TruePeopleSearch.com removal requests.

    TruePeopleSearch opt-out process:
    1. Navigate to https://www.truepeoplesearch.com/removal
    2. Search for your record by name
    3. Click "Remove" button next to your record
    4. Enter email for verification
    5. Complete removal via verification email link
    """

    async def fill_form(
        self, page: Page, identity: IdentityContext, source_url: str
    ) -> bool:
        """
        Fill TruePeopleSearch removal form.

        Note: TruePeopleSearch requires searching for the record first,
        then selecting it from results. This is a multi-step process.
        """
        try:
            # Check if we're on the search page or record page
            page_url = page.url.lower()

            # If on search page, need to search first
            if "removal" in page_url and "results" not in page_url:
                # Fill search form
                name_parts = identity.full_name.split()
                if len(name_parts) >= 2:
                    first_name = name_parts[0]
                    last_name = name_parts[-1]

                    # Fill first name
                    first_input = page.locator(
                        'input[name="firstName"], input[name="fn"], input[placeholder*="First" i]'
                    ).first
                    if await first_input.count() > 0:
                        await first_input.fill(first_name)
                        self.log.debug("filled_truepeoplesearch_first_name")

                    # Fill last name
                    last_input = page.locator(
                        'input[name="lastName"], input[name="ln"], input[placeholder*="Last" i]'
                    ).first
                    if await last_input.count() > 0:
                        await last_input.fill(last_name)
                        self.log.debug("filled_truepeoplesearch_last_name")

                    # Click search
                    search_button = page.locator(
                        'button[type="submit"], button:has-text("Search")'
                    ).first
                    if await search_button.count() > 0:
                        await search_button.click()
                        await page.wait_for_load_state("networkidle", timeout=10000)
                        self.log.debug("submitted_truepeoplesearch_search")

            # Now we should be on results page - find the matching record
            # Look for "Remove" buttons
            remove_buttons = page.locator('a:has-text("Remove"), button:has-text("Remove")')
            button_count = await remove_buttons.count()

            if button_count > 0:
                # Click first Remove button (ideally we'd match by address, but this is MVP)
                await remove_buttons.first.click()
                await page.wait_for_load_state("networkidle", timeout=10000)
                self.log.debug("clicked_truepeoplesearch_remove")

            # Fill email verification field
            if identity.email:
                email_input = page.locator(
                    'input[type="email"], input[name="email"]'
                ).first
                if await email_input.count() > 0:
                    await email_input.fill(identity.email)
                    self.log.debug("filled_truepeoplesearch_email")

            await asyncio.sleep(1)
            return True

        except Exception as exc:
            self.log.warning("truepeoplesearch_fill_failed", error_type=type(exc).__name__)
            return False

    async def submit(self, page: Page) -> bool:
        """Submit TruePeopleSearch removal request."""
        try:
            # Find and click submit button
            submit_selectors = [
                'button[type="submit"]',
                'input[type="submit"]',
                'button:has-text("Submit")',
                'button:has-text("Remove Record")',
                'button:has-text("Send Verification")',
            ]

            for selector in submit_selectors:
                button = page.locator(selector).first
                if await button.count() > 0:
                    await button.click()
                    self.log.debug("clicked_truepeoplesearch_submit", selector=selector)

                    # Wait for result
                    try:
                        await page.wait_for_load_state("networkidle", timeout=15000)
                    except Exception:
                        await asyncio.sleep(2)

                    return True

            self.log.warning("truepeoplesearch_no_submit_button")
            return False

        except Exception as exc:
            self.log.warning("truepeoplesearch_submit_failed", error_type=type(exc).__name__)
            return False

    async def extract_confirmation(self, page: Page, page_text: str) -> str | None:
        """Extract confirmation from TruePeopleSearch result page."""
        text_lower = page_text.lower()
        if "verification email" in text_lower or "check your email" in text_lower:
            return "EMAIL_VERIFICATION_SENT"
        return None
