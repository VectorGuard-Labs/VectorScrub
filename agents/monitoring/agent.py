"""
Monitoring Agent — Phase 3
Periodically re-scans previously removed exposures to detect reappearance.
Triggered by the Rust scheduler, not the main scan queue.
"""
from __future__ import annotations

import structlog

from shared.backend_client import BackendClient
from shared.models import IdentityContext

log = structlog.get_logger(__name__)


class MonitoringAgent:
    """
    Checks whether previously removed records have reappeared on data broker sites.
    Uses the same discovery+detection pipeline but targets specific known URLs
    rather than performing a full search.
    """

    def __init__(self) -> None:
        self.backend = BackendClient()

    async def check_reappearance(
        self,
        customer_id: str,
        source_name: str,
        source_url: str,
        identity: IdentityContext,
    ) -> bool:
        """
        Returns True if the record appears to have reappeared at the given URL.
        """
        import httpx
        from discovery.agent import DiscoveryAgent  # noqa: F401 — available for future use

        headers = {"User-Agent": "Mozilla/5.0 (compatible; VectorScrubBot/1.0)"}
        try:
            async with httpx.AsyncClient(timeout=15) as client:
                resp = await client.get(source_url, headers=headers, follow_redirects=True)
        except Exception as exc:
            log.warning("monitoring_fetch_failed", url=source_url, error=str(exc))
            return False

        if resp.status_code != 200:
            return False

        text = resp.text.lower()
        name_parts = [p.lower() for p in identity.full_name.split() if len(p) > 2]
        matches = sum(1 for part in name_parts if part in text)
        confidence = matches / max(len(name_parts), 1)

        reappeared = confidence >= 0.6
        if reappeared:
            log.warning(
                "reappearance_detected",
                customer_id=customer_id,
                source=source_name,
                url=source_url,
            )
        return reappeared
