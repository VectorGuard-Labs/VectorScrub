"""
Is each broker's public opt-out page still there?

Every night the worker fetches each broker's opt-out page (the addresses in OPT_OUT_REGISTRY,
which are public) and reports what happened, so a link that has gone dead is found by us, not
by a customer. The result feeds the dashboard's opt-out guide.

What this does and does not tell you:
  * It is a plain HTTP request, not the browser the removal executor drives. Some sites treat
    the two differently, so "reachable" here is a good sign, not a promise.
  * "Reachable" never means a removal works. Only an actual verified removal does, and that is
    tracked separately. Do not turn this check into a claim about removals.
  * No customer data is involved anywhere: the pages are public and nothing is submitted.

It does not try to get past bot protection. A site that refuses us is reported as blocked, and
a form with a CAPTCHA is reported as such: a person has to complete it.
"""
from __future__ import annotations

import asyncio
from urllib.parse import urlsplit

import httpx
import structlog

from shared.config import get_config
from shared.models import OptOutCheck, OptOutStatus, RemovalProcedure
from shared.page_text import extract_text, looks_blocked
from shared.safe_fetch import FetchBlocked, ResponseTooLarge, safe_get

log = structlog.get_logger(__name__)

# Said openly, as the discovery agent does.
HEADERS = {"User-Agent": "Mozilla/5.0 (compatible; VectorScrubBot/1.0; +https://vectorscrub.com/bot)"}

BLOCKED_STATUSES = frozenset({401, 403, 451})
NOT_FOUND_STATUSES = frozenset({400, 404, 405, 410})

# What the common CAPTCHA services put in the page that carries their widget.
CAPTCHA_WIDGET_MARKERS = (
    "g-recaptcha",
    "grecaptcha",
    "recaptcha/api",
    "recaptcha/enterprise",
    "h-captcha",
    "hcaptcha",
    "cf-turnstile",
    "challenges.cloudflare.com/turnstile",
)


def classify(status: int, html: str) -> OptOutStatus:
    """What a response to an opt-out page request means."""
    if status in BLOCKED_STATUSES:
        return OptOutStatus.BLOCKED
    if status in NOT_FOUND_STATUSES:
        return OptOutStatus.NOT_FOUND
    if status != 200:
        return OptOutStatus.UNAVAILABLE  # 429, 5xx, odd 4xx/3xx
    # A bot wall that answers 200 in place of the page ("verify you are human").
    if looks_blocked(extract_text(html)):
        return OptOutStatus.BLOCKED
    lowered = html.lower()
    if any(marker in lowered for marker in CAPTCHA_WIDGET_MARKERS):
        return OptOutStatus.CAPTCHA
    return OptOutStatus.REACHABLE


def _allowed_domain(url: str) -> str | None:
    """The site a redirect may stay on: the page's own host without a leading www., so a
    redirect from www.example.com to example.com is fine and one to another company is not."""
    host = urlsplit(url).hostname
    if not host:
        return None
    return host[4:] if host.startswith("www.") else host


class OptOutHealthChecker:
    def __init__(self, delay_seconds: float | None = None, timeout_seconds: float | None = None) -> None:
        cfg = get_config()
        self._delay = cfg.optout_health_delay_seconds if delay_seconds is None else delay_seconds
        self._timeout = cfg.request_timeout_seconds if timeout_seconds is None else timeout_seconds

    async def check_page(self, client: httpx.AsyncClient, source: str, url: str) -> OptOutCheck:
        """Fetch one opt-out page. Never raises; logs the site name and a fixed reason only."""
        try:
            fetched = await safe_get(
                client,
                url,
                headers=HEADERS,
                timeout=self._timeout,
                allowed_domain=_allowed_domain(url),
            )
        except FetchBlocked as exc:
            # Includes a redirect to another company's domain: the page is not where we say it is.
            log.warning("optout_health_fetch_blocked", source=source, reason=exc.reason)
            return OptOutCheck(source=source, status=OptOutStatus.UNAVAILABLE)
        except ResponseTooLarge:
            log.warning("optout_health_response_too_large", source=source)
            return OptOutCheck(source=source, status=OptOutStatus.UNAVAILABLE)
        except (httpx.HTTPError, asyncio.TimeoutError) as exc:
            log.warning("optout_health_fetch_failed", source=source, error_type=type(exc).__name__)
            return OptOutCheck(source=source, status=OptOutStatus.UNAVAILABLE)
        return OptOutCheck(
            source=source, status=classify(fetched.status, fetched.text), http_status=fetched.status
        )

    async def check_all(self, registry: dict[str, RemovalProcedure] | None = None) -> list[OptOutCheck]:
        """
        Check every web-form broker, one at a time with a pause between, so a nightly run is a
        trickle of ordinary page loads and not a burst. Email brokers have no page to fetch and
        are left out.
        """
        if registry is None:
            from removal.agent import OPT_OUT_REGISTRY as registry  # noqa: N811
        targets = [(name, p.opt_out_url) for name, p in sorted(registry.items()) if p.opt_out_url]
        results: list[OptOutCheck] = []
        async with httpx.AsyncClient() as client:
            for i, (name, url) in enumerate(targets):
                if i and self._delay:
                    await asyncio.sleep(self._delay)
                results.append(await self.check_page(client, name, url))
        counts = {s.value: sum(1 for r in results if r.status is s) for s in OptOutStatus}
        log.info("optout_health_checked", pages=len(results), **counts)
        return results
