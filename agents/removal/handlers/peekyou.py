"""
PeekYou.com opt-out handler.

Simple form with email verification.
"""
from playwright.async_api import Page
from .base import BrokerHandler
from shared.models import IdentityContext


class PeekYouHandler(BrokerHandler):
    """
    PeekYou.com opt-out handler.

    Simple flow:
    1. Enter full name
    2. Enter email
    3. Submit
    4. Verify via email link

    Success rate: ~90%
    """

    async def fill_form(
        self, page: Page, identity: IdentityContext, source_url: str
    ) -> bool:
        """Fill PeekYou opt-out form."""
        filled = 0

        # Full name
        full_name = (
            identity.full_name or f"{identity.first_name} {identity.last_name}"
        ).strip()
        if full_name:
            name_selectors = [
                'input[name="full_name"]',
                'input[name="name"]',
                'input[id="full_name"]',
                'input[placeholder*="name" i]',
            ]
            for selector in name_selectors:
                try:
                    if await page.locator(selector).count() > 0:
                        await page.locator(selector).first.fill(full_name)
                        self.log.debug("filled_name", selector=selector)
                        filled += 1
                        break
                except Exception:
                    continue

        # Email
        if identity.email:
            email_selectors = [
                'input[name="email"]',
                'input[type="email"]',
                'input[id="email"]',
            ]
            for selector in email_selectors:
                try:
                    if await page.locator(selector).count() > 0:
                        await page.locator(selector).first.fill(identity.email)
                        self.log.debug("filled_email", selector=selector)
                        filled += 1
                        break
                except Exception:
                    continue

        return filled > 0

    async def submit(self, page: Page) -> bool:
        """Submit PeekYou opt-out."""
        submit_selectors = [
            'button:has-text("Submit")',
            'button:has-text("Opt Out")',
            'button[type="submit"]',
            'input[type="submit"]',
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
