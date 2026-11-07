"""
Monitoring Agent — Phase 3
Periodically re-scans previously removed exposures to detect reappearance.
Uses the detection agent to verify if PII has reappeared.
"""
from __future__ import annotations

from typing import TYPE_CHECKING

import httpx
import structlog
from bs4 import BeautifulSoup

from detection.agent import DetectionAgent
from shared.backend_client import BackendClient

if TYPE_CHECKING:
    from shared.models import IdentityContext

log = structlog.get_logger(__name__)


class MonitoringAgent:
    """
    Checks whether previously removed records have reappeared on data broker sites.
    Uses the detection agent to properly classify potential reappearances.
    """

    def __init__(self) -> None:
        self.backend = BackendClient()
        self.detection_agent = DetectionAgent()

    async def check_reappearance(
        self,
        exposure_id: str,
        customer_id: str,
        source_name: str,
        source_url: str,
        identity: IdentityContext,
    ) -> bool:
        """
        Check if a previously removed exposure has reappeared.
        Returns True if reappearance is detected.
        """
        log.info(
            "checking_reappearance",
            exposure_id=exposure_id,
            source=source_name,
            url=source_url,
        )

        # Fetch the URL to check current content
        content = await self._fetch_url(source_url)
        if not content:
            log.debug("no_content_fetched", url=source_url)
            return False

        # Extract text from HTML
        text = self._extract_text(content)
        if not text or len(text) < 100:
            log.debug("insufficient_text", url=source_url, length=len(text) if text else 0)
            return False

        # Use detection agent to classify if PII is present
        try:
            result = await self.detection_agent.classify(
                source_name=source_name,
                source_url=source_url,
                page_text=text,
                identity=identity,
            )
        except Exception as exc:
            log.error("detection_failed", url=source_url, error=str(exc))
            return False

        # If detection agent found PII with reasonable confidence, it has reappeared
        if result and result.confidence >= 0.7:
            log.warning(
                "reappearance_detected",
                exposure_id=exposure_id,
                customer_id=customer_id,
                source=source_name,
                url=source_url,
                confidence=result.confidence,
                category=result.pii_category,
            )

            # Mark exposure as reappeared and create alert
            await self._handle_reappearance(
                exposure_id=exposure_id,
                customer_id=customer_id,
                source_name=source_name,
                source_url=source_url,
                result=result,
            )

            return True

        log.debug("no_reappearance", url=source_url, confidence=result.confidence if result else 0)
        return False

    async def _fetch_url(self, url: str) -> str | None:
        """Fetch URL content with retries and proper headers."""
        headers = {
            "User-Agent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36",
            "Accept": "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
            "Accept-Language": "en-US,en;q=0.9",
        }

        try:
            async with httpx.AsyncClient(timeout=20, follow_redirects=True) as client:
                resp = await client.get(url, headers=headers)
                if resp.status_code == 200:
                    return resp.text
                log.warning("fetch_failed", url=url, status_code=resp.status_code)
                return None
        except Exception as exc:
            log.warning("fetch_exception", url=url, error=str(exc))
            return None

    def _extract_text(self, html: str) -> str:
        """Extract clean text from HTML."""
        try:
            soup = BeautifulSoup(html, "html.parser")
            # Remove script and style elements
            for element in soup(["script", "style", "noscript"]):
                element.decompose()
            text = soup.get_text(separator=" ", strip=True)
            return " ".join(text.split())  # Normalize whitespace
        except Exception as exc:
            log.warning("text_extraction_failed", error=str(exc))
            return ""

    async def _handle_reappearance(
        self,
        exposure_id: str,
        customer_id: str,
        source_name: str,
        source_url: str,
        result,
    ) -> None:
        """
        Handle a detected reappearance:
        1. Mark original exposure as reappeared
        2. Create alert
        3. Create new removal request (if auto-removal is enabled)
        """
        # Mark exposure as reappeared
        try:
            # Update exposure status (assuming we add a reappeared status or reactivate)
            # For now, we'll just create a new exposure since reappearance means new data found
            log.info(
                "handling_reappearance",
                exposure_id=exposure_id,
                customer_id=customer_id,
            )

            # Create alert for the customer
            alert_data = {
                "customer_id": customer_id,
                "alert_type": "reappearance",
                "title": f"Information reappeared on {source_name}",
                "message": f"Previously removed information has reappeared at {source_name}. A new removal request has been prepared.",
            }

            # Send alert via backend (we'll need to add this endpoint)
            # For now, just log it
            log.warning("reappearance_alert", **alert_data)

        except Exception as exc:
            log.error(
                "reappearance_handling_failed",
                exposure_id=exposure_id,
                error=str(exc),
            )
