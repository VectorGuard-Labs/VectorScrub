"""
Nuwber.com opt-out handler.

Note: Often has CAPTCHA challenges (~70% success rate due to this).
"""
from playwright.async_api import Page
from .base import BrokerHandler
from shared.models import IdentityContext


class NuwberHandler(BrokerHandler):
    """
    Nuwber.com removal handler.

    Flow:
    1. Search for record
    2. Request removal
    3. Email/phone verification
    4. CAPTCHA completion (blocker)

    Success rate: ~70% (CAPTCHA challenges frequent)
    """

    async def fill_form(
        self, page: Page, identity: IdentityContext, source_url: str
    ) -> bool:
        """Fill Nuwber removal form."""
        filled = 0

        # Email or phone
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

        # Full name
        full_name = (
            identity.full_name or f"{identity.first_name} {identity.last_name}"
        ).strip()
        if full_name:
            name_selectors = [
                'input[name="name"]',
                'input[name="full_name"]',
                'input[id="name"]',
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

        return filled > 0

    async def submit(self, page: Page) -> bool:
        """Submit Nuwber removal (likely to hit CAPTCHA)."""
        submit_selectors = [
            'button:has-text("Remove")',
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

    async def has_captcha(self, page: Page) -> bool:
        """
        Nuwber frequently uses CAPTCHA.

        Override to add Nuwber-specific CAPTCHA detection.
        """
        # Check generic CAPTCHA first
        if await super().has_captcha(page):
            return True

        # Check Nuwber-specific patterns
        nuwber_captcha_selectors = [
            'div[class*="captcha"]',
            'div[id*="captcha"]',
            '.verification-challenge',
        ]

        for selector in nuwber_captcha_selectors:
            try:
                if await page.locator(selector).count() > 0:
                    self.log.debug("nuwber_captcha_detected", selector=selector)
                    return True
            except Exception:
                continue

        return False
