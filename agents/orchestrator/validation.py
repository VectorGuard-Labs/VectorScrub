"""
Detection validation framework for VectorScrub.

This module provides validation and confidence scoring for PII detections
to reduce false positives and improve accuracy.
"""

import re
from dataclasses import dataclass
from enum import Enum
from typing import Dict, List, Optional


class PIIType(Enum):
    """Types of PII that can be detected."""

    FULL_NAME = "full_name"
    EMAIL = "email"
    PHONE = "phone"
    ADDRESS = "address"
    DATE_OF_BIRTH = "date_of_birth"
    SSN = "ssn"
    CREDIT_CARD = "credit_card"
    UNKNOWN = "unknown"


@dataclass
class ValidationResult:
    """Result of validation check."""

    is_valid: bool
    confidence: float  # 0.0 to 1.0
    pii_type: PIIType
    match_quality: str  # "exact", "partial", "fuzzy", "likely_false_positive"
    evidence: List[str]
    warnings: List[str]


class PIIValidator:
    """
    Validates detected PII to reduce false positives.

    Uses pattern matching, heuristics, and context analysis to score
    the confidence of a detection.
    """

    # Common false positive patterns
    FALSE_POSITIVE_PATTERNS = {
        "full_name": [
            r"^(admin|user|test|demo|example|sample|lorem ipsum)$",
            r"^(john doe|jane doe|joe smith|mary smith)$",
            r"^(foo bar|test user|placeholder)$",
        ],
        "email": [
            r"^(test|demo|example|noreply|no-reply|admin)@",
            r"@(example\.com|test\.com|localhost|invalid)$",
            r"^(user\d+|test\d+|admin\d+)@",
        ],
        "phone": [
            r"^(000|111|222|333|444|555|666|777|888|999)-?\d{3}-?\d{4}$",
            r"^1234567890$",
            r"^(123|555)-?(123|555)-?(1234|0000)$",
        ],
        "address": [
            r"^(123 main st|456 elm st|789 oak ave)",
            r"test (street|st|avenue|ave|road|rd)",
            r"example (city|town|village)",
        ],
    }

    # High-confidence patterns
    HIGH_CONFIDENCE_PATTERNS = {
        "email": r"^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}$",
        "phone": r"^\+?1?[-.\s]?\(?\d{3}\)?[-.\s]?\d{3}[-.\s]?\d{4}$",
        "ssn": r"^\d{3}-?\d{2}-?\d{4}$",
        "credit_card": r"^\d{4}[-\s]?\d{4}[-\s]?\d{4}[-\s]?\d{4}$",
    }

    def __init__(self, identity_context: Optional[Dict] = None):
        """
        Initialize validator with identity context.

        Args:
            identity_context: Customer's identity profile for cross-checking
        """
        self.identity_context = identity_context or {}

    def validate_detection(
        self,
        detected_value: str,
        pii_type: str,
        source_context: str = "",
    ) -> ValidationResult:
        """
        Validate a PII detection.

        Args:
            detected_value: The detected PII value
            pii_type: Type of PII (full_name, email, phone, etc.)
            source_context: Surrounding text/context from source

        Returns:
            ValidationResult with confidence score and validity
        """
        pii_enum = self._parse_pii_type(pii_type)
        evidence = []
        warnings = []
        confidence = 0.5  # Start at 50%

        # Check for false positive patterns
        if self._is_false_positive(detected_value, pii_type):
            warnings.append("Matches known false positive pattern")
            confidence -= 0.3
            return ValidationResult(
                is_valid=False,
                confidence=max(0.0, confidence),
                pii_type=pii_enum,
                match_quality="likely_false_positive",
                evidence=evidence,
                warnings=warnings,
            )

        # Check format validity
        if pii_type in self.HIGH_CONFIDENCE_PATTERNS:
            pattern = self.HIGH_CONFIDENCE_PATTERNS[pii_type]
            if re.match(pattern, detected_value, re.IGNORECASE):
                evidence.append("Matches high-confidence format pattern")
                confidence += 0.2
            else:
                warnings.append("Does not match expected format")
                confidence -= 0.2

        # Cross-check with identity context
        match_quality = self._cross_check_identity(detected_value, pii_type)
        if match_quality == "exact":
            evidence.append("Exact match with identity profile")
            confidence += 0.3
        elif match_quality == "partial":
            evidence.append("Partial match with identity profile")
            confidence += 0.1
        elif match_quality == "fuzzy":
            evidence.append("Fuzzy match with identity profile")
            confidence += 0.05

        # Analyze source context
        context_score = self._analyze_context(source_context, pii_type)
        confidence += context_score
        if context_score > 0:
            evidence.append(f"Contextual evidence score: +{context_score:.2f}")
        elif context_score < 0:
            warnings.append(f"Weak context score: {context_score:.2f}")

        # Ensure confidence is in valid range
        confidence = max(0.0, min(1.0, confidence))

        # Determine validity (confidence threshold: 0.6)
        is_valid = confidence >= 0.6

        return ValidationResult(
            is_valid=is_valid,
            confidence=confidence,
            pii_type=pii_enum,
            match_quality=match_quality,
            evidence=evidence,
            warnings=warnings,
        )

    def _parse_pii_type(self, pii_type: str) -> PIIType:
        """Parse PII type string to enum."""
        try:
            return PIIType(pii_type.lower())
        except ValueError:
            return PIIType.UNKNOWN

    def _is_false_positive(self, value: str, pii_type: str) -> bool:
        """Check if value matches known false positive patterns."""
        if pii_type not in self.FALSE_POSITIVE_PATTERNS:
            return False

        value_lower = value.lower().strip()
        for pattern in self.FALSE_POSITIVE_PATTERNS[pii_type]:
            if re.match(pattern, value_lower, re.IGNORECASE):
                return True

        return False

    def _cross_check_identity(self, value: str, pii_type: str) -> str:
        """
        Cross-check detected value with customer's identity profile.

        Returns:
            "exact", "partial", "fuzzy", or "no_match"
        """
        if not self.identity_context:
            return "no_match"

        value_lower = value.lower().strip()

        # Check for exact matches
        if pii_type == "full_name" and "full_name" in self.identity_context:
            identity_name = self.identity_context["full_name"].lower().strip()
            if value_lower == identity_name:
                return "exact"
            # Check partial (first name or last name match)
            value_parts = set(value_lower.split())
            identity_parts = set(identity_name.split())
            if value_parts & identity_parts:  # Set intersection
                return "partial"

        elif pii_type == "email" and "email_addresses" in self.identity_context:
            identity_emails = [
                e.lower().strip() for e in self.identity_context["email_addresses"]
            ]
            if value_lower in identity_emails:
                return "exact"

        elif pii_type == "phone" and "phone_numbers" in self.identity_context:
            # Normalize phone numbers (remove formatting)
            normalized_value = re.sub(r"[^\d]", "", value)
            for identity_phone in self.identity_context.get("phone_numbers", []):
                normalized_identity = re.sub(r"[^\d]", "", identity_phone)
                if normalized_value == normalized_identity:
                    return "exact"
                # Check if one is substring of other (e.g., with/without country code)
                if (
                    normalized_value in normalized_identity
                    or normalized_identity in normalized_value
                ):
                    return "partial"

        elif pii_type == "address" and "current_address" in self.identity_context:
            identity_address = self.identity_context["current_address"].lower().strip()
            if value_lower in identity_address or identity_address in value_lower:
                return "partial"

        return "no_match"

    def _analyze_context(self, context: str, pii_type: str) -> float:
        """
        Analyze surrounding context to determine confidence boost/penalty.

        Returns:
            Score adjustment (-0.2 to +0.2)
        """
        if not context:
            return 0.0

        context_lower = context.lower()
        score = 0.0

        # Positive indicators
        positive_keywords = {
            "full_name": ["name:", "full name:", "owner:", "resident:", "tenant:"],
            "email": [
                "email:",
                "e-mail:",
                "contact:",
                "reach at:",
                "send to:",
            ],
            "phone": [
                "phone:",
                "call:",
                "tel:",
                "telephone:",
                "mobile:",
                "cell:",
            ],
            "address": [
                "address:",
                "located at:",
                "resides at:",
                "mailing address:",
                "street:",
            ],
        }

        if pii_type in positive_keywords:
            for keyword in positive_keywords[pii_type]:
                if keyword in context_lower:
                    score += 0.1
                    break  # Only count once

        # Negative indicators
        negative_keywords = [
            "example",
            "sample",
            "test",
            "placeholder",
            "lorem ipsum",
            "copyright",
            "privacy policy",
        ]

        for keyword in negative_keywords:
            if keyword in context_lower:
                score -= 0.1
                break

        return min(0.2, max(-0.2, score))


def batch_validate(
    detections: List[Dict],
    identity_context: Optional[Dict] = None,
    min_confidence: float = 0.6,
) -> List[Dict]:
    """
    Batch validate multiple detections and filter by confidence threshold.

    Args:
        detections: List of detection dicts with 'value', 'pii_type', 'context' keys
        identity_context: Customer identity for cross-checking
        min_confidence: Minimum confidence threshold (0.0-1.0)

    Returns:
        Filtered list of valid detections with validation results
    """
    validator = PIIValidator(identity_context)
    validated = []

    for detection in detections:
        result = validator.validate_detection(
            detected_value=detection["value"],
            pii_type=detection["pii_type"],
            source_context=detection.get("context", ""),
        )

        if result.is_valid and result.confidence >= min_confidence:
            validated.append(
                {
                    **detection,
                    "validation": {
                        "confidence": result.confidence,
                        "match_quality": result.match_quality,
                        "evidence": result.evidence,
                        "warnings": result.warnings,
                    },
                }
            )

    return validated
