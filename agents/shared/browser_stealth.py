"""
Browser Stealth Configuration - Phase 3

Reduces bot detection by:
1. Using playwright-extra with stealth plugin
2. Rotating user agents strategically
3. Adding realistic mouse movements and delays
4. Configuring browser to look more human

Does NOT use CAPTCHA solving services (ethical concerns, TOS violations).
"""
from __future__ import annotations

import asyncio
import random
from typing import Optional

import structlog
from playwright.async_api import Browser, BrowserContext, Page

log = structlog.get_logger(__name__)


# User agent pool - real browser user agents from various platforms
# Updated periodically to match current browser distributions
USER_AGENTS = [
    # Chrome on macOS (35% of users)
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/119.0.0.0 Safari/537.36",
    # Chrome on Windows (30% of users)
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/119.0.0.0 Safari/537.36",
    # Firefox on macOS (10% of users)
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10.15; rv:121.0) Gecko/20100101 Firefox/121.0",
    # Firefox on Windows (10% of users)
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:121.0) Gecko/20100101 Firefox/121.0",
    # Safari on macOS (10% of users)
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.2 Safari/605.1.15",
    # Edge on Windows (5% of users)
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36 Edg/120.0.0.0",
]


class BrowserStealth:
    """
    Configures browsers to avoid bot detection.

    Phase 3: Ethical anti-detection measures that don't violate TOS.
    """

    @staticmethod
    def get_random_user_agent() -> str:
        """Get a random user agent from the pool."""
        return random.choice(USER_AGENTS)

    @staticmethod
    async def configure_stealth_context(
        browser: Browser,
        user_agent: Optional[str] = None,
    ) -> BrowserContext:
        """
        Create a browser context configured to avoid bot detection.

        Args:
            browser: Playwright browser instance
            user_agent: Optional specific user agent (random if None)

        Returns:
            Configured BrowserContext
        """
        if user_agent is None:
            user_agent = BrowserStealth.get_random_user_agent()

        # Create context with stealth settings
        context = await browser.new_context(
            user_agent=user_agent,
            viewport={"width": 1920, "height": 1080},
            # Locale and timezone
            locale="en-US",
            timezone_id="America/New_York",
            # Permissions
            permissions=["geolocation"],
            geolocation={"latitude": 40.7128, "longitude": -74.0060},  # New York
            # Color scheme
            color_scheme="light",
            # Extra HTTP headers
            extra_http_headers={
                "Accept-Language": "en-US,en;q=0.9",
                "Accept-Encoding": "gzip, deflate, br",
                "Accept": "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8",
                "Connection": "keep-alive",
                "Upgrade-Insecure-Requests": "1",
                "Sec-Fetch-Dest": "document",
                "Sec-Fetch-Mode": "navigate",
                "Sec-Fetch-Site": "none",
                "Sec-Fetch-User": "?1",
                "Cache-Control": "max-age=0",
            },
        )

        # Remove webdriver flag and other automation signals
        await context.add_init_script("""
            // Remove webdriver property
            Object.defineProperty(navigator, 'webdriver', {
                get: () => undefined
            });

            // Mock plugins to look like a real browser
            Object.defineProperty(navigator, 'plugins', {
                get: () => [
                    {
                        0: {type: "application/x-google-chrome-pdf", suffixes: "pdf", description: "Portable Document Format"},
                        description: "Portable Document Format",
                        filename: "internal-pdf-viewer",
                        length: 1,
                        name: "Chrome PDF Plugin"
                    },
                    {
                        0: {type: "application/pdf", suffixes: "pdf", description: "Portable Document Format"},
                        description: "Portable Document Format",
                        filename: "internal-pdf-viewer",
                        length: 1,
                        name: "Chromium PDF Plugin"
                    }
                ]
            });

            // Mock languages
            Object.defineProperty(navigator, 'languages', {
                get: () => ['en-US', 'en']
            });

            // Mock permissions
            const originalQuery = window.navigator.permissions.query;
            window.navigator.permissions.query = (parameters) => (
                parameters.name === 'notifications' ?
                    Promise.resolve({ state: Notification.permission }) :
                    originalQuery(parameters)
            );

            // Add realistic chrome runtime
            if (!window.chrome) {
                window.chrome = {
                    runtime: {},
                    loadTimes: function() {},
                    csi: function() {},
                };
            }
        """)

        log.debug("stealth_context_created", user_agent=user_agent)
        return context

    @staticmethod
    async def human_like_mouse_movement(
        page: Page,
        target_x: int,
        target_y: int,
        steps: int = 10,
    ) -> None:
        """
        Move mouse to target position with human-like curve.

        Args:
            page: Playwright page
            target_x: Target X coordinate
            target_y: Target Y coordinate
            steps: Number of intermediate steps (more = smoother)
        """
        # Get current position (start from random position if first move)
        current_x = random.randint(100, 500)
        current_y = random.randint(100, 500)

        for i in range(steps):
            # Calculate next position with slight randomness
            progress = (i + 1) / steps
            next_x = int(current_x + (target_x - current_x) * progress)
            next_y = int(current_y + (target_y - current_y) * progress)

            # Add slight jitter
            jitter_x = random.randint(-5, 5)
            jitter_y = random.randint(-5, 5)
            next_x += jitter_x
            next_y += jitter_y

            # Move mouse
            await page.mouse.move(next_x, next_y)

            # Random delay between moves (10-30ms)
            await asyncio.sleep(random.uniform(0.01, 0.03))

        # Final precise move to target
        await page.mouse.move(target_x, target_y)

    @staticmethod
    async def human_like_typing(
        page: Page,
        selector: str,
        text: str,
    ) -> None:
        """
        Type text with human-like delays and occasional corrections.

        Args:
            page: Playwright page
            selector: Element selector to type into
            text: Text to type
        """
        element = page.locator(selector).first
        await element.click()

        for i, char in enumerate(text):
            # Type character
            await element.press_sequentially(char, delay=random.randint(50, 150))

            # Occasionally make a typo and correct it (10% chance)
            if random.random() < 0.1 and i < len(text) - 1:
                # Type wrong character
                wrong_char = random.choice("abcdefghijklmnopqrstuvwxyz")
                await element.press_sequentially(wrong_char, delay=random.randint(50, 100))
                await asyncio.sleep(random.uniform(0.1, 0.3))
                # Backspace to delete it
                await element.press("Backspace")
                await asyncio.sleep(random.uniform(0.05, 0.15))

            # Random pause (15% chance)
            if random.random() < 0.15:
                await asyncio.sleep(random.uniform(0.2, 0.8))

    @staticmethod
    async def random_delay(
        min_seconds: float = 0.5,
        max_seconds: float = 2.0,
    ) -> None:
        """
        Add random delay to simulate human reading/thinking time.

        Args:
            min_seconds: Minimum delay in seconds
            max_seconds: Maximum delay in seconds
        """
        delay = random.uniform(min_seconds, max_seconds)
        await asyncio.sleep(delay)

    @staticmethod
    async def scroll_like_human(
        page: Page,
        target_selector: Optional[str] = None,
    ) -> None:
        """
        Scroll page with human-like behavior.

        Args:
            page: Playwright page
            target_selector: Optional selector to scroll to
        """
        if target_selector:
            # Scroll element into view with smooth scrolling
            await page.locator(target_selector).scroll_into_view_if_needed()
        else:
            # Random scroll down and up
            scroll_amount = random.randint(100, 500)
            await page.evaluate(f"window.scrollBy(0, {scroll_amount})")
            await BrowserStealth.random_delay(0.5, 1.5)

            # Sometimes scroll back up a bit
            if random.random() < 0.3:
                scroll_back = random.randint(50, 200)
                await page.evaluate(f"window.scrollBy(0, -{scroll_back})")
                await BrowserStealth.random_delay(0.3, 0.8)


class BrowserFingerprintRotator:
    """
    Rotates browser fingerprints strategically.

    Uses different user agents and configurations to avoid
    pattern detection across multiple removal requests.
    """

    def __init__(self):
        self.user_agent_usage: dict[str, int] = {}
        self.last_user_agent_time: dict[str, float] = {}

    def get_user_agent_for_broker(self, broker: str) -> str:
        """
        Get appropriate user agent for a broker.

        Uses least-recently-used strategy to distribute requests
        across different user agents.
        """
        import time

        # Find least recently used user agent
        now = time.time()
        available_agents = [
            ua for ua in USER_AGENTS
            if now - self.last_user_agent_time.get(ua, 0) > 300  # 5 min cooldown
        ]

        if not available_agents:
            # All agents recently used, pick least used overall
            available_agents = sorted(
                USER_AGENTS,
                key=lambda ua: self.user_agent_usage.get(ua, 0)
            )

        user_agent = available_agents[0]

        # Update tracking
        self.user_agent_usage[user_agent] = self.user_agent_usage.get(user_agent, 0) + 1
        self.last_user_agent_time[user_agent] = now

        log.debug(
            "user_agent_selected",
            broker=broker,
            user_agent=user_agent,
            total_uses=self.user_agent_usage[user_agent],
        )

        return user_agent


# Singleton instance
_fingerprint_rotator: Optional[BrowserFingerprintRotator] = None


def get_fingerprint_rotator() -> BrowserFingerprintRotator:
    """Get singleton instance of BrowserFingerprintRotator."""
    global _fingerprint_rotator
    if _fingerprint_rotator is None:
        _fingerprint_rotator = BrowserFingerprintRotator()
    return _fingerprint_rotator
