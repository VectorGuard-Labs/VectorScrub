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
