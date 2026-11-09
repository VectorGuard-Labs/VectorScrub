"""
Spokeo data broker detector.
"""

from typing import Dict, List, Optional

from .base import DataBrokerDetector, DetectionResult, RemovalDifficulty
from .registry import register_detector


@register_detector
class SpokeoDetector(DataBrokerDetector):
    """Detector for Spokeo.com."""

    def get_broker_name(self) -> str:
        return "Spokeo"

    def get_broker_domain(self) -> str:
        return "spokeo.com"

    def get_removal_info(self) -> Dict:
        return {
            "removal_url": "https://www.spokeo.com/optout",
            "removal_difficulty": RemovalDifficulty.EASY,
            "removal_instructions": (
                "1. Visit the Spokeo opt-out page\n"
                "2. Search for your listing\n"
                "3. Click 'Remove this record'\n"
                "4. Verify via email\n"
                "5. Removal typically completes within 72 hours"
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
        Search Spokeo for a person.

        Note: Spokeo may require authentication for full results.
        This implementation uses public search endpoints.
        """
        import httpx
        import structlog
        from bs4 import BeautifulSoup

        log = structlog.get_logger()
        results = []

        try:
            # Construct search URL
            name_parts = full_name.split(maxsplit=1)
            if len(name_parts) < 2:
                log.warning("spokeo_invalid_name", name=full_name)
                return results

            first_name, last_name = name_parts[0], name_parts[1]

            # Build search query
            query_parts = [f"{first_name} {last_name}"]
            if city and state:
                query_parts.append(f"{city}, {state}")
            elif state:
                query_parts.append(state)

            search_url = "https://www.spokeo.com/search"
            params = {"q": " ".join(query_parts)}

            async with httpx.AsyncClient(timeout=30.0, follow_redirects=True) as client:
                headers = {
                    "User-Agent": "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36"
                }
                response = await client.get(search_url, params=params, headers=headers)

                if response.status_code != 200:
                    log.warning(
                        "spokeo_search_failed",
                        status_code=response.status_code,
                        name=full_name,
                    )
                    return results

                soup = BeautifulSoup(response.text, "html.parser")

                # Parse search results
                result_cards = soup.find_all("div", class_=lambda x: x and "result" in x.lower())

                for card in result_cards[:5]:  # Limit to top 5 results
                    detected_info = {}

                    # Extract name
                    name_elem = card.find(["h2", "h3", "div"], class_=lambda x: x and "name" in x.lower())
                    if name_elem:
                        detected_info["full_name"] = name_elem.get_text(strip=True)

                    # Extract location
                    location_elem = card.find(class_=lambda x: x and ("location" in x.lower() or "address" in x.lower()))
                    if location_elem:
                        location_text = location_elem.get_text(strip=True)
                        detected_info["location"] = location_text

                        # Try to parse city/state
                        if "," in location_text:
                            parts = [p.strip() for p in location_text.split(",")]
                            if len(parts) >= 2:
                                detected_info["city"] = parts[0]
                                detected_info["state"] = parts[1]

                    # Extract age if available
                    age_elem = card.find(class_=lambda x: x and "age" in x.lower())
                    if age_elem:
                        age_text = age_elem.get_text(strip=True)
                        # Extract number from text like "Age 45"
                        import re
                        age_match = re.search(r"\d+", age_text)
                        if age_match:
                            detected_info["age"] = int(age_match.group())

                    # Extract profile URL
                    link_elem = card.find("a", href=True)
                    source_url = "https://www.spokeo.com"
                    if link_elem:
                        href = link_elem["href"]
                        if href.startswith("/"):
                            source_url = f"https://www.spokeo.com{href}"
                        else:
                            source_url = href

                    if detected_info:
                        result = DetectionResult(
                            source_name=self.get_broker_name(),
                            source_url=source_url,
                            detected_info=detected_info,
                            confidence=0.7,  # Medium confidence for parsed results
                            removal_info=self.get_removal_info(),
                        )
                        results.append(result)

                log.info(
                    "spokeo_search_complete",
                    name=full_name,
                    results_found=len(results),
                )

        except Exception as e:
            log.error(
                "spokeo_search_error",
                name=full_name,
                error=str(e),
            )

        return results
