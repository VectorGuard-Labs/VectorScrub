"""
BeenVerified.com removal handler.

BeenVerified has an opt-out search form where you find your record
by entering personal details, then submit an opt-out request.
"""
import asyncio

from playwright.async_api import Page

from shared.models import IdentityContext
from .base import BrokerHandler


class BeenVerifiedHandler(BrokerHandler):
    """
    Handler for BeenVerified.com opt-out requests.

    BeenVerified opt-out process:
    1. Navigate to https://www.beenverified.com/app/optout/search
    2. Search for your record by name (and optionally age/location)
    3. Select your record from results
    4. Confirm opt-out request
    """

    async def fill_form(
        self, page: Page, identity: IdentityContext, source_url: str
    ) -> bool:
        """Fill BeenVerified opt-out search form."""
        try:
            # Wait for search form
            await page.wait_for_selector(
                'input[name="firstName"], input[name="fn"]',
                timeout=10000
            )

            # Parse name
            name_parts = identity.full_name.split()
            first_name = name_parts[0] if name_parts else ""
            last_name = name_parts[-1] if len(name_parts) > 1 else ""

            # Fill first name
            first_input = page.locator(
                'input[name="firstName"], input[name="fn"], input[placeholder*="First" i]'
            ).first
            if await first_input.count() > 0:
                await first_input.fill(first_name)
                self.log.debug("filled_beenverified_first_name")

            # Fill last name
            last_input = page.locator(
                'input[name="lastName"], input[name="ln"], input[placeholder*="Last" i]'
            ).first
            if await last_input.count() > 0:
                await last_input.fill(last_name)
                self.log.debug("filled_beenverified_last_name")

            # Fill optional state/city if present
            if identity.current_address:
                # Try to extract state from address
                address_parts = identity.current_address.split(",")
                if len(address_parts) >= 2:
                    # Last part often contains state
                    state_part = address_parts[-1].strip().split()[0]  # Get state abbreviation
                    state_input = page.locator(
                        'select[name="state"], input[name="state"]'
                    ).first
                    if await state_input.count() > 0:
                        # If it's a select, try to select the option
                        if (await state_input.evaluate("el => el.tagName")).lower() == "select":
                            try:
                                await state_input.select_option(label=state_part)
                                self.log.debug("selected_beenverified_state", state=state_part)
                            except Exception:
                                pass  # State selection failed, continue anyway
                        else:
                            await state_input.fill(state_part)
                            self.log.debug("filled_beenverified_state")

            # Click search button
            search_button = page.locator(
                'button[type="submit"], button:has-text("Search")'
            ).first
            if await search_button.count() > 0:
                await search_button.click()
                await page.wait_for_load_state("networkidle", timeout=15000)
                self.log.debug("submitted_beenverified_search")

            await asyncio.sleep(2)  # Wait for results to load

            # Select the first matching record
            # BeenVerified shows a list of records - click "Remove" on the first one
            remove_buttons = page.locator(
                'button:has-text("Remove"), a:has-text("Remove"), button:has-text("Opt Out")'
            )
            if await remove_buttons.count() > 0:
                await remove_buttons.first.click()
                await asyncio.sleep(1)
                self.log.debug("clicked_beenverified_remove_button")

            return True

        except Exception as exc:
            self.log.warning("beenverified_fill_failed", error_type=type(exc).__name__)
            return False

    async def submit(self, page: Page) -> bool:
        """Submit BeenVerified opt-out confirmation."""
        try:
            # After clicking Remove, there might be a confirmation step
            confirm_selectors = [
                'button[type="submit"]',
                'button:has-text("Confirm")',
                'button:has-text("Submit")',
                'button:has-text("Remove My Record")',
            ]

            for selector in confirm_selectors:
                button = page.locator(selector).first
                if await button.count() > 0:
                    await button.click()
                    self.log.debug("clicked_beenverified_confirm", selector=selector)

                    # Wait for result
                    try:
                        await page.wait_for_load_state("networkidle", timeout=15000)
                    except Exception:
                        await asyncio.sleep(2)

                    return True

            # If no confirmation button found, the removal might already be submitted
            self.log.debug("beenverified_no_confirm_button_assuming_submitted")
            return True

        except Exception as exc:
            self.log.warning("beenverified_submit_failed", error_type=type(exc).__name__)
            return False
