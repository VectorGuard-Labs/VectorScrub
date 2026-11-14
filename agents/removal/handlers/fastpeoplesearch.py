"""
FastPeopleSearch.com opt-out handler.

Very similar to TruePeopleSearch (same owner).
"""
from playwright.async_api import Page
from .base import BrokerHandler
from shared.models import IdentityContext


class FastPeopleSearchHandler(BrokerHandler):
    """
    FastPeopleSearch.com removal handler.

    Simple flow:
    1. Enter email address
    2. Enter or select record URL
    3. Submit and wait for verification email

    Success rate: ~95%
    """

    async def fill_form(
        self, page: Page, identity: IdentityContext, source_url: str
    ) -> bool:
        """Fill FastPeopleSearch removal form."""
        filled = 0

        # Email
        if identity.email:
            email_selectors = ['input[name="email"]', 'input[type="email"]']
            for selector in email_selectors:
                try:
                    if await page.locator(selector).count() > 0:
                        await page.locator(selector).first.fill(identity.email)
                        self.log.debug("filled_email", selector=selector)
                        filled += 1
                        break
                except Exception:
                    continue

        # Record URL
        if source_url:
            url_selectors = ['input[name="url"]', 'input[name="record_url"]']
            for selector in url_selectors:
                try:
                    if await page.locator(selector).count() > 0:
                        await page.locator(selector).first.fill(source_url)
                        self.log.debug("filled_url", selector=selector)
                        filled += 1
                        break
                except Exception:
                    continue

        return filled > 0

    async def submit(self, page: Page) -> bool:
        """Submit FastPeopleSearch removal."""
        submit_selectors = [
            'button:has-text("Remove My Record")',
            'button:has-text("Submit")',
            'button[type="submit"]',
        ]

        for selector in submit_selectors:
            try:
                if await page.locator(selector).count() > 0:
                    await page.locator(selector).first.click()
                    self.log.debug("clicked_submit", selector=selector)
                    return True
            except Exception:
                continue

        return False
