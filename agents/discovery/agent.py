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
from shared.metrics import get_metrics_collector, OperationType, OperationStatus
from shared.models import DiscoveryReport, DiscoveryResult, IdentityContext, SourceOutcome
from shared.page_text import MIN_PAGE_TEXT_CHARS, extract_text, looks_blocked, surname_of, window_around_surname
from shared.rate_limiter import RateLimitExceeded, get_rate_limiter
from shared.safe_fetch import FetchBlocked, ResponseTooLarge, safe_get

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

# Sources marked "manual" show results only through an interactive search (a form, then a
# sign-up flow); there is no address with a name in it that returns a listing, so they cannot
# be checked automatically. As of Nov 2026: their own help pages describe searching by form, and
# the address patterns that were configured for them returned 404 (Ownerly is a property-records
# site, not a people search). They are not fetched and are reported to the customer as needing
# a manual search, instead of being counted as checked or as failures.
# Supported data broker sources
# FREE tier: 4 brokers (acquisition tier)
# PERSONAL: those 4 plus 11 more (15 total)
# PREMIUM: all of Personal's plus 19 more (34 total), the "expanded source coverage" of the plan.
# Every source here needs an entry in removal/agent.py OPT_OUT_REGISTRY, or a customer would
# be offered a removal that cannot run (tests/test_discovery.py checks this).
# The URL templates follow each site's public search-URL pattern; they have not been
# exercised against the live sites from the test suite.
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
        "manual": True,
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
        "manual": True,
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
        "manual": True,
    },
    {
        "name": "ownerly.com",
        "url": "https://www.ownerly.com/people/{name_slug}",
        "tier": "personal",
        "manual": True,
    },
    {
        "name": "instantcheckmate.com",
        "url": "https://www.instantcheckmate.com/people/{name_slug}/",
        "tier": "personal",
        "manual": True,
    },
    {
        "name": "radaris.com",
        "url": "https://radaris.com/p/{name_slug}/",
        "tier": "personal",
    },
    # PREMIUM-only expanded coverage (4 additional, 19 total)
    {
        "name": "truthfinder.com",
        "url": "https://www.truthfinder.com/results/?firstName={first}&lastName={last}",
        "tier": "premium",
    },
    {
        "name": "thatsthem.com",
        "url": "https://thatsthem.com/name/{name_slug}",
        "tier": "premium",
    },
    {
        "name": "smartbackgroundchecks.com",
        "url": "https://www.smartbackgroundchecks.com/people/{name_slug}",
        "tier": "premium",
    },
    {
        "name": "advancedbackgroundchecks.com",
        "url": "https://www.advancedbackgroundchecks.com/name/{name_slug}",
        "tier": "premium",
    },
    # Phase 1 Expansion - Low-Hanging Fruit (15 additional, 34 total)
    {
        "name": "peekyou.com",
        "url": "https://www.peekyou.com/{name_slug}",
        "tier": "premium",
    },
    {
        "name": "addresses.com",
        "url": "https://www.addresses.com/people/{name_slug}",
        "tier": "premium",
    },
    {
        "name": "usphonebook.com",
        "url": "https://www.usphonebook.com/{name_slug}",
        "tier": "premium",
    },
    {
        "name": "anywho.com",
        "url": "https://www.anywho.com/people/{name}",
        "tier": "premium",
    },
    {
        "name": "411.com",
        "url": "https://www.411.com/name/{name_slug}",
        "tier": "premium",
    },
    {
        "name": "checkpeople.com",
        "url": "https://www.checkpeople.com/landingpages/namesearch?fn={first}&ln={last}",
        "tier": "premium",
    },
    {
        "name": "peoplewiz.com",
        "url": "https://www.peoplewiz.com/people/{name_slug}",
        "tier": "premium",
    },
    {
        "name": "infotracer.com",
        "url": "https://infotracer.com/people/{first}-{last}",
        "tier": "premium",
    },
    {
        "name": "publicrecordsnow.com",
        "url": "https://www.publicrecordsnow.com/people/{name_slug}",
        "tier": "premium",
    },
    {
        "name": "myaddressbook.com",
        "url": "https://www.myaddressbook.com/search.php?q={name}",
        "tier": "premium",
    },
    {
        "name": "neighbor.report",
        "url": "https://neighbor.report/search?name={name}",
        "tier": "premium",
    },
    {
        "name": "pipl.com",
        "url": "https://pipl.com/search/?q={name}",
        "tier": "premium",
    },
    {
        "name": "zoominfo.com",
        "url": "https://www.zoominfo.com/p/{name_slug}",
        "tier": "premium",
        "manual": True,  # Requires business account for full access
    },
    {
        "name": "contactout.com",
        "url": "https://contactout.com/search/{name_slug}",
        "tier": "premium",
        "manual": True,  # B2B focus, requires account
    },
    {
        "name": "rocketreach.co",
        "url": "https://rocketreach.co/person/{name_slug}",
        "tier": "premium",
        "manual": True,  # B2B focus, requires account
    },
]

# Tier hierarchy: free gets 4 brokers, personal 15, premium 19. A tier gets every source
# at or below its own level.
TIER_ORDER = {"free": 0, "personal": 1, "premium": 2}


# HTTP statuses, grouped by what they mean for the customer. 429 and 5xx never get this far
# (they raise SourceUnhealthy first); any other status is treated as "unavailable".
BLOCKED_STATUSES = frozenset({401, 403, 451})
NOT_FOUND_STATUSES = frozenset({400, 404, 405, 410})


def outcome_for_status(status: int) -> SourceOutcome:
    """What a non-200 answer means: the site refuses us, the address is wrong, or it is down."""
    if status in BLOCKED_STATUSES:
        return SourceOutcome.BLOCKED
    if status in NOT_FOUND_STATUSES:
        return SourceOutcome.NOT_FOUND
    return SourceOutcome.UNAVAILABLE


class DiscoveryAgent:
    """
    Searches supported data brokers for a customer's personal information.
    Uses Playwright for JavaScript-heavy sites and httpx for simple ones.
    """

    def __init__(self, tier: str = "free") -> None:
        self.tier = tier
        # What happened at each source during this run, filled in by _fetch_source.
        self._outcomes: dict[str, SourceOutcome] = {}
        self.cfg = get_config()
        self._tier_level = TIER_ORDER.get(tier, 0)
        self._rate_limiter = get_rate_limiter()
        self._metrics = get_metrics_collector()

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
        start_time = asyncio.get_event_loop().time()
        headers = {
            "User-Agent": (
                "Mozilla/5.0 (compatible; VectorScrubBot/1.0; "
                "+https://vectorscrub.com/bot)"
            ),
        }
        host = urlparse(url).hostname or source_name
        breaker = get_circuit_breaker(f"discovery:{host}", SOURCE_BREAKER)

        # Check rate limit before attempting request
        try:
            await self._rate_limiter.check_rate_limit(source_name, "discovery")
        except RateLimitExceeded as exc:
            log.info(
                "discovery_rate_limited",
                source=source_name,
                retry_after=exc.retry_after_seconds,
            )
            self._outcomes[source_name] = SourceOutcome.UNAVAILABLE
            return []

        async def get():
            # safe_get follows redirects itself, one validated hop at a time, stays on this
            # source's domain, refuses private/loopback/link-local addresses, and caps the body.
            fetched = await safe_get(
                client,
                url,
                headers=headers,
                timeout=self.cfg.request_timeout_seconds,
                allowed_domain=source_name,
            )
            if fetched.status == 429 or fetched.status >= 500:
                raise SourceUnhealthy(fetched.status)
            return fetched

        try:
            resp = await breaker.execute_async(get)
        except CircuitOpenError:
            log.info("discovery_skipped_circuit_open", source=source_name)
            self._outcomes[source_name] = SourceOutcome.UNAVAILABLE
            await self._metrics.record_operation(
                broker=source_name,
                operation=OperationType.DISCOVERY,
                status=OperationStatus.FAILURE,
                metadata={"reason": "circuit_open"},
            )
            return []
        except SourceUnhealthy as exc:
            log.warning("discovery_source_unhealthy", source=source_name, status=exc.args[0])
            self._outcomes[source_name] = SourceOutcome.UNAVAILABLE
            await self._metrics.record_operation(
                broker=source_name,
                operation=OperationType.DISCOVERY,
                status=OperationStatus.FAILURE,
                metadata={"reason": "unhealthy", "status": exc.args[0]},
            )
            return []
        except FetchBlocked as exc:
            log.warning("discovery_fetch_blocked", source=source_name, reason=exc.reason)
            self._outcomes[source_name] = SourceOutcome.UNAVAILABLE
            await self._metrics.record_operation(
                broker=source_name,
                operation=OperationType.DISCOVERY,
                status=OperationStatus.BLOCKED,
                metadata={"reason": exc.reason},
            )
            return []
        except ResponseTooLarge:
            log.warning("discovery_response_too_large", source=source_name)
            self._outcomes[source_name] = SourceOutcome.UNREADABLE
            await self._metrics.record_operation(
                broker=source_name,
                operation=OperationType.DISCOVERY,
                status=OperationStatus.FAILURE,
                metadata={"reason": "response_too_large"},
            )
            return []
        except (httpx.HTTPError, asyncio.TimeoutError) as exc:
            # Type only: transport error text can embed the URL, and the URL embeds the name.
            log.warning("discovery_fetch_failed", source=source_name, error_type=type(exc).__name__)
            self._outcomes[source_name] = SourceOutcome.UNAVAILABLE
            status = OperationStatus.TIMEOUT if isinstance(exc, asyncio.TimeoutError) else OperationStatus.FAILURE
            await self._metrics.record_operation(
                broker=source_name,
                operation=OperationType.DISCOVERY,
                status=status,
                metadata={"error_type": type(exc).__name__},
            )
            return []

        if resp.status != 200:
            log.debug("discovery_non_200", source=source_name, status=resp.status)
            self._outcomes[source_name] = outcome_for_status(resp.status)
            return []

        # Visible text, not raw HTML, so markup and scripts do not count as matches.
        page_text = extract_text(resp.text)
        if len(page_text) < MIN_PAGE_TEXT_CHARS or looks_blocked(page_text):
            log.debug("discovery_unusable_page", source=source_name, length=len(page_text))
            # A CAPTCHA / "verify you are human" page is a refusal; an empty shell is not.
            self._outcomes[source_name] = (
                SourceOutcome.BLOCKED if looks_blocked(page_text) else SourceOutcome.UNREADABLE
            )
            return []
        # From here the page was read, whether or not it lists the customer.
        self._outcomes[source_name] = SourceOutcome.CHECKED

        # Record successful request for rate limiting
        await self._rate_limiter.record_request(source_name, "discovery")

        # Record metrics
        duration_ms = (asyncio.get_event_loop().time() - start_time) * 1000
        await self._metrics.record_operation(
            broker=source_name,
            operation=OperationType.DISCOVERY,
            status=OperationStatus.SUCCESS,
            duration_ms=duration_ms,
        )

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
        return (await self.run_report(identity)).candidates

    async def run_report(self, identity: IdentityContext) -> DiscoveryReport:
        """
        Like `run`, and also reports what happened at every source, so a scan that could
        only read a few sites is not mistaken for one that read them all.
        """
        self._outcomes = {}
        all_targets = self._build_search_urls(identity)
        manual = {src["name"] for src in self._sources_for_tier() if src.get("manual")}
        search_targets = [t for t in all_targets if t[0] not in manual]
        for name in manual:
            self._outcomes[name] = SourceOutcome.NOT_SUPPORTED
        log.info(
            "discovery_started",
            customer_id=str(identity.customer_id),
            source_count=len(all_targets),
            fetchable=len(search_targets),
        )

        all_results: list[DiscoveryResult] = []
        sem = asyncio.Semaphore(self.cfg.max_concurrent_sources)

        async def fetch_with_sem(client, name, url):
            async with sem:
                return await self._fetch_source(client, name, url, identity)

        async with httpx.AsyncClient() as client:
            tasks = [fetch_with_sem(client, name, url) for name, url in search_targets]
            batches = await asyncio.gather(*tasks, return_exceptions=True)

        for (name, _), batch in zip(search_targets, batches):
            if isinstance(batch, Exception):
                log.error("discovery_source_error", error_type=type(batch).__name__)
                self._outcomes[name] = SourceOutcome.UNAVAILABLE
                continue
            all_results.extend(batch)

        # Every source gets an entry, in the order searched; one that recorded nothing is
        # treated as not checked rather than silently dropped.
        outcomes = {
            name: self._outcomes.get(name, SourceOutcome.UNAVAILABLE) for name, _ in all_targets
        }
        report = DiscoveryReport(candidates=all_results, outcomes=outcomes)

        log.info(
            "discovery_completed",
            customer_id=str(identity.customer_id),
            candidates_found=len(all_results),
            sources_checked=report.checked,
            sources_total=report.total,
        )
        return report
