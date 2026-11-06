"""
HTTP client for agent → Rust backend communication.
Agents call back to the backend to report scan progress and record exposures.
"""
import httpx
import structlog

from .config import get_config
from .models import DetectionResult, ScanJobStatus

log = structlog.get_logger(__name__)


class BackendClient:
    """
    Async HTTP client wrapping the VectorScrub internal API endpoints.
    All requests include the agent API key header.
    """

    def __init__(self) -> None:
        cfg = get_config()
        self._base = cfg.backend_api_url.rstrip("/") + "/api/v1"
        self._headers = {
            "X-Agent-Key": cfg.agent_api_key,
            "Content-Type": "application/json",
        }

    async def update_scan_status(
        self,
        job_id: str,
        status: ScanJobStatus,
        sources_completed: int | None = None,
        sources_failed: int | None = None,
        exposures_found: int | None = None,
        error_message: str | None = None,
    ) -> None:
        payload = {
            "job_id": job_id,
            "status": status.value,
            "sources_completed": sources_completed,
            "sources_failed": sources_failed,
            "exposures_found": exposures_found,
            "error_message": error_message,
        }
        async with httpx.AsyncClient(timeout=10) as client:
            resp = await client.put(
                f"{self._base}/internal/scan/status",
                json=payload,
                headers=self._headers,
            )
            resp.raise_for_status()
        log.info("scan_status_updated", job_id=job_id, status=status.value)

    async def record_exposure(
        self,
        customer_id: str,
        scan_job_id: str,
        result: DetectionResult,
    ) -> None:
        payload = {
            "customer_id": customer_id,
            "scan_job_id": scan_job_id,
            "source_name": result.source_name,
            "source_url": result.source_url,
            "pii_category": result.pii_category.value,
            "severity": result.severity.value,
            "pii_summary": result.pii_summary,
            "confidence": result.confidence,
        }
        async with httpx.AsyncClient(timeout=10) as client:
            resp = await client.post(
                f"{self._base}/internal/exposures",
                json=payload,
                headers=self._headers,
            )
            resp.raise_for_status()
        log.info(
            "exposure_recorded",
            source=result.source_name,
            category=result.pii_category.value,
        )

    async def get_pending_removals(self) -> list[dict]:
        """
        Fetches removal requests with status=authorized ready for execution.

        Returns:
            List of dicts with keys: id, customer_id, exposure_id, status,
            source_name, source_url, etc.
        """
        async with httpx.AsyncClient(timeout=10) as client:
            resp = await client.get(
                f"{self._base}/internal/removal-requests?status=authorized",
                headers=self._headers,
            )
            resp.raise_for_status()
            data = resp.json()
            return data.get("requests", [])

    async def update_removal_status(
        self,
        removal_request_id: str,
        status: str,
        confirmation_ref: str | None = None,
        evidence_hash: str | None = None,
        next_check_days: int | None = None,
    ) -> None:
        """Updates removal request status after submission."""
        payload = {
            "status": status,
            "confirmation_ref": confirmation_ref,
            "evidence_hash": evidence_hash,
            "next_check_days": next_check_days,
        }
        async with httpx.AsyncClient(timeout=10) as client:
            resp = await client.put(
                f"{self._base}/internal/removal-requests/{removal_request_id}/status",
                json=payload,
                headers=self._headers,
            )
            resp.raise_for_status()
        log.info("removal_status_updated", removal_request_id=removal_request_id, status=status)

    async def record_removal_audit(
        self,
        removal_request_id: str,
        customer_id: str,
        action: str,
        actor: str,
        details: dict,
    ) -> None:
        """Records an audit log entry for a removal request."""
        payload = {
            "customer_id": customer_id,
            "action": action,
            "actor": actor,
            "details": details,
        }
        async with httpx.AsyncClient(timeout=10) as client:
            resp = await client.post(
                f"{self._base}/internal/removal-requests/{removal_request_id}/audit",
                json=payload,
                headers=self._headers,
            )
            resp.raise_for_status()
        log.debug("removal_audit_recorded", removal_request_id=removal_request_id, action=action)
