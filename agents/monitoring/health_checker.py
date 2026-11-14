"""
Broker Health Checker - Nightly Verification System

Automatically checks all broker opt-out pages to detect:
- Site downtime
- URL changes (404s)
- New CAPTCHA implementations
- SSL/certificate issues
- Unexpected redirects

Run nightly via cron to proactively detect issues before customers encounter them.
"""
import asyncio
from dataclasses import dataclass
from datetime import datetime
from typing import Dict, List, Optional
from urllib.parse import urlparse

import httpx
from playwright.async_api import async_playwright, Page
import structlog

from discovery.agent import DISCOVERY_SOURCES
from removal.agent import OPT_OUT_REGISTRY
from shared.metrics import get_metrics_collector
from shared.config import get_config

log = structlog.get_logger(__name__)


@dataclass
class HealthCheckResult:
    """Result of a broker health check."""
    broker: str
    url: str
    timestamp: float
    is_healthy: bool
    http_status: Optional[int] = None
    response_time_ms: Optional[float] = None
    has_captcha: bool = False
    has_ssl_error: bool = False
    is_redirected: bool = False
    redirect_url: Optional[str] = None
    error_type: Optional[str] = None
    error_message: Optional[str] = None


class BrokerHealthChecker:
    """
    Automated health checker for all broker opt-out pages.

    Performs lightweight checks without submitting any data:
    1. HTTP GET request to opt-out page
    2. Verify page loads (200 OK)
    3. Check for CAPTCHA presence
    4. Verify SSL certificate
    5. Detect unexpected redirects
    """

    def __init__(self):
        self.config = get_config()
        self.metrics = get_metrics_collector()
        self._results: List[HealthCheckResult] = []

    async def _detect_captcha_in_html(self, html: str) -> bool:
        """Quick CAPTCHA detection from HTML content."""
        captcha_indicators = [
            "recaptcha",
            "hcaptcha",
            "g-recaptcha",
            "h-captcha",
            "cf-turnstile",
            "cloudflare",
            "data-sitekey",
        ]
        html_lower = html.lower()
        return any(indicator in html_lower for indicator in captcha_indicators)

    async def _check_broker_http(self, broker: str, url: str) -> HealthCheckResult:
        """
        Check broker health via HTTP request.

        Fast, lightweight check without browser automation.
        """
        start_time = asyncio.get_event_loop().time()
        result = HealthCheckResult(
            broker=broker,
            url=url,
            timestamp=start_time,
            is_healthy=False,
        )

        try:
            async with httpx.AsyncClient(
                follow_redirects=True,
                verify=True,
                timeout=30.0,
            ) as client:
                response = await client.get(
                    url,
                    headers={
                        "User-Agent": (
                            "Mozilla/5.0 (compatible; VectorScrubHealthBot/1.0; "
                            "+https://vectorscrub.com/health-checker)"
                        ),
                    },
                )

                duration_ms = (asyncio.get_event_loop().time() - start_time) * 1000

                result.http_status = response.status_code
                result.response_time_ms = duration_ms

                # Check for redirects
                if len(response.history) > 0:
                    original_host = urlparse(url).hostname
                    final_host = urlparse(str(response.url)).hostname
                    if original_host != final_host:
                        result.is_redirected = True
                        result.redirect_url = str(response.url)

                # Check for CAPTCHA in HTML
                if response.status_code == 200:
                    result.has_captcha = self._detect_captcha_in_html(response.text)

                # Healthy if 200 OK, no CAPTCHA, and not redirected to different domain
                result.is_healthy = (
                    response.status_code == 200
                    and not result.has_captcha
                    and not result.is_redirected
                )

        except httpx.HTTPStatusError as exc:
            result.http_status = exc.response.status_code
            result.error_type = "http_error"
            result.error_message = f"HTTP {exc.response.status_code}"

        except httpx.SSLError as exc:
            result.has_ssl_error = True
            result.error_type = "ssl_error"
            result.error_message = "SSL certificate error"

        except httpx.TimeoutException:
            result.error_type = "timeout"
            result.error_message = "Request timed out after 30s"

        except Exception as exc:
            result.error_type = type(exc).__name__
            result.error_message = str(exc)[:200]  # Truncate long errors

        return result

    async def _check_broker_browser(self, broker: str, url: str) -> HealthCheckResult:
        """
        Check broker health via browser automation.

        More thorough check that mimics actual removal submissions.
        Used when HTTP check detects potential issues.
        """
        start_time = asyncio.get_event_loop().time()
        result = HealthCheckResult(
            broker=broker,
            url=url,
            timestamp=start_time,
            is_healthy=False,
        )

        try:
            async with async_playwright() as p:
                browser = await p.chromium.launch(headless=True)
                try:
                    page = await browser.new_page()
                    await page.set_viewport_size({"width": 1280, "height": 1024})

                    # Navigate to page
                    response = await page.goto(url, timeout=30000)
                    await page.wait_for_load_state("networkidle", timeout=20000)

                    duration_ms = (asyncio.get_event_loop().time() - start_time) * 1000

                    result.http_status = response.status if response else None
                    result.response_time_ms = duration_ms

                    # Check for redirects
                    final_url = page.url
                    if final_url != url:
                        original_host = urlparse(url).hostname
                        final_host = urlparse(final_url).hostname
                        if original_host != final_host:
                            result.is_redirected = True
                            result.redirect_url = final_url

                    # Check for CAPTCHA
                    captcha_selectors = [
                        'iframe[src*="recaptcha"]',
                        'iframe[src*="hcaptcha"]',
                        'iframe[src*="challenges.cloudflare.com"]',
                        ".g-recaptcha",
                        ".h-captcha",
                        ".cf-turnstile",
                        "[data-sitekey]",
                    ]
                    for selector in captcha_selectors:
                        if await page.locator(selector).count() > 0:
                            result.has_captcha = True
                            break

                    # Healthy if loaded, no CAPTCHA, not redirected
                    result.is_healthy = (
                        result.http_status == 200
                        and not result.has_captcha
                        and not result.is_redirected
                    )

                finally:
                    await browser.close()

        except Exception as exc:
            result.error_type = type(exc).__name__
            result.error_message = str(exc)[:200]

        return result

    async def check_all_brokers(
        self,
        use_browser: bool = False,
        max_concurrent: int = 10,
    ) -> List[HealthCheckResult]:
        """
        Check health of all brokers in the registry.

        Args:
            use_browser: Use browser automation instead of HTTP (slower but thorough)
            max_concurrent: Maximum concurrent checks

        Returns:
            List of health check results
        """
        log.info("broker_health_check_started", use_browser=use_browser)

        # Get all unique broker URLs from opt-out registry
        broker_urls: Dict[str, str] = {}
        for broker_name, procedure in OPT_OUT_REGISTRY.items():
            if procedure.opt_out_url:
                broker_urls[broker_name] = procedure.opt_out_url

        log.info("checking_brokers", total_brokers=len(broker_urls))

        # Run checks concurrently
        semaphore = asyncio.Semaphore(max_concurrent)

        async def check_with_sem(broker: str, url: str):
            async with semaphore:
                if use_browser:
                    return await self._check_broker_browser(broker, url)
                else:
                    return await self._check_broker_http(broker, url)

        tasks = [
            check_with_sem(broker, url)
            for broker, url in broker_urls.items()
        ]

        results = await asyncio.gather(*tasks, return_exceptions=True)

        # Filter out exceptions and store results
        self._results = []
        for i, result in enumerate(results):
            if isinstance(result, Exception):
                broker = list(broker_urls.keys())[i]
                url = list(broker_urls.values())[i]
                log.error(
                    "health_check_exception",
                    broker=broker,
                    error_type=type(result).__name__,
                )
                # Create failed result
                self._results.append(
                    HealthCheckResult(
                        broker=broker,
                        url=url,
                        timestamp=asyncio.get_event_loop().time(),
                        is_healthy=False,
                        error_type=type(result).__name__,
                        error_message=str(result)[:200],
                    )
                )
            else:
                self._results.append(result)

        # Log summary
        healthy_count = sum(1 for r in self._results if r.is_healthy)
        unhealthy_count = len(self._results) - healthy_count

        log.info(
            "broker_health_check_completed",
            total_checked=len(self._results),
            healthy=healthy_count,
            unhealthy=unhealthy_count,
        )

        return self._results

    def get_unhealthy_brokers(self) -> List[HealthCheckResult]:
        """Get list of unhealthy brokers from last check."""
        return [r for r in self._results if not r.is_healthy]

    def get_captcha_brokers(self) -> List[HealthCheckResult]:
        """Get list of brokers with CAPTCHA detected."""
        return [r for r in self._results if r.has_captcha]

    async def save_results_to_redis(self) -> None:
        """Save health check results to Redis for monitoring."""
        import redis.asyncio as redis

        r = redis.Redis.from_url(
            self.config.redis_url,
            decode_responses=True,
        )

        try:
            pipe = r.pipeline()
            timestamp = datetime.utcnow().isoformat()

            # Store overall health summary
            summary = {
                "timestamp": timestamp,
                "total_brokers": len(self._results),
                "healthy": sum(1 for r in self._results if r.is_healthy),
                "unhealthy": sum(1 for r in self._results if not r.is_healthy),
                "with_captcha": sum(1 for r in self._results if r.has_captcha),
                "with_ssl_errors": sum(1 for r in self._results if r.has_ssl_error),
                "redirected": sum(1 for r in self._results if r.is_redirected),
            }

            pipe.hset("health_check:summary", mapping=summary)
            pipe.expire("health_check:summary", 7 * 24 * 3600)  # 7 days

            # Store individual broker results
            for result in self._results:
                key = f"health_check:broker:{result.broker}"
                data = {
                    "url": result.url,
                    "timestamp": timestamp,
                    "is_healthy": str(result.is_healthy),
                    "http_status": str(result.http_status or ""),
                    "response_time_ms": str(result.response_time_ms or ""),
                    "has_captcha": str(result.has_captcha),
                    "has_ssl_error": str(result.has_ssl_error),
                    "is_redirected": str(result.is_redirected),
                    "redirect_url": result.redirect_url or "",
                    "error_type": result.error_type or "",
                    "error_message": result.error_message or "",
                }
                pipe.hset(key, mapping=data)
                pipe.expire(key, 7 * 24 * 3600)  # 7 days

            await pipe.execute()
            log.info("health_check_results_saved", broker_count=len(self._results))

        finally:
            await r.aclose()

    def generate_report(self) -> str:
        """Generate human-readable health check report."""
        report_lines = [
            "=" * 80,
            "BROKER HEALTH CHECK REPORT",
            f"Timestamp: {datetime.utcnow().isoformat()}",
            "=" * 80,
            "",
        ]

        # Summary
        healthy = sum(1 for r in self._results if r.is_healthy)
        unhealthy = len(self._results) - healthy
        report_lines.extend([
            "SUMMARY:",
            f"  Total Brokers: {len(self._results)}",
            f"  Healthy: {healthy}",
            f"  Unhealthy: {unhealthy}",
            "",
        ])

        # Unhealthy brokers
        if unhealthy > 0:
            report_lines.extend([
                "UNHEALTHY BROKERS:",
                "",
            ])
            for result in self.get_unhealthy_brokers():
                report_lines.extend([
                    f"  ❌ {result.broker}",
                    f"     URL: {result.url}",
                    f"     Status: {result.http_status or 'N/A'}",
                    f"     Error: {result.error_message or 'N/A'}",
                ])
                if result.has_captcha:
                    report_lines.append("     ⚠️  CAPTCHA detected")
                if result.has_ssl_error:
                    report_lines.append("     ⚠️  SSL error")
                if result.is_redirected:
                    report_lines.append(f"     ⚠️  Redirected to: {result.redirect_url}")
                report_lines.append("")

        # CAPTCHA warnings
        captcha_brokers = self.get_captcha_brokers()
        if captcha_brokers:
            report_lines.extend([
                "CAPTCHA DETECTED:",
                "",
            ])
            for result in captcha_brokers:
                report_lines.append(f"  ⚠️  {result.broker} - {result.url}")
            report_lines.append("")

        report_lines.append("=" * 80)
        return "\n".join(report_lines)


async def run_health_check(use_browser: bool = False) -> BrokerHealthChecker:
    """
    Run a complete health check and return the checker with results.

    Args:
        use_browser: Use browser automation (slower but more accurate)

    Returns:
        BrokerHealthChecker instance with results
    """
    checker = BrokerHealthChecker()
    await checker.check_all_brokers(use_browser=use_browser)
    await checker.save_results_to_redis()

    # Print report
    print(checker.generate_report())

    # Log critical issues
    unhealthy = checker.get_unhealthy_brokers()
    if unhealthy:
        log.warning(
            "unhealthy_brokers_detected",
            count=len(unhealthy),
            brokers=[r.broker for r in unhealthy],
        )

    return checker


if __name__ == "__main__":
    """Run health check directly."""
    import sys

    use_browser = "--browser" in sys.argv

    asyncio.run(run_health_check(use_browser=use_browser))
