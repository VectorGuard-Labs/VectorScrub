"""
Whitepages data broker detector.
"""

import re
from typing import Dict, List, Optional

import httpx
from bs4 import BeautifulSoup

from .base import DataBrokerDetector, DetectionResult, RemovalDifficulty
from .registry import register_detector


@register_detector
class WhitepagesDetector(DataBrokerDetector):
    """Detector for Whitepages.com."""

    def get_broker_name(self) -> str:
        return "Whitepages"

    def get_broker_domain(self) -> str:
        return "whitepages.com"

    def get_removal_info(self) -> Dict:
        return {
            "removal_url": "https://www.whitepages.com/suppression-requests",
            "removal_difficulty": RemovalDifficulty.MEDIUM,
            "removal_instructions": (
                "1. Visit the Whitepages suppression request page\n"
                "2. Find your listing using name and location\n"
                "3. Click 'Remove this record'\n"
                "4. Verify your identity via phone or email\n"
                "5. Wait 24-48 hours for removal"
            ),
        }

    async def search(
        self,
        full_name: str,
        city: Optional[str] = None,
        state: Optional[str] = None,
        age: Optional[int] = None,
    ) -> List[DetectionResult]:
        """
        Search Whitepages for a person.

        Note: This is a simplified implementation. In production, you would need:
        - Proper rate limiting
        - Proxy rotation
        - CAPTCHA solving
        - Session management
        """
        results = []

        # Build search URL
        name_parts = full_name.split()
        if len(name_parts) < 2:
            return results

        first_name = name_parts[0]
        last_name = " ".join(name_parts[1:])

        # Construct search query
        params = {
            "firstname": first_name,
            "lastname": last_name,
        }

        if city and state:
            params["where"] = f"{city}, {state}"
        elif state:
            params["where"] = state

        try:
            async with httpx.AsyncClient(timeout=30.0, follow_redirects=True) as client:
                # Search page
                search_url = "https://www.whitepages.com/name"
                response = await client.get(search_url, params=params)

                if response.status_code != 200:
                    return results

                # Parse search results
                soup = BeautifulSoup(response.text, "html.parser")

                # Find person listings (simplified - actual selectors vary)
                person_cards = soup.find_all("div", class_=re.compile(r"person-card|result"))

                for card in person_cards[:5]:  # Limit to first 5 results
                    detected_info = self._parse_person_card(card)

                    if detected_info:
                        # Get profile URL
                        profile_link = card.find("a", href=re.compile(r"/name/"))
                        profile_url = (
                            f"https://www.whitepages.com{profile_link['href']}"
                            if profile_link
                            else f"https://www.whitepages.com/name/{first_name}-{last_name}"
                        )

                        # Calculate confidence (simplified)
                        confidence = 0.7  # Base confidence for Whitepages match

                        result = self.create_detection_result(
                            profile_url=profile_url,
                            detected_info=detected_info,
                            confidence=confidence,
                        )

                        results.append(result)

        except Exception as e:
            # Log error but don't fail the entire scan
            import structlog

            log = structlog.get_logger()
            log.error("whitepages_search_failed", error=str(e))

        return results

    def _parse_person_card(self, card_element) -> Optional[Dict]:
        """Parse a person card element to extract information."""
        try:
            info = {}

            # Extract name
            name_elem = card_element.find(class_=re.compile(r"name|person-name"))
            if name_elem:
                info["name"] = name_elem.get_text(strip=True)

            # Extract age
            age_elem = card_element.find(class_=re.compile(r"age"))
            if age_elem:
                age_text = age_elem.get_text(strip=True)
                age_match = re.search(r"\d+", age_text)
                if age_match:
                    info["age"] = age_match.group()

            # Extract location
            location_elem = card_element.find(class_=re.compile(r"location|address"))
            if location_elem:
                location_text = location_elem.get_text(strip=True)
                # Try to parse "City, State"
                location_parts = location_text.split(",")
                if len(location_parts) >= 2:
                    info["city"] = location_parts[0].strip()
                    info["state"] = location_parts[1].strip()

            # Extract phone (if visible)
            phone_elem = card_element.find(class_=re.compile(r"phone"))
            if phone_elem:
                info["phone"] = phone_elem.get_text(strip=True)

            return info if info else None

        except Exception:
            return None
