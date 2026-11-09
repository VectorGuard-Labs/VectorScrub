"""
Data broker detection aggregator.

Runs all registered detectors and consolidates results.
"""

import asyncio
from typing import Dict, List

import structlog

from .base import DetectionResult
from .registry import DataBrokerRegistry


class DataBrokerAggregator:
    """
    Aggregates results from all data broker detectors.

    Runs detectors in parallel and deduplicates results.
    """

    def __init__(self, identity_context: Dict):
        """
        Initialize aggregator with identity context.

        Args:
            identity_context: Customer's identity profile
        """
        self.identity_context = identity_context
        self.log = structlog.get_logger()

    async def search_all_brokers(
        self,
        full_name: str,
        city: str = None,
        state: str = None,
        age: int = None,
        min_confidence: float = 0.6,
    ) -> List[DetectionResult]:
        """
        Search all registered data brokers.

        Args:
            full_name: Person's full name
            city: City (optional)
            state: State abbreviation (optional)
            age: Age (optional)
            min_confidence: Minimum confidence threshold

        Returns:
            List of detection results from all brokers
        """
        detectors = DataBrokerRegistry.get_all_detectors()

        self.log.info(
            "searching_data_brokers",
            num_detectors=len(detectors),
            brokers=[d.broker_name for d in detectors],
        )

        # Run all detectors in parallel
        tasks = [
            self._search_single_broker(detector, full_name, city, state, age)
            for detector in detectors
        ]

        results_by_broker = await asyncio.gather(*tasks, return_exceptions=True)

        # Flatten results and filter by confidence
        all_results = []
        for broker_results in results_by_broker:
            if isinstance(broker_results, Exception):
                self.log.error("broker_search_failed", error=str(broker_results))
                continue

            if isinstance(broker_results, list):
                all_results.extend(broker_results)

        # Filter by confidence and cross-check with identity
        filtered_results = []
        for result in all_results:
            # Recalculate confidence using identity context
            detector = DataBrokerRegistry.get_detector(result.broker_name)
            adjusted_confidence = detector.is_match(
                result.detected_pii, self.identity_context
            )

            # Update result confidence
            result.confidence = max(result.confidence, adjusted_confidence)

            if result.confidence >= min_confidence:
                filtered_results.append(result)

        # Sort by confidence (highest first)
        filtered_results.sort(key=lambda r: r.confidence, reverse=True)

        self.log.info(
            "broker_search_complete",
            total_results=len(all_results),
            filtered_results=len(filtered_results),
            min_confidence=min_confidence,
        )

        return filtered_results

    async def _search_single_broker(
        self,
        detector,
        full_name: str,
        city: str,
        state: str,
        age: int,
    ) -> List[DetectionResult]:
        """Search a single broker with error handling."""
        try:
            self.log.debug(
                "searching_broker",
                broker=detector.broker_name,
                name=full_name,
            )

            results = await detector.search(
                full_name=full_name,
                city=city,
                state=state,
                age=age,
            )

            self.log.info(
                "broker_search_success",
                broker=detector.broker_name,
                num_results=len(results),
            )

            return results

        except Exception as e:
            self.log.error(
                "broker_search_error",
                broker=detector.broker_name,
                error=str(e),
            )
            return []

    def get_removal_summary(self, results: List[DetectionResult]) -> Dict:
        """
        Generate a summary of removal actions needed.

        Args:
            results: List of detection results

        Returns:
            Dictionary with removal summary
        """
        summary = {
            "total_exposures": len(results),
            "brokers": {},
            "by_difficulty": {
                "easy": [],
                "medium": [],
                "hard": [],
                "very_hard": [],
            },
        }

        for result in results:
            broker_name = result.broker_name

            # Add to broker summary
            if broker_name not in summary["brokers"]:
                summary["brokers"][broker_name] = {
                    "count": 0,
                    "removal_url": result.removal_url,
                    "removal_difficulty": result.removal_difficulty.value,
                    "instructions": result.additional_info.get("removal_instructions", ""),
                }

            summary["brokers"][broker_name]["count"] += 1

            # Add to difficulty summary
            difficulty = result.removal_difficulty.value
            summary["by_difficulty"][difficulty].append(
                {"broker": broker_name, "profile_url": result.profile_url}
            )

        return summary
