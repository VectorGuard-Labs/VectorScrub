"""
TruthFinder data broker detector.
"""

from typing import Dict, List, Optional

from .base import DataBrokerDetector, DetectionResult, RemovalDifficulty
from .registry import register_detector


@register_detector
class TruthFinderDetector(DataBrokerDetector):
    """Detector for TruthFinder.com."""

    def get_broker_name(self) -> str:
        return "TruthFinder"

    def get_broker_domain(self) -> str:
        return "truthfinder.com"

    def get_removal_info(self) -> Dict:
        return {
            "removal_url": "https://www.truthfinder.com/opt-out/",
            "removal_difficulty": RemovalDifficulty.HARD,
            "removal_instructions": (
                "1. Visit the TruthFinder opt-out page\n"
                "2. Search for your record\n"
                "3. Submit opt-out request with record URL\n"
                "4. Verify your identity (may require government ID)\n"
                "5. Wait 5-7 business days for removal\n"
                "Note: TruthFinder has a more complex removal process"
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
        Search TruthFinder for a person.

        Note: TruthFinder requires authentication for full results.
        This implementation uses public search endpoints.
        """
        import httpx
        import structlog
        from bs4 import BeautifulSoup

        log = structlog.get_logger()
        results = []

        try:
            # Construct search parameters
            name_parts = full_name.split(maxsplit=1)
            if len(name_parts) < 2:
                log.warning("truthfinder_invalid_name", name=full_name)
                return results

            first_name, last_name = name_parts[0], name_parts[1]

            # TruthFinder search endpoint
            search_url = "https://www.truthfinder.com/results"
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
                        "truthfinder_search_failed",
                        status_code=response.status_code,
                        name=full_name,
                    )
                    return results

                soup = BeautifulSoup(response.text, "html.parser")

                # Parse results - TruthFinder typically has summary cards
                result_cards = soup.find_all("div", class_=lambda x: x and ("record" in x.lower() or "person" in x.lower()))

                for card in result_cards[:5]:  # Limit to top 5 results
                    detected_info = {}

                    # Extract name
                    name_elem = card.find(["h2", "h3", "div"], class_=lambda x: x and "name" in x.lower())
                    if name_elem:
                        detected_info["full_name"] = name_elem.get_text(strip=True)

                    # Extract location
                    location_elem = card.find(class_=lambda x: x and ("city" in x.lower() or "location" in x.lower()))
                    if location_elem:
                        location_text = location_elem.get_text(strip=True)
                        detected_info["location"] = location_text

                        # Parse city/state
                        if "," in location_text:
                            parts = [p.strip() for p in location_text.split(",")]
                            if len(parts) >= 2:
                                detected_info["city"] = parts[0]
                                import re
                                state_match = re.search(r"\b([A-Z]{2})\b", parts[-1])
                                if state_match:
                                    detected_info["state"] = state_match.group(1)

                    # Extract age
                    age_elem = card.find(class_=lambda x: x and "age" in x.lower())
                    if age_elem:
                        age_text = age_elem.get_text(strip=True)
                        import re
                        age_match = re.search(r"\d+", age_text)
                        if age_match:
                            detected_info["age"] = int(age_match.group())

                    # Extract phone numbers if available
                    phone_elem = card.find(class_=lambda x: x and "phone" in x.lower())
                    if phone_elem:
                        detected_info["phone"] = phone_elem.get_text(strip=True)

                    # Extract profile URL
                    link_elem = card.find("a", href=True)
                    source_url = "https://www.truthfinder.com"
                    if link_elem:
                        href = link_elem["href"]
                        if href.startswith("/"):
                            source_url = f"https://www.truthfinder.com{href}"
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
                    "truthfinder_search_complete",
                    name=full_name,
                    results_found=len(results),
                )

        except Exception as e:
            log.error(
                "truthfinder_search_error",
                name=full_name,
                error=str(e),
            )

        return results


@register_detector
class InteliusDetector(DataBrokerDetector):
    """Detector for Intelius.com."""

    def get_broker_name(self) -> str:
        return "Intelius"

    def get_broker_domain(self) -> str:
        return "intelius.com"

    def get_removal_info(self) -> Dict:
        return {
            "removal_url": "https://www.intelius.com/optout",
            "removal_difficulty": RemovalDifficulty.MEDIUM,
            "removal_instructions": (
                "1. Visit the Intelius opt-out page\n"
                "2. Search for your listing\n"
                "3. Select your record and click 'Opt-Out'\n"
                "4. Enter your email for verification\n"
                "5. Click confirmation link in email\n"
                "6. Wait 72 hours for removal"
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
        Search Intelius for a person.

        Note: Intelius may require authentication for full results.
        This implementation uses public search endpoints.
        """
        import httpx
        import structlog
        from bs4 import BeautifulSoup

        log = structlog.get_logger()
        results = []

        try:
            # Construct search query
            name_parts = full_name.split(maxsplit=1)
            if len(name_parts) < 2:
                log.warning("intelius_invalid_name", name=full_name)
                return results

            first_name, last_name = name_parts[0], name_parts[1]

            # Intelius search format
            search_url = "https://www.intelius.com/people-search"
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
                        "intelius_search_failed",
                        status_code=response.status_code,
                        name=full_name,
                    )
                    return results

                soup = BeautifulSoup(response.text, "html.parser")

                # Parse search results
                result_items = soup.find_all("div", class_=lambda x: x and ("result" in x.lower() or "listing" in x.lower()))

                for item in result_items[:5]:  # Limit to top 5 results
                    detected_info = {}

                    # Extract name
                    name_elem = item.find(["h2", "h3", "span"], class_=lambda x: x and "name" in x.lower())
                    if name_elem:
                        detected_info["full_name"] = name_elem.get_text(strip=True)

                    # Extract location
                    location_elem = item.find(class_=lambda x: x and ("location" in x.lower() or "address" in x.lower()))
                    if location_elem:
                        location_text = location_elem.get_text(strip=True)
                        detected_info["location"] = location_text

                        # Parse city/state
                        if "," in location_text:
                            parts = [p.strip() for p in location_text.split(",")]
                            if len(parts) >= 2:
                                detected_info["city"] = parts[0]
                                import re
                                state_match = re.search(r"\b([A-Z]{2})\b", parts[-1])
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

                    # Extract phone if available
                    phone_elem = item.find(class_=lambda x: x and "phone" in x.lower())
                    if phone_elem:
                        detected_info["phone"] = phone_elem.get_text(strip=True)

                    # Extract email if available
                    email_elem = item.find(class_=lambda x: x and "email" in x.lower())
                    if email_elem:
                        detected_info["email"] = email_elem.get_text(strip=True)

                    # Extract profile URL
                    link_elem = item.find("a", href=True)
                    source_url = "https://www.intelius.com"
                    if link_elem:
                        href = link_elem["href"]
                        if href.startswith("/"):
                            source_url = f"https://www.intelius.com{href}"
                        else:
                            source_url = href

                    if detected_info:
                        result = DetectionResult(
                            source_name=self.get_broker_name(),
                            source_url=source_url,
                            detected_info=detected_info,
                            confidence=0.7,  # Medium-high confidence
                            removal_info=self.get_removal_info(),
                        )
                        results.append(result)

                log.info(
                    "intelius_search_complete",
                    name=full_name,
                    results_found=len(results),
                )

        except Exception as e:
            log.error(
                "intelius_search_error",
                name=full_name,
                error=str(e),
            )

        return results


@register_detector
class PeopleFinderDetector(DataBrokerDetector):
    """Detector for PeopleFinders.com."""

    def get_broker_name(self) -> str:
        return "PeopleFinders"

    def get_broker_domain(self) -> str:
        return "peoplefinders.com"

    def get_removal_info(self) -> Dict:
        return {
            "removal_url": "https://www.peoplefinders.com/manage",
            "removal_difficulty": RemovalDifficulty.EASY,
            "removal_instructions": (
                "1. Visit the PeopleFinders manage page\n"
                "2. Search for your record\n"
                "3. Click 'Remove this record'\n"
                "4. Verify via email\n"
                "5. Removal typically within 24-48 hours"
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
        Search PeopleFinders for a person.

        Note: PeopleFinders may require authentication for full results.
        This implementation uses public search endpoints.
        """
        import httpx
        import structlog
        from bs4 import BeautifulSoup

        log = structlog.get_logger()
        results = []

        try:
            # Construct search query
            name_parts = full_name.split(maxsplit=1)
            if len(name_parts) < 2:
                log.warning("peoplefinders_invalid_name", name=full_name)
                return results

            first_name, last_name = name_parts[0], name_parts[1]

            # PeopleFinders search format
            search_url = "https://www.peoplefinders.com/people"
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
                        "peoplefinders_search_failed",
                        status_code=response.status_code,
                        name=full_name,
                    )
                    return results

                soup = BeautifulSoup(response.text, "html.parser")

                # Parse search results
                result_cards = soup.find_all("div", class_=lambda x: x and ("person" in x.lower() or "result" in x.lower()))

                for card in result_cards[:5]:  # Limit to top 5 results
                    detected_info = {}

                    # Extract name
                    name_elem = card.find(["h2", "h3", "div"], class_=lambda x: x and "name" in x.lower())
                    if name_elem:
                        detected_info["full_name"] = name_elem.get_text(strip=True)

                    # Extract location
                    location_elem = card.find(class_=lambda x: x and ("location" in x.lower() or "city" in x.lower()))
                    if location_elem:
                        location_text = location_elem.get_text(strip=True)
                        detected_info["location"] = location_text

                        # Parse city/state
                        if "," in location_text:
                            parts = [p.strip() for p in location_text.split(",")]
                            if len(parts) >= 2:
                                detected_info["city"] = parts[0]
                                import re
                                state_match = re.search(r"\b([A-Z]{2})\b", parts[-1])
                                if state_match:
                                    detected_info["state"] = state_match.group(1)

                    # Extract age
                    age_elem = card.find(class_=lambda x: x and "age" in x.lower())
                    if age_elem:
                        age_text = age_elem.get_text(strip=True)
                        import re
                        age_match = re.search(r"\d+", age_text)
                        if age_match:
                            detected_info["age"] = int(age_match.group())

                    # Extract phone numbers
                    phone_elem = card.find(class_=lambda x: x and "phone" in x.lower())
                    if phone_elem:
                        detected_info["phone"] = phone_elem.get_text(strip=True)

                    # Extract relatives if shown
                    relatives_elem = card.find(class_=lambda x: x and "relative" in x.lower())
                    if relatives_elem:
                        detected_info["relatives"] = relatives_elem.get_text(strip=True)

                    # Extract profile URL
                    link_elem = card.find("a", href=True)
                    source_url = "https://www.peoplefinders.com"
                    if link_elem:
                        href = link_elem["href"]
                        if href.startswith("/"):
                            source_url = f"https://www.peoplefinders.com{href}"
                        else:
                            source_url = href

                    if detected_info:
                        result = DetectionResult(
                            source_name=self.get_broker_name(),
                            source_url=source_url,
                            detected_info=detected_info,
                            confidence=0.7,  # Medium-high confidence
                            removal_info=self.get_removal_info(),
                        )
                        results.append(result)

                log.info(
                    "peoplefinders_search_complete",
                    name=full_name,
                    results_found=len(results),
                )

        except Exception as e:
            log.error(
                "peoplefinders_search_error",
                name=full_name,
                error=str(e),
            )

        return results
