"""
Discovery Agent — Phase 1
Searches people-search engines and public directories for a customer's
personal information. Returns candidate URLs for the Detection Agent to verify.

Architecture note: This agent receives ONLY the minimum PII needed to
search each source. It never logs raw PII values.
"""
import asyncio
from typing import AsyncGenerator

import httpx
import structlog
from tenacity import retry, stop_after_attempt, wait_exponential

from ..shared.config import get_config
from ..shared.models import DiscoveryResult, IdentityContext

log = structlog.get_logger(__name__)

# Supported data broker sources — MVP tier
# Each entry: (source_name, search_url_template, selectors)
DISCOVERY_SOURCES = [
    {
        "name": "spokeo.com",
        "url": "https://www.spokeo.com/search?q={name}",
        "tier": "free",
    },
    {
        "name": "whitepages.com",
        "url": "https://www.whitepages.com/name/{name_slug}",
        "tier": "free",
    },
    {
        "name": "peoplefinder.com",
        "url": "https://www.peoplefinder.com/search/?q={name}",
        "tier": "free",
    },
    {
        "name": "intelius.com",
        "url": "https://www.intelius.com/people-search/",
        "tier": "personal",
    },
    {
        "name": "beenverified.com",
        "url": "https://www.beenverified.com/people/{name_slug}/",
        "tier": "personal",
    },
    {
        "name": "instantcheckmate.com",
        "url": "https://www.instantcheckmate.com/people/{name_slug}/",
        "tier": "premium",
    },
    {
        "name": "radaris.com",
        "url": "https://radaris.com/p/{name_slug}/",
        "tier": "premium",
    },
    {
        "name": "mylife.com",
        "url": "https://www.mylife.com/people-search/name/{name_slug}/",
        "tier": "personal",
    },
]

TIER_ORDER = {"free": 0, "personal": 1, "premium": 2, "family": 2}


class DiscoveryAgent:
    """
    Searches supported data brokers for a customer's personal information.
    Uses Playwright for JavaScript-heavy sites and httpx for simple ones.
    """

    def __init__(self, tier: str = "free") -> None:
        self.tier = tier
        self.cfg = get_config()
        self._tier_level = TIER_ORDER.get(tier, 0)

    def _sources_for_tier(self) -> list[dict]:
        return [
            s for s in DISCOVERY_SOURCES
            if TIER_ORDER.get(s["tier"], 0) <= self._tier_level
        ]

    def _build_search_urls(self, identity: IdentityContext) -> list[tuple[str, str]]:
        """Generate (source_name, url) pairs for this customer."""
        name_slug = identity.full_name.lower().replace(" ", "-")
        name_query = identity.full_name.replace(" ", "+")
        results = []
        for source in self._sources_for_tier():
            url = (
                source["url"]
                .replace("{name}", name_query)
                .replace("{name_slug}", name_slug)
            )
            results.append((source["name"], url))
        return results

    @retry(stop=stop_after_attempt(3), wait=wait_exponential(multiplier=1, min=2, max=10))
    async def _fetch_source(
        self, client: httpx.AsyncClient, source_name: str, url: str, identity: IdentityContext
    ) -> list[DiscoveryResult]:
        """
        Fetch a single source and do a lightweight keyword scan for PII signals.
        A high-confidence result goes to the Detection Agent for deep analysis.
        """
        headers = {
            "User-Agent": (
                "Mozilla/5.0 (compatible; VectorScrubBot/1.0; "
                "+https://vectorscrub.com/bot)"
            ),
        }
        try:
            resp = await client.get(url, headers=headers, follow_redirects=True, timeout=self.cfg.request_timeout_seconds)
        except (httpx.ConnectError, httpx.TimeoutException) as exc:
            log.warning("discovery_fetch_failed", source=source_name, error=str(exc))
            return []

        if resp.status_code != 200:
            log.debug("discovery_non_200", source=source_name, status=resp.status_code)
            return []

        text = resp.text.lower()
        results = []

        # Lightweight keyword matching — name parts and email domain
        name_parts = [p.lower() for p in identity.full_name.split() if len(p) > 2]
        email_domains = [e.split("@")[-1].lower() for e in identity.email_addresses]

        matches = sum(1 for part in name_parts if part in text)
        confidence = min(matches / max(len(name_parts), 1), 1.0)

        # Boost confidence if city/state from address appears
        if identity.current_address:
            addr_parts = identity.current_address.split(",")
            if any(p.strip().lower() in text for p in addr_parts):
                confidence = min(confidence + 0.3, 1.0)

        if confidence >= 0.4:
            # Mask the name in the summary to avoid logging raw PII
            name_initials = " ".join(p[0] + "***" for p in identity.full_name.split())
            results.append(
                DiscoveryResult(
                    source_name=source_name,
                    source_url=url,
                    raw_text_snippet=f"[candidate match for {name_initials}]",
                    confidence=round(confidence, 2),
                )
            )

        return results

    async def run(self, identity: IdentityContext) -> list[DiscoveryResult]:
        """
        Run discovery across all sources for this tier.
        Returns a list of candidate DiscoveryResults for the Detection Agent.
        """
        search_targets = self._build_search_urls(identity)
        log.info(
            "discovery_started",
            customer_id=str(identity.customer_id),
            source_count=len(search_targets),
        )

        all_results: list[DiscoveryResult] = []
        sem = asyncio.Semaphore(self.cfg.max_concurrent_sources)

        async def fetch_with_sem(client, name, url):
            async with sem:
                return await self._fetch_source(client, name, url, identity)

        async with httpx.AsyncClient() as client:
            tasks = [fetch_with_sem(client, name, url) for name, url in search_targets]
            batches = await asyncio.gather(*tasks, return_exceptions=True)

        for batch in batches:
            if isinstance(batch, Exception):
                log.error("discovery_source_error", error=str(batch))
                continue
            all_results.extend(batch)

        log.info(
            "discovery_completed",
            customer_id=str(identity.customer_id),
            candidates_found=len(all_results),
        )
        return all_results
