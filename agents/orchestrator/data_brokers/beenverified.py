"""
BeenVerified data broker detector.
"""

from typing import Dict, List, Optional

from .base import DataBrokerDetector, DetectionResult, RemovalDifficulty
from .registry import register_detector


@register_detector
class BeenVerifiedDetector(DataBrokerDetector):
    """Detector for BeenVerified.com."""

    def get_broker_name(self) -> str:
        return "BeenVerified"

    def get_broker_domain(self) -> str:
        return "beenverified.com"

    def get_removal_info(self) -> Dict:
        return {
            "removal_url": "https://www.beenverified.com/app/optout/search",
            "removal_difficulty": RemovalDifficulty.MEDIUM,
            "removal_instructions": (
                "1. Visit the BeenVerified opt-out page\n"
                "2. Search for your listing using your name and state\n"
                "3. Select your record from search results\n"
                "4. Enter your email address\n"
                "5. Check your email and click the confirmation link\n"
                "6. Wait up to 72 hours for removal"
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
        Search BeenVerified for a person.

        Note: BeenVerified may require authentication for full results.
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
                log.warning("beenverified_invalid_name", name=full_name)
                return results

            first_name, last_name = name_parts[0], name_parts[1]

            # BeenVerified search format
            search_url = "https://www.beenverified.com/people"
            params = {
                "firstName": first_name,
                "lastName": last_name,
            }

            if state:
                params["state"] = state
            if city:
                params["city"] = city

            async with httpx.AsyncClient(timeout=30.0, follow_redirects=True) as client:
                headers = {
                    "User-Agent": "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36"
                }
                response = await client.get(search_url, params=params, headers=headers)

                if response.status_code != 200:
                    log.warning(
                        "beenverified_search_failed",
                        status_code=response.status_code,
                        name=full_name,
                    )
                    return results

                soup = BeautifulSoup(response.text, "html.parser")

                # Parse search results
                result_items = soup.find_all(["div", "article"], class_=lambda x: x and ("person" in x.lower() or "result" in x.lower()))

                for item in result_items[:5]:  # Limit to top 5 results
                    detected_info = {}

                    # Extract name
                    name_elem = item.find(["h2", "h3", "span"], class_=lambda x: x and "name" in x.lower())
                    if name_elem:
                        detected_info["full_name"] = name_elem.get_text(strip=True)

                    # Extract location/address
                    addr_elem = item.find(class_=lambda x: x and ("address" in x.lower() or "location" in x.lower()))
                    if addr_elem:
                        location_text = addr_elem.get_text(strip=True)
                        detected_info["location"] = location_text

                        # Parse city/state
                        if "," in location_text:
                            parts = [p.strip() for p in location_text.split(",")]
                            if len(parts) >= 2:
                                detected_info["city"] = parts[0]
                                # State is often last part
                                state_part = parts[-1]
                                # Extract state code (2 letters)
                                import re
                                state_match = re.search(r"\b([A-Z]{2})\b", state_part)
                                if state_match:
                                    detected_info["state"] = state_match.group(1)

                    # Extract age
                    age_elem = item.find(class_=lambda x: x and "age" in x.lower())
                    if age_elem:
                        age_text = age_elem.get_text(strip=True)
                        import re
                        age_match = re.search(r"\d+", age_text)
                        if age_match:
                            detected_info["age"] = int(age_match.group())

                    # Extract relatives if available
                    relatives_elem = item.find(class_=lambda x: x and "relative" in x.lower())
                    if relatives_elem:
                        detected_info["relatives"] = relatives_elem.get_text(strip=True)

                    # Extract profile URL
                    link_elem = item.find("a", href=True)
                    source_url = "https://www.beenverified.com"
                    if link_elem:
                        href = link_elem["href"]
                        if href.startswith("/"):
                            source_url = f"https://www.beenverified.com{href}"
                        else:
                            source_url = href

                    if detected_info:
                        result = DetectionResult(
                            source_name=self.get_broker_name(),
                            source_url=source_url,
                            detected_info=detected_info,
                            confidence=0.65,  # Medium confidence
                            removal_info=self.get_removal_info(),
                        )
                        results.append(result)

                log.info(
                    "beenverified_search_complete",
                    name=full_name,
                    results_found=len(results),
                )

        except Exception as e:
            log.error(
                "beenverified_search_error",
                name=full_name,
                error=str(e),
            )

        return results
