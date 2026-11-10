"""
Discovery Agent — Phase 1
Searches people-search engines and public directories for a customer's
personal information. Returns candidate pages, with the relevant page text,
for the Detection Agent to verify.

Architecture note: This agent receives ONLY the minimum PII needed to
search each source. It never logs raw PII values.
"""
import asyncio
from urllib.parse import urlparse

import httpx
import structlog
from tenacity import retry, stop_after_attempt, wait_exponential

from shared.circuit_breaker import CircuitBreakerConfig, CircuitOpenError, get_circuit_breaker
from shared.config import get_config
from shared.models import DiscoveryResult, IdentityContext
from shared.page_text import MIN_PAGE_TEXT_CHARS, extract_text, looks_blocked, surname_of, window_around_surname

# Characters of page text handed to the detection agent per candidate. The detection agent
# redacts it (shared/pii_guard.py) and then sends at most its own cap to the model.
CANDIDATE_TEXT_CHARS = 3000

# A source that keeps failing or rate-limiting us is left alone for a few minutes
# instead of being hammered by every customer's scan.
SOURCE_BREAKER = CircuitBreakerConfig(
    failure_threshold=5, timeout_seconds=300, success_threshold=2, request_timeout_seconds=60
)


class SourceUnhealthy(Exception):
    """The source answered 429 or 5xx: counts against its circuit breaker."""

log = structlog.get_logger(__name__)

# Supported data broker sources
# FREE tier: 4 brokers (acquisition tier)
# PERSONAL/PREMIUM: All 15 brokers
DISCOVERY_SOURCES = [
    # FREE tier brokers (4)
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
        "name": "truepeoplesearch.com",
        "url": "https://www.truepeoplesearch.com/results?name={name}",
        "tier": "free",
    },
    # PERSONAL+ brokers (11 additional, 15 total)
    {
        "name": "fastpeoplesearch.com",
        "url": "https://www.fastpeoplesearch.com/name/{name_slug}",
        "tier": "personal",
    },
    {
        "name": "familytreenow.com",
        "url": "https://www.familytreenow.com/search/genealogy/results?first={first}&last={last}",
        "tier": "personal",
    },
    {
        "name": "nuwber.com",
        "url": "https://nuwber.com/search?name={name}",
        "tier": "personal",
    },
    {
        "name": "beenverified.com",
        "url": "https://www.beenverified.com/people/{name_slug}/",
        "tier": "personal",
    },
    {
        "name": "intelius.com",
        "url": "https://www.intelius.com/people-search/",
        "tier": "personal",
    },
    {
        "name": "mylife.com",
        "url": "https://www.mylife.com/people-search/name/{name_slug}/",
        "tier": "personal",
    },
    {
        "name": "ussearch.com",
        "url": "https://www.ussearch.com/search/people?firstname={first}&lastname={last}",
        "tier": "personal",
    },
    {
        "name": "peoplelooker.com",
        "url": "https://www.peoplelooker.com/results?fn={first}&ln={last}",
        "tier": "personal",
    },
    {
        "name": "ownerly.com",
        "url": "https://www.ownerly.com/people/{name_slug}",
        "tier": "personal",
    },
    {
        "name": "instantcheckmate.com",
        "url": "https://www.instantcheckmate.com/people/{name_slug}/",
        "tier": "personal",
    },
    {
        "name": "radaris.com",
        "url": "https://radaris.com/p/{name_slug}/",
        "tier": "personal",
    },
]

# Tier hierarchy: free gets 4 brokers, all paid tiers get all 15
TIER_ORDER = {"free": 0, "personal": 1, "premium": 1}


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

        # Extract first and last name for sources that need them separately
        name_parts = identity.full_name.split()
        first_name = name_parts[0] if name_parts else ""
        last_name = name_parts[-1] if len(name_parts) > 1 else ""

        results = []
        for source in self._sources_for_tier():
            url = (
                source["url"]
                .replace("{name}", name_query)
                .replace("{name_slug}", name_slug)
                .replace("{first}", first_name)
                .replace("{last}", last_name)
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
        host = urlparse(url).hostname or source_name
        breaker = get_circuit_breaker(f"discovery:{host}", SOURCE_BREAKER)

        async def get() -> httpx.Response:
            resp = await client.get(
                url, headers=headers, follow_redirects=True, timeout=self.cfg.request_timeout_seconds
            )
            if resp.status_code == 429 or resp.status_code >= 500:
                raise SourceUnhealthy(resp.status_code)
            return resp

        try:
            resp = await breaker.execute_async(get)
        except CircuitOpenError:
            log.info("discovery_skipped_circuit_open", source=source_name)
            return []
        except SourceUnhealthy as exc:
            log.warning("discovery_source_unhealthy", source=source_name, status=exc.args[0])
            return []
        except (httpx.HTTPError, asyncio.TimeoutError) as exc:
            # Type only: transport error text can embed the URL, and the URL embeds the name.
            log.warning("discovery_fetch_failed", source=source_name, error_type=type(exc).__name__)
            return []

        if resp.status_code != 200:
            log.debug("discovery_non_200", source=source_name, status=resp.status_code)
            return []

        # Visible text, not raw HTML, so markup and scripts do not count as matches.
        page_text = extract_text(resp.text)
        if len(page_text) < MIN_PAGE_TEXT_CHARS or looks_blocked(page_text):
            log.debug("discovery_unusable_page", source=source_name, length=len(page_text))
            return []
        text = page_text.lower()
        results = []

        # Lightweight keyword matching: name parts and address parts
        name_parts = [p.lower() for p in identity.full_name.split() if len(p) > 2]

        matches = sum(1 for part in name_parts if part in text)
        confidence = min(matches / max(len(name_parts), 1), 1.0)

        # Boost confidence if city/state from address appears
        if identity.current_address:
            addr_parts = identity.current_address.split(",")
            if any(p.strip().lower() in text for p in addr_parts if p.strip()):
                confidence = min(confidence + 0.3, 1.0)

        if confidence >= 0.4:
            # Hand the detection agent the part of the page that is about this customer.
            # (Before this, the snippet was a placeholder string, so the detection model was
            # never shown any page content.) Detection redacts it before any model sees it.
            snippet = window_around_surname(page_text, surname_of(identity.full_name), CANDIDATE_TEXT_CHARS)
            if snippet is None:
                snippet = page_text[:CANDIDATE_TEXT_CHARS]
            results.append(
                DiscoveryResult(
                    source_name=source_name,
                    source_url=url,
                    raw_text_snippet=snippet,
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
