"""
Detection Agent — Phase 1
Receives candidate pages from the Discovery Agent and uses an LLM to classify
and confirm actual PII exposures.

SAFETY RULE (enforced in code, not by convention): no customer PII ever reaches
the LLM. This class is the ONLY place a model is called, and every payload goes
through shared.pii_guard first:

  * The customer's identity is never sent, not even masked hints. Matching against
    the customer's name, address, phone, email, and date of birth happens locally.
  * The page excerpt is redacted: the customer's values become [CUSTOMER_*]
    placeholders and other personal data becomes generic placeholders.
  * The page URL is not sent (profile URLs often contain the person's name).
  * Third parties' names (relatives, neighbours) are removed by pattern rules, a
    spaCy NER pass, and finally a deny-by-default vocabulary filter that drops every
    word that is not structural vocabulary (shared/pii_vocab.py).
  * Before the call, the excerpt is re-checked twice: no customer value may remain,
    and nothing outside the vocabulary may remain. Either failure blocks the call
    with OutboundPiiError and no request is made. If the NER model is required
    (default) but cannot be loaded, the call is blocked too.

Do not add another LLM call site anywhere in the agents package. If a new task
needs a model, route it through _run_classifier's guard.
"""
import asyncio
from dataclasses import dataclass
from typing import Optional
from urllib.parse import urlparse

import structlog
from langchain_openai import ChatOpenAI
from langchain_anthropic import ChatAnthropic
from langchain_core.messages import HumanMessage, SystemMessage
from pydantic import BaseModel, Field

from shared.config import get_config
from shared.models import (
    DetectionResult,
    DiscoveryResult,
    ExposureSeverity,
    IdentityContext,
    PiiCategory,
)
from shared.pii_guard import (
    Evidence,
    assert_clean,
    assert_vocabulary_only,
    build_pii_summary,
    redact_for_llm,
)
from shared.pii_ner import PersonDetector

log = structlog.get_logger(__name__)

DETECTION_SYSTEM_PROMPT = """You are a PII detection specialist for a privacy protection service.
You are given a REDACTED excerpt from a data broker web page. Personal details were replaced with
placeholder tokens before you received it, and you are never shown the customer's real details:

  [CUSTOMER_NAME] [CUSTOMER_ADDRESS] [CUSTOMER_PHONE] [CUSTOMER_EMAIL] [CUSTOMER_DOB]
      the text at that position matched the customer being protected.
  [SURNAME] [FIRST_NAME]
      only the surname / only the first name matched (could be a relative or a namesake).
  [NAME] [ADDRESS] [PHONE] [EMAIL] [DATE] [AGE] [ZIP] [SSN] [NUMBER]
      other personal data that did not match the customer.

Everything else on the page has been removed except a small set of structural words (age, born,
lives, phone, relatives, "no results found", ...). A "…" marks removed text. Do not try to infer
what was removed.

You are also given a summary of which customer fields were found in the full excerpt.

Your task is to determine:
1. Whether this page lists the customer (not merely someone who shares a surname).
2. What categories of PII about the customer are present.
3. The severity of the exposure.

Judge by how the tokens relate: [CUSTOMER_NAME] next to [CUSTOMER_ADDRESS] or [CUSTOMER_PHONE]
in the same record is a strong match; a lone [SURNAME] among many [NAME] tokens is not.

Respond ONLY with valid JSON matching this schema:
{
  "is_match": true/false,
  "pii_categories": ["full_name", "home_address", "phone_number", "email_address", "date_of_birth", ...],
  "severity": "low" | "medium" | "high" | "critical",
  "confidence": 0.0-1.0
}

Severity guidelines:
- critical: customer name + address + phone or DOB combined
- high: customer name + address OR name + multiple contact details
- medium: customer name + partial address OR name + email
- low: name only or partial identifiers
"""


@dataclass
class DetectionReport:
    """Outcome of classifying a batch of candidates."""
    confirmed: list[DetectionResult]
    checked: int   # candidates examined
    failed: int    # of those, how many could not be assessed (not the same as "no match")


def _host(url: str) -> str:
    """Host only, for logs: listing URLs embed the customer's name."""
    try:
        return urlparse(url).hostname or "unknown"
    except ValueError:
        return "unknown"


class LLMClassification(BaseModel):
    is_match: bool
    pii_categories: list[str]
    severity: str
    # Bounded so a malformed model answer (e.g. 95) is rejected as a failed check instead of
    # crashing the whole scan later when the DetectionResult is built.
    confidence: float = Field(ge=0.0, le=1.0)
    # No longer requested from the model: the dashboard summary is built locally
    # (shared.pii_guard.build_pii_summary) because the model never sees identity.
    pii_summary: str = ""


def _yes_no(flag: bool) -> str:
    return "yes" if flag else "no"


def _evidence_block(ev: Evidence) -> str:
    return (
        "Customer fields found in the excerpt (computed locally):\n"
        f"- full name: {_yes_no(ev.name_full)}\n"
        f"- surname: {_yes_no(ev.surname)}\n"
        f"- first name: {_yes_no(ev.first_name)}\n"
        f"- address: {_yes_no(ev.address)}\n"
        f"- phone: {_yes_no(ev.phone)}\n"
        f"- email: {_yes_no(ev.email)}\n"
        f"- date of birth: {_yes_no(ev.dob)}\n"
        f"- other people's names on the page: {ev.other_people}"
    )


class DetectionAgent:
    """
    Uses an LLM to classify PII exposures from discovery candidates.
    The LLM sees only redacted page text — never customer PII (see module docstring).
    """

    def __init__(self) -> None:
        self.cfg = get_config()
        cfg = self.cfg
        if cfg.default_llm_provider == "anthropic" and cfg.anthropic_api_key:
            self._llm = ChatAnthropic(
                model=self.cfg.default_llm_model,
                api_key=cfg.anthropic_api_key,
                max_tokens=512,
            )
        else:
            self._llm = ChatOpenAI(
                model=cfg.default_llm_model,
                api_key=cfg.openai_api_key,
                max_tokens=512,
                temperature=0,
            )
        self._ner = PersonDetector(cfg.pii_ner_model)

    def _redact(self, snippet: str, identity: IdentityContext):
        """
        Redaction for one call. Runs in a worker thread (NER is CPU-bound). Raises
        NerUnavailableError, so the call is never made, if NER is required but missing.
        """
        if self.cfg.pii_ner_required:
            self._ner.ensure_loaded()
            detector = self._ner
        else:
            detector = self._ner if self._ner.available else None
        return redact_for_llm(snippet, identity, person_detector=detector)

    async def _run_classifier(
        self,
        candidate: DiscoveryResult,
        identity: IdentityContext,
        max_chars: int = 1500,
    ) -> tuple[LLMClassification, Evidence]:
        """
        Redact, verify, then call the LLM once. This is the only LLM call site.

        Raises OutboundPiiError (before any network request) if the outbound payload
        still contains a customer value, and raises on any LLM/transport failure so
        callers can decide whether a failure means "no match" (discovery) or "could
        not check" (monitoring).
        """
        redacted = await asyncio.to_thread(
            self._redact, candidate.raw_text_snippet[:max_chars], identity
        )

        # Deliberately no URL: profile URLs commonly embed the person's name.
        user_message = (
            f"Data broker source: {candidate.source_name}\n\n"
            f"{_evidence_block(redacted.evidence)}\n\n"
            f"Redacted page excerpt:\n{redacted.text}\n\n"
            "Analyze and respond with JSON only."
        )

        # Fail closed: nothing leaves the process unless everything derived from
        # outside data (source name + redacted excerpt) is free of customer values.
        # The prompt scaffolding and the evidence block are constants/booleans, so they
        # are excluded: a customer surnamed "Page" must not be blocked by the word
        # "page" in our own prompt.
        assert_clean(f"{candidate.source_name}\n{redacted.text}", identity)
        # ...and nothing outside the structural vocabulary (third-party text) remains.
        assert_vocabulary_only(redacted.text)

        structured_llm = self._llm.with_structured_output(LLMClassification)
        classification = await structured_llm.ainvoke([
            SystemMessage(content=DETECTION_SYSTEM_PROMPT),
            HumanMessage(content=user_message),
        ])
        return classification, redacted.evidence

    async def assess_page(
        self,
        candidate: DiscoveryResult,
        identity: IdentityContext,
        max_chars: int = 3000,
    ) -> LLMClassification:
        """
        Strict variant of classify() for monitoring. Returns the raw LLM verdict
        and RAISES if the LLM call fails or is blocked by the PII guard, so an
        outage is never mistaken for "the record is gone".
        """
        classification, _ = await self._run_classifier(candidate, identity, max_chars=max_chars)
        log.info(
            "detection_assessed",
            source=candidate.source_name,
            is_match=classification.is_match,
            confidence=classification.confidence,
        )
        return classification

    async def classify_strict(
        self,
        candidate: DiscoveryResult,
        identity: IdentityContext,
    ) -> Optional[DetectionResult]:
        """
        Classify one candidate. Returns a DetectionResult for a confirmed exposure and None
        for a page that was checked and is not a match.

        RAISES if the page could not be checked (LLM outage, bad credentials, rate limit,
        malformed model output, or the PII guard blocking the call). "Not a match" and
        "could not check" must never look the same: the first is a clean result, the
        second is a gap in the scan.
        """
        classification, evidence = await self._run_classifier(candidate, identity)
        log.info("detection_classified", source=candidate.source_name, is_match=classification.is_match)

        if not classification.is_match or classification.confidence < 0.5:
            return None

        # Map string categories to enum values, ignore unknowns
        categories = []
        for cat in classification.pii_categories:
            try:
                categories.append(PiiCategory(cat))
            except ValueError:
                pass

        if not categories:
            categories = [PiiCategory.OTHER]

        # Use highest-priority category for the record
        primary_category = categories[0]

        try:
            severity = ExposureSeverity(classification.severity)
        except ValueError:
            severity = ExposureSeverity.MEDIUM

        return DetectionResult(
            source_name=candidate.source_name,
            source_url=candidate.source_url,
            pii_category=primary_category,
            severity=severity,
            # Built locally from match evidence, masked; the model never wrote it.
            pii_summary=build_pii_summary(identity, evidence),
            confidence=round(classification.confidence, 2),
        )

    async def classify(
        self,
        candidate: DiscoveryResult,
        identity: IdentityContext,
    ) -> Optional[DetectionResult]:
        """
        Lenient variant of classify_strict(): a page that could not be checked yields None,
        exactly like a page that is not a match. Use only where that distinction does not
        matter; scans use run_report(), which counts the failures.
        """
        try:
            return await self.classify_strict(candidate, identity)
        except Exception as exc:
            # Type only: exception text from HTTP/LLM libraries can embed URLs or request
            # bodies, and URLs embed the customer's name.
            log.error(
                "detection_llm_error",
                host=_host(candidate.source_url),
                error_type=type(exc).__name__,
            )
            return None

    async def run_report(
        self,
        candidates: list[DiscoveryResult],
        identity: IdentityContext,
    ) -> DetectionReport:
        """
        Classify all discovery candidates and say how many could not be checked, so the
        caller never reports "no exposures" for pages that were never actually assessed.
        """
        confirmed: list[DetectionResult] = []
        failed = 0
        for candidate in candidates:
            try:
                result = await self.classify_strict(candidate, identity)
            except Exception as exc:
                failed += 1
                log.error(
                    "detection_llm_error",
                    host=_host(candidate.source_url),
                    error_type=type(exc).__name__,
                )
                continue
            if result:
                confirmed.append(result)
                log.info(
                    "exposure_confirmed",
                    source=result.source_name,
                    category=result.pii_category.value,
                    severity=result.severity.value,
                )
        return DetectionReport(confirmed=confirmed, checked=len(candidates), failed=failed)

    async def run(
        self,
        candidates: list[DiscoveryResult],
        identity: IdentityContext,
    ) -> list[DetectionResult]:
        """Classify all discovery candidates. Returns confirmed exposures only."""
        return (await self.run_report(candidates, identity)).confirmed
