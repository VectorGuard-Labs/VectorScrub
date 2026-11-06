"""
Detection Agent — Phase 1
Receives candidate URLs from the Discovery Agent and uses an LLM to
classify and confirm actual PII exposures.

Safety principle: The LLM receives only the page text snippet and a
minimal structured prompt — never the customer's raw PII. Matching is
done by comparing anonymized tokens, not by sending identity data to
the model.
"""
import re
from typing import Optional

import structlog
from langchain_openai import ChatOpenAI
from langchain_anthropic import ChatAnthropic
from langchain_core.messages import HumanMessage, SystemMessage
from pydantic import BaseModel

from shared.config import get_config
from shared.models import (
    DetectionResult,
    DiscoveryResult,
    ExposureSeverity,
    IdentityContext,
    PiiCategory,
)

log = structlog.get_logger(__name__)

DETECTION_SYSTEM_PROMPT = """You are a PII detection specialist for a privacy protection service.
You are given a snippet of text from a data broker website and a set of anonymized identity tokens.
Your task is to determine:
1. Whether the text contains personal information matching the identity tokens.
2. What categories of PII are present.
3. The severity of the exposure.

Identity tokens are anonymized (e.g., first letters only). You must reason about whether the
text likely refers to the same person WITHOUT being given their full identity.

Respond ONLY with valid JSON matching this schema:
{
  "is_match": true/false,
  "pii_categories": ["full_name", "home_address", ...],
  "severity": "low" | "medium" | "high" | "critical",
  "confidence": 0.0-1.0,
  "pii_summary": "short masked description e.g. 'J*** D***, 1** M*** St'"
}

Severity guidelines:
- critical: full name + full address + phone or DOB combined
- high: full name + address OR full name + multiple contact details
- medium: full name + partial address OR name + email
- low: name only or partial identifiers
"""


class LLMClassification(BaseModel):
    is_match: bool
    pii_categories: list[str]
    severity: str
    confidence: float
    pii_summary: str


class DetectionAgent:
    """
    Uses an LLM to classify PII exposures from discovery candidates.
    Sends ONLY anonymized tokens to the model — never raw customer PII.
    """

    def __init__(self) -> None:
        cfg = get_config()
        if cfg.default_llm_provider == "anthropic" and cfg.anthropic_api_key:
            self._llm = ChatAnthropic(
                model="claude-3-5-haiku-20241022",
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

    def _anonymize_identity(self, identity: IdentityContext) -> dict:
        """
        Convert full PII to anonymized tokens safe to send to the LLM.
        e.g. "John Smith" → "J*** S***", "john@example.com" → "j***@e***.com"
        """
        def mask_name(name: str) -> str:
            return " ".join(p[0] + "***" if p else "***" for p in name.split())

        def mask_email(email: str) -> str:
            if "@" in email:
                local, domain = email.split("@", 1)
                domain_parts = domain.split(".")
                return f"{local[0]}***@{domain_parts[0][0]}***.{domain_parts[-1]}"
            return "***"

        def mask_phone(phone: str) -> str:
            digits = re.sub(r"\D", "", phone)
            return f"***-***-{digits[-4:]}" if len(digits) >= 4 else "***"

        def mask_address(addr: str) -> str:
            parts = addr.split(",")
            if parts:
                street_parts = parts[0].split()
                masked = " ".join(p[0] + "***" if i > 0 else p for i, p in enumerate(street_parts))
                return masked + ("," + ",".join(parts[1:]) if len(parts) > 1 else "")
            return "***"

        return {
            "name_token": mask_name(identity.full_name) if identity.full_name else None,
            "address_token": mask_address(identity.current_address) if identity.current_address else None,
            "email_tokens": [mask_email(e) for e in identity.email_addresses],
            "phone_tokens": [mask_phone(p) for p in identity.phone_numbers],
        }

    async def classify(
        self,
        candidate: DiscoveryResult,
        identity: IdentityContext,
    ) -> Optional[DetectionResult]:
        """
        Run LLM classification on a discovery candidate.
        Returns a DetectionResult if confirmed exposure, None otherwise.
        """
        tokens = self._anonymize_identity(identity)
        user_message = f"""
Data broker source: {candidate.source_name}
URL: {candidate.source_url}

Page snippet:
{candidate.raw_text_snippet[:1500]}

Anonymized identity tokens to match against:
{tokens}

Analyze and respond with JSON only.
"""
        try:
            response = await self._llm.ainvoke([
                SystemMessage(content=DETECTION_SYSTEM_PROMPT),
                HumanMessage(content=user_message),
            ])
            raw = response.content.strip()
            # Strip markdown code fences if present
            if raw.startswith("```"):
                raw = re.sub(r"^```[a-z]*\n?", "", raw)
                raw = raw.rstrip("```").strip()

            import json
            data = json.loads(raw)
            classification = LLMClassification(**data)

        except Exception as exc:
            log.error("detection_llm_error", source=candidate.source_name, error=str(exc))
            return None

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
            pii_summary=classification.pii_summary[:500],
            confidence=round(classification.confidence, 2),
        )

    async def run(
        self,
        candidates: list[DiscoveryResult],
        identity: IdentityContext,
    ) -> list[DetectionResult]:
        """
        Classify all discovery candidates. Returns confirmed exposures.
        """
        confirmed: list[DetectionResult] = []
        for candidate in candidates:
            result = await self.classify(candidate, identity)
            if result:
                confirmed.append(result)
                log.info(
                    "exposure_confirmed",
                    source=result.source_name,
                    category=result.pii_category.value,
                    severity=result.severity.value,
                )
        return confirmed
