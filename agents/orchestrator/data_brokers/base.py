"""
Base class for data broker detectors.
"""

from abc import ABC, abstractmethod
from dataclasses import dataclass
from enum import Enum
from typing import Dict, List, Optional


class RemovalDifficulty(Enum):
    """Difficulty level for removal from a data broker."""

    EASY = "easy"  # Online opt-out form
    MEDIUM = "medium"  # Email or phone verification required
    HARD = "hard"  # Physical mail or notarization required
    VERY_HARD = "very_hard"  # Legal action or multiple steps required


@dataclass
class DetectionResult:
    """Result of a data broker detection."""

    broker_name: str
    broker_url: str
    profile_url: str
    detected_pii: Dict[str, str]  # e.g., {"name": "John Doe", "age": "35"}
    confidence: float  # 0.0 to 1.0
    removal_url: Optional[str] = None
    removal_difficulty: RemovalDifficulty = RemovalDifficulty.MEDIUM
    additional_info: Dict = None

    def __post_init__(self):
        if self.additional_info is None:
            self.additional_info = {}


class DataBrokerDetector(ABC):
    """
    Abstract base class for data broker detectors.

    Each detector implements search and detection logic for a specific
    data broker service.
    """

    def __init__(self):
        self.broker_name = self.get_broker_name()
        self.broker_domain = self.get_broker_domain()
        self.removal_info = self.get_removal_info()

    @abstractmethod
    def get_broker_name(self) -> str:
        """Return the name of the data broker (e.g., 'Whitepages')."""
        pass

    @abstractmethod
    def get_broker_domain(self) -> str:
        """Return the primary domain of the broker (e.g., 'whitepages.com')."""
        pass

    @abstractmethod
    def get_removal_info(self) -> Dict:
        """
        Return removal information for this broker.

        Returns:
            Dict with keys:
                - removal_url: URL to removal/opt-out page
                - removal_difficulty: RemovalDifficulty enum value
                - removal_instructions: Human-readable instructions
        """
        pass

    @abstractmethod
    async def search(
        self,
        full_name: str,
        city: Optional[str] = None,
        state: Optional[str] = None,
        age: Optional[int] = None,
    ) -> List[DetectionResult]:
        """
        Search for a person on this data broker.

        Args:
            full_name: Person's full name
            city: City (optional)
            state: State abbreviation (optional)
            age: Age (optional)

        Returns:
            List of detection results
        """
        pass

    def is_match(self, detected_info: Dict, identity_context: Dict) -> float:
        """
        Calculate match confidence between detected info and identity context.

        Args:
            detected_info: Information found on broker
            identity_context: Customer's identity profile

        Returns:
            Confidence score 0.0 to 1.0
        """
        confidence = 0.0
        matches = 0
        total_checks = 0

        # Check name match
        if "name" in detected_info and "full_name" in identity_context:
            total_checks += 1
            detected_name = detected_info["name"].lower().strip()
            identity_name = identity_context["full_name"].lower().strip()

            if detected_name == identity_name:
                confidence += 0.3
                matches += 1
            elif self._partial_name_match(detected_name, identity_name):
                confidence += 0.15
                matches += 0.5

        # Check age match (within 2 years)
        if "age" in detected_info and "date_of_birth" in identity_context:
            total_checks += 1
            try:
                detected_age = int(detected_info["age"])
                # Calculate age from DOB
                from datetime import datetime

                dob = datetime.fromisoformat(identity_context["date_of_birth"])
                actual_age = (datetime.now() - dob).days // 365

                if abs(detected_age - actual_age) <= 2:
                    confidence += 0.2
                    matches += 1
            except (ValueError, KeyError):
                pass

        # Check location match
        if "city" in detected_info and "state" in detected_info:
            total_checks += 1
            if "current_address" in identity_context:
                address_lower = identity_context["current_address"].lower()
                city_match = detected_info["city"].lower() in address_lower
                state_match = detected_info["state"].lower() in address_lower

                if city_match and state_match:
                    confidence += 0.3
                    matches += 1
                elif city_match or state_match:
                    confidence += 0.15
                    matches += 0.5

        # Check phone match
        if "phone" in detected_info and "phone_numbers" in identity_context:
            total_checks += 1
            detected_phone = self._normalize_phone(detected_info["phone"])
            for identity_phone in identity_context.get("phone_numbers", []):
                if detected_phone == self._normalize_phone(identity_phone):
                    confidence += 0.2
                    matches += 1
                    break

        # If we have matches, boost confidence
        if matches > 0 and total_checks > 0:
            match_ratio = matches / total_checks
            confidence += match_ratio * 0.2

        return min(1.0, confidence)

    def _partial_name_match(self, name1: str, name2: str) -> bool:
        """Check if two names have partial overlap (first or last name)."""
        parts1 = set(name1.split())
        parts2 = set(name2.split())
        return bool(parts1 & parts2)  # Set intersection

    def _normalize_phone(self, phone: str) -> str:
        """Normalize phone number (remove formatting)."""
        import re

        return re.sub(r"[^\d]", "", phone)

    def create_detection_result(
        self, profile_url: str, detected_info: Dict, confidence: float
    ) -> DetectionResult:
        """
        Helper to create a DetectionResult with broker info pre-filled.

        Args:
            profile_url: URL to the detected profile
            detected_info: Dictionary of detected PII
            confidence: Confidence score

        Returns:
            DetectionResult instance
        """
        removal_info = self.get_removal_info()

        return DetectionResult(
            broker_name=self.broker_name,
            broker_url=f"https://{self.broker_domain}",
            profile_url=profile_url,
            detected_pii=detected_info,
            confidence=confidence,
            removal_url=removal_info.get("removal_url"),
            removal_difficulty=removal_info.get(
                "removal_difficulty", RemovalDifficulty.MEDIUM
            ),
            additional_info={"removal_instructions": removal_info.get("removal_instructions", "")},
        )
