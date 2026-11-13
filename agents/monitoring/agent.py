"""
Monitoring Agent — removal verification and reappearance detection.

Two jobs, both driven by the orchestrator's monitoring worker:

* verify_removal:      a removal request was submitted; is the record actually gone?
* check_reappearance:  a record was verified removed; has it come back?

Both reduce to the same question — "does this broker page still show this
customer?" — answered by _inspect_page(), which returns PRESENT, ABSENT or
UNKNOWN. UNKNOWN is a first-class answer: the customer is told a record is gone
only on positive evidence, never because a check failed or was blocked.

Positive evidence for ABSENT is one of:
  * the page is gone (HTTP 404/410),
  * the page is a substantial, server-rendered page at the SAME address that we asked for,
    and the customer's surname does not appear anywhere on it (checked locally, no LLM
    call), or
  * the LLM, shown the page text and *anonymized* identity tokens, says "no match" with
    real confidence.

Everything else is UNKNOWN and is retried soon by the backend rather than being reported
as a result: fetch errors, 403/429/5xx, CAPTCHA or bot-wall pages, a redirect to a different
page (often the homepage or a search page, which never mentions anyone), client-rendered
shells and "enable JavaScript" pages, pages too thin to judge, an LLM that is down, and an
LLM verdict that is low-confidence or ambiguous.

Privacy: raw PII is never logged and never sent to the LLM. Only page text and
anonymized tokens (see DetectionAgent._anonymize_identity) reach the model.
"""
from __future__ import annotations

from typing import TYPE_CHECKING

import asyncio
from dataclasses import dataclass
from enum import Enum
from urllib.parse import urlparse

import httpx
import structlog

from detection.agent import DetectionAgent
from shared.backend_client import BackendClient
from shared.circuit_breaker import CircuitBreakerConfig, CircuitOpenError, get_circuit_breaker
from shared.models import DiscoveryResult
from shared.page_text import (
    MIN_PAGE_TEXT_CHARS,
    extract_text,
    looks_blocked,
    looks_js_shell,
    surname_of,
    window_around_surname,
)
from shared.safe_fetch import FetchBlocked, ResponseTooLarge, safe_get

if TYPE_CHECKING:
    from shared.models import IdentityContext

log = structlog.get_logger(__name__)

# Confidence needed to call a page PRESENT at all (same bar discovery used to
# confirm the exposure in the first place).
PRESENT_MIN_CONFIDENCE = 0.5
# Higher bar to alert a customer that a *removed* record has reappeared, to keep
# false alarms down.
REAPPEARANCE_MIN_CONFIDENCE = 0.7
# Confidence the LLM needs behind a "this page is not the customer" verdict before that
# counts as the record being gone. Anything between is ambiguous and stays UNKNOWN.
ABSENT_MIN_CONFIDENCE = 0.7
# "The surname is not on this page" only means something if there is enough page to have
# found it on. A few hundred characters is a stub, a notice, or a login wall.
MIN_ABSENT_TEXT_CHARS = 500

# How much page text (centred on the customer's surname) is shown to the LLM.
# Kept deliberately small: this is third-party page content that can include the
# customer's own details and relatives' names. Discovery sends 1,500 characters;
# 3,000 leaves room to centre on the right record on a long results page.
LLM_WINDOW_CHARS = 3000

# A source that keeps failing is left alone for a while instead of being hammered.
FETCH_BREAKER = CircuitBreakerConfig(
    failure_threshold=5, timeout_seconds=300, success_threshold=2, request_timeout_seconds=30
)


class SourceUnhealthy(Exception):
    """The source answered 429 or 5xx: counts against its circuit breaker."""


class MonitoringOutcome(str, Enum):
    """Result of a reappearance check. Values match the backend API."""
    CLEAR = "clear"
    REAPPEARED = "reappeared"
    INCONCLUSIVE = "inconclusive"


class VerificationOutcome(str, Enum):
    """Result of a removal verification. Values match the backend API."""
    REMOVED = "removed"
    STILL_PRESENT = "still_present"
    INCONCLUSIVE = "inconclusive"


class PageState(str, Enum):
    PRESENT = "present"
    ABSENT = "absent"
    UNKNOWN = "unknown"


@dataclass
class _Fetch:
    kind: str  # "ok" | "gone" | "error"
    html: str = ""
    # The request ended up on a different page than the one asked for (e.g. a removed profile
    # redirecting to the homepage). Such a page says nothing about the original record.
    moved: bool = False
    # Lowercased path finally served; only used to tell a canonical redirect from a generic one.
    final_path: str = ""


class MonitoringAgent:
    """Re-checks broker pages to verify removals and detect reappearance."""

    def __init__(self) -> None:
        self.backend = BackendClient()
        self.detection_agent = DetectionAgent()

    # ------------------------------------------------------------------
    # Public API
    # ------------------------------------------------------------------

    async def verify_removal(
        self,
        removal_request_id: str,
        customer_id: str,
        source_name: str,
        source_url: str,
        identity: IdentityContext,
    ) -> VerificationOutcome:
        """Check a submitted removal and report the result to the backend."""
        state, confidence = await self._inspect_page(source_name, source_url, identity)

        if state is PageState.ABSENT:
            outcome = VerificationOutcome.REMOVED
        elif state is PageState.PRESENT:
            outcome = VerificationOutcome.STILL_PRESENT
        else:
            outcome = VerificationOutcome.INCONCLUSIVE

        log.info(
            "removal_verification_checked",
            removal_request_id=removal_request_id,
            source=source_name,
            outcome=outcome.value,
            confidence=confidence,
        )
        await self.backend.report_verification_result(
            removal_request_id, customer_id, outcome.value
        )
        return outcome

    async def check_reappearance(
        self,
        exposure_id: str,
        customer_id: str,
        source_name: str,
        source_url: str,
        identity: IdentityContext,
    ) -> MonitoringOutcome:
        """
        Check whether a removed record is back and report the result to the
        backend, which reactivates the exposure and alerts the customer. This never
        submits a new removal: that always needs fresh customer authorization.
        """
        state, confidence = await self._inspect_page(source_name, source_url, identity)

        if state is PageState.PRESENT and confidence >= REAPPEARANCE_MIN_CONFIDENCE:
            outcome = MonitoringOutcome.REAPPEARED
        elif state is PageState.ABSENT:
            outcome = MonitoringOutcome.CLEAR
        else:
            # UNKNOWN, or a match too weak to alarm on but too real to call "clear":
            # don't raise a false alarm, and don't claim the record is still gone either.
            outcome = MonitoringOutcome.INCONCLUSIVE

        log.info(
            "reappearance_checked",
            exposure_id=exposure_id,
            source=source_name,
            outcome=outcome.value,
            confidence=confidence,
        )
        await self.backend.report_monitoring_result(exposure_id, customer_id, outcome.value)
        return outcome

    # ------------------------------------------------------------------
    # Page inspection
    # ------------------------------------------------------------------

    async def _inspect_page(
        self, source_name: str, source_url: str, identity: IdentityContext
    ) -> tuple[PageState, float]:
        """Return (state, confidence) for "does this page still show the customer?"."""
        fetched = await self._fetch_url(source_url, allowed_domain=source_name)

        if fetched.kind == "gone":
            return PageState.ABSENT, 1.0
        if fetched.kind == "error":
            return PageState.UNKNOWN, 0.0
        surname = surname_of(identity.full_name)
        # A redirect is tolerated only when the page we landed on still names the customer
        # (a broker canonicalising a search URL to the person's results page). A redirect to a
        # homepage or a generic page says nothing about the record.
        if fetched.moved and not (surname and surname.lower() in fetched.final_path):
            log.info("page_redirected_elsewhere", source=source_name)
            return PageState.UNKNOWN, 0.0

        text = extract_text(fetched.html)
        if len(text) < MIN_PAGE_TEXT_CHARS:
            log.debug("page_too_thin", source=source_name, length=len(text))
            return PageState.UNKNOWN, 0.0
        if looks_blocked(text):
            log.info("page_looks_blocked", source=source_name)
            return PageState.UNKNOWN, 0.0
        if looks_js_shell(fetched.html, text):
            log.info("page_is_client_rendered", source=source_name)
            return PageState.UNKNOWN, 0.0

        if not surname:
            log.warning("no_surname_for_check", source=source_name)
            return PageState.UNKNOWN, 0.0

        window = window_around_surname(text, surname, LLM_WINDOW_CHARS)
        if window is None:
            if len(text) < MIN_ABSENT_TEXT_CHARS:
                log.debug("page_too_short_to_rule_out", source=source_name, length=len(text))
                return PageState.UNKNOWN, 0.0
            return PageState.ABSENT, 1.0

        candidate = DiscoveryResult(
            source_name=source_name,
            source_url=source_url,
            raw_text_snippet=window,
            confidence=1.0,
        )
        try:
            verdict = await self.detection_agent.assess_page(
                candidate, identity, max_chars=LLM_WINDOW_CHARS
            )
        except Exception as exc:
            # An LLM outage must never read as "the record is gone". Type only: the text of
            # an LLM/HTTP error can contain request details.
            log.error("assessment_failed", source=source_name, error_type=type(exc).__name__)
            return PageState.UNKNOWN, 0.0

        confidence = round(float(verdict.confidence), 2)
        if verdict.is_match and confidence >= PRESENT_MIN_CONFIDENCE:
            return PageState.PRESENT, confidence
        if not verdict.is_match and confidence >= ABSENT_MIN_CONFIDENCE:
            return PageState.ABSENT, confidence
        # A match the model is unsure of, or a non-match it is unsure of: not evidence of
        # removal, and not enough to say the record is back.
        return PageState.UNKNOWN, confidence

    async def _fetch_url(self, url: str, allowed_domain: str | None = None) -> _Fetch:
        """
        GET the page. 404/410 means the page is gone (a good outcome after a removal);
        any other non-200 or a transport error is inconclusive. Calls to one host go through
        a circuit breaker: after repeated failures or 429/5xx answers the host is left alone
        for a few minutes, and those checks come back inconclusive instead of piling on.

        The request goes through safe_get: only public addresses, only the source's own
        domain (including after redirects), and a cap on the body size.
        """
        headers = {
            "User-Agent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36",
            "Accept": "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
            "Accept-Language": "en-US,en;q=0.9",
        }
        host = urlparse(url).hostname or "unknown"
        breaker = get_circuit_breaker(f"monitor:{host}", FETCH_BREAKER)

        async def get():
            async with httpx.AsyncClient(timeout=20) as client:
                fetched = await safe_get(
                    client, url, headers=headers, timeout=20, allowed_domain=allowed_domain
                )
            if fetched.status == 429 or fetched.status >= 500:
                raise SourceUnhealthy(fetched.status)
            return fetched

        try:
            resp = await breaker.execute_async(get)
        except CircuitOpenError:
            log.info("fetch_skipped_circuit_open", host=host)
            return _Fetch("error")
        except SourceUnhealthy as exc:
            log.warning("fetch_failed", host=host, status_code=exc.args[0])
            return _Fetch("error")
        except FetchBlocked as exc:
            log.warning("fetch_blocked", host=host, reason=exc.reason)
            return _Fetch("error")
        except ResponseTooLarge:
            log.warning("fetch_response_too_large", host=host)
            return _Fetch("error")
        except (httpx.HTTPError, asyncio.TimeoutError) as exc:
            # Type only: transport error text can embed the URL, and URLs can embed names.
            log.warning("fetch_exception", host=host, error_type=type(exc).__name__)
            return _Fetch("error")

        if resp.status == 200:
            return _Fetch("ok", resp.text, moved=resp.moved, final_path=resp.final_path)
        if resp.status in (404, 410):
            return _Fetch("gone")
        log.warning("fetch_failed", host=host, status_code=resp.status)
        return _Fetch("error")
