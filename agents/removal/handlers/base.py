"""
Base class for site-specific broker removal handlers.

Each handler encapsulates the specific form-filling and submission logic
for a particular data broker's opt-out process.
"""
from abc import ABC, abstractmethod
from typing import Optional

from playwright.async_api import Page
import structlog

from shared.models import IdentityContext

log = structlog.get_logger(__name__)


class BrokerHandler(ABC):
    """
    Abstract base class for broker-specific removal handlers.

    Each broker implementation must provide:
    - fill_form: Navigate and fill the opt-out form with customer data
    - submit: Submit the form and wait for result page
    - extract_confirmation: Parse confirmation reference from result page (optional)
    """

    def __init__(self):
        self.log = log.bind(handler=self.__class__.__name__)

    @abstractmethod
    async def fill_form(
        self, page: Page, identity: IdentityContext, source_url: str
    ) -> bool:
        """
        Navigate to opt-out page and fill the form.

        Args:
            page: Playwright Page object (already on opt-out URL)
            identity: Customer identity data
            source_url: URL of the listing to remove

        Returns:
            True if form was filled successfully, False otherwise
        """
        pass

    @abstractmethod
    async def submit(self, page: Page) -> bool:
        """
        Submit the opt-out form and wait for navigation/result.

        Args:
            page: Playwright Page object (form already filled)

        Returns:
            True if submission succeeded (navigation occurred), False otherwise
        """
        pass

    async def extract_confirmation(self, page: Page, page_text: str) -> Optional[str]:
        """
        Extract confirmation reference from the result page.

        Override this if the broker has a specific confirmation format
        not covered by the generic regex patterns.

        Args:
            page: Playwright Page object on result page
            page_text: Visible text content of result page

        Returns:
            Confirmation reference/ID, or None if not found
        """
        return None

    async def has_captcha(self, page: Page) -> bool:
        """
        Check if page has a CAPTCHA challenge.

        Override this to detect broker-specific CAPTCHA implementations.

        Args:
            page: Playwright Page object

        Returns:
            True if CAPTCHA detected
        """
        selectors = [
            'iframe[src*="recaptcha"]',
            'iframe[src*="hcaptcha"]',
            'iframe[src*="challenges.cloudflare.com"]',
            ".g-recaptcha",
            ".h-captcha",
            ".cf-turnstile",
            "[data-sitekey]",
        ]
        for selector in selectors:
            if await page.locator(selector).count() > 0:
                return True
        return False
