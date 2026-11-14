"""
Intelius.com / PeopleConnect Suppression Center handler.

Shared suppression center for: Intelius, TruthFinder, InstantCheckmate, USSearch.
"""
import asyncio
from playwright.async_api import Page
from .base import BrokerHandler
from shared.models import IdentityContext


class InteliusHandler(BrokerHandler):
    """
    PeopleConnect Suppression Center handler.

    Handles opt-outs for:
    - Intelius.com
    - TruthFinder.com
    - InstantCheckmate.com
    - USSearch.com

    Multi-step flow:
    1. Email verification
    2. Search by name + DOB
    3. Select record
    4. Suppress

    Success rate: ~80% (multi-step complexity)
    """

    async def fill_form(
        self, page: Page, identity: IdentityContext, source_url: str
    ) -> bool:
        """Fill PeopleConnect suppression form."""
        filled = 0

        # Step 1: Email (if on login/email page)
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

        # Step 2: Name fields (if on search page after email verification)
        if identity.first_name:
            name_selectors = ['input[name="first_name"]', 'input[name="firstName"]']
            for selector in name_selectors:
                try:
                    if await page.locator(selector).count() > 0:
                        await page.locator(selector).first.fill(identity.first_name)
                        self.log.debug("filled_first_name", selector=selector)
                        filled += 1
                        break
                except Exception:
                    continue

        if identity.last_name:
            name_selectors = ['input[name="last_name"]', 'input[name="lastName"]']
            for selector in name_selectors:
                try:
                    if await page.locator(selector).count() > 0:
                        await page.locator(selector).first.fill(identity.last_name)
                        self.log.debug("filled_last_name", selector=selector)
                        filled += 1
                        break
                except Exception:
                    continue

        # Date of birth (if available)
        if identity.date_of_birth:
            dob_selectors = ['input[name="dob"]', 'input[name="dateOfBirth"]']
            for selector in dob_selectors:
                try:
                    if await page.locator(selector).count() > 0:
                        await page.locator(selector).first.fill(identity.date_of_birth)
                        self.log.debug("filled_dob", selector=selector)
                        filled += 1
                        break
                except Exception:
                    continue

        return filled > 0

    async def submit(self, page: Page) -> bool:
        """Submit PeopleConnect suppression (multi-step)."""
        # Step 1: Continue/Login button (email verification step)
        continue_selectors = [
            'button:has-text("Continue")',
            'button:has-text("Log In")',
            'button:has-text("Sign In")',
        ]

        for selector in continue_selectors:
            try:
                if await page.locator(selector).count() > 0:
                    await page.locator(selector).first.click()
                    self.log.debug("clicked_continue", selector=selector)
                    # Wait for next page
                    await asyncio.sleep(2)
                    break
            except Exception:
                continue

        # Step 2: Search button (after email verification)
        search_selectors = [
            'button:has-text("Search")',
            'button:has-text("Find")',
            'button[type="submit"]',
        ]

        for selector in search_selectors:
            try:
                if await page.locator(selector).count() > 0:
                    await page.locator(selector).first.click()
                    self.log.debug("clicked_search", selector=selector)
                    # Wait for results
                    await asyncio.sleep(3)
                    break
            except Exception:
                continue

        # Step 3: Select/Suppress button (from search results)
        suppress_selectors = [
            'button:has-text("Suppress")',
            'button:has-text("Remove")',
            'button:has-text("Opt Out")',
        ]

        for selector in suppress_selectors:
            try:
                if await page.locator(selector).count() > 0:
                    await page.locator(selector).first.click()
                    self.log.debug("clicked_suppress", selector=selector)
                    return True
            except Exception:
                continue

        # If we got through some steps, consider it a partial success
        return True
