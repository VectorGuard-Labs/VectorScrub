"""
Generic fallback handler for brokers without site-specific implementations.

Uses common patterns to fill forms - approximately 60% success rate.
"""
from playwright.async_api import Page
from .base import BrokerHandler
from shared.models import IdentityContext


class GenericHandler(BrokerHandler):
    """
    Generic handler that attempts to fill forms using common patterns.

    This is the fallback when no site-specific handler exists. It looks for
    common field names like 'email', 'name', 'url' etc. and tries generic
    submit button patterns.

    Success rate: ~60% (varies by broker)
    """

    async def fill_form(
        self, page: Page, identity: IdentityContext, source_url: str
    ) -> bool:
        """Fill form using generic selectors."""
        full_name = (
            identity.full_name or f"{identity.first_name} {identity.last_name}"
        ).strip()
        filled = 0

        # Name field
        name_selectors = [
            'input[name*="name" i]',
            'input[id*="name" i]',
            'input[placeholder*="name" i]',
        ]
        if full_name:
            for selector in name_selectors:
                try:
                    if await page.locator(selector).count() > 0:
                        await page.locator(selector).first.fill(full_name)
                        self.log.debug("filled_name_field", selector=selector)
                        filled += 1
                        break
                except Exception:
                    continue

        # URL/listing field
        url_selectors = [
            'input[name*="url" i]',
            'input[name*="listing" i]',
            'input[id*="url" i]',
            'textarea[name*="url" i]',
        ]
        if source_url:
            for selector in url_selectors:
                try:
                    if await page.locator(selector).count() > 0:
                        await page.locator(selector).first.fill(source_url)
                        self.log.debug("filled_url_field", selector=selector)
                        filled += 1
                        break
                except Exception:
                    continue

        # Email field
        email_selectors = [
            'input[name*="email" i]',
            'input[type="email"]',
            'input[id*="email" i]',
        ]
        if identity.email:
            for selector in email_selectors:
                try:
                    if await page.locator(selector).count() > 0:
                        await page.locator(selector).first.fill(identity.email)
                        self.log.debug("filled_email_field", selector=selector)
                        filled += 1
                        break
                except Exception:
                    continue

        return filled > 0

    async def submit(self, page: Page) -> bool:
        """Submit using generic button patterns."""
        submit_selectors = [
            'button[type="submit"]',
            'input[type="submit"]',
            'button:has-text("Submit")',
            'button:has-text("Remove")',
            'button:has-text("Opt Out")',
            'button:has-text("Continue")',
        ]

        for selector in submit_selectors:
            try:
                if await page.locator(selector).count() > 0:
                    await page.locator(selector).first.click()
                    self.log.debug("clicked_submit", selector=selector)
                    return True
            except Exception:
                continue

        # Fallback: press Enter on first input
        try:
            if await page.locator("input").count() > 0:
                await page.locator("input").first.press("Enter")
                self.log.debug("fallback_submit", method="enter_key")
                return True
        except Exception:
            pass

        return False
