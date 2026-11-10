"""
HTTP client for agent → Rust backend communication.
Agents call back to the backend to report scan progress and record exposures.
"""
import httpx
import structlog

from .config import get_config
from .models import DetectionResult, ScanJobStatus

log = structlog.get_logger(__name__)


class ScanJobLost(Exception):
    """
    The backend says this worker no longer holds the scan job (409): it finished, was
    requeued after a missed heartbeat, or another worker took over. Stop working on it.
    """


class IdentityDenied(Exception):
    """
    The backend refused to release identity (403). Identity is only released for work that
    is currently actionable, so this means the work item is stale or not ours. It is never a
    reason to retry.
    """


class BackendClient:
    """
    Async HTTP client wrapping the VectorScrub internal API endpoints.
    All requests include the agent API key header.

    Identity is requested per work item (a scan job with its token, a removal request, or
    an exposure due for monitoring), never per customer: the backend derives the customer
    from the item and refuses anything that is not currently actionable.
    """

    def __init__(self, transport: httpx.AsyncBaseTransport | None = None) -> None:
        cfg = get_config()
        self._base = cfg.backend_api_url.rstrip("/") + "/api/v1"
        self._headers = {
            "X-Agent-Key": cfg.agent_api_key,
            "Content-Type": "application/json",
        }
        self._transport = transport  # tests inject httpx.MockTransport

    def _client(self, timeout: float = 10) -> httpx.AsyncClient:
        return httpx.AsyncClient(timeout=timeout, transport=self._transport)

    # ------------------------------------------------------------------
    # Scans: claim -> progress/heartbeat -> complete | fail
    # ------------------------------------------------------------------

    async def claim_scan(self, job_id: str) -> int | None:
        """
        Take a queued job. Returns the attempt number (the fencing token to send with every
        later update), or None if the job is not claimable: someone else has it, it is
        finished, or it is gone. A duplicate queue entry therefore does no harm.
        """
        async with self._client() as client:
            resp = await client.post(f"{self._base}/internal/scan/{job_id}/claim", headers=self._headers)
        if resp.status_code == 409:
            return None
        resp.raise_for_status()
        return int(resp.json()["attempt"])

    async def fetch_scan_identity(self, customer_id: str, job_id: str, identity_token: str) -> dict:
        """Identity for a scan job. Needs the job-scoped token that came with the job."""
        return await self._identity(
            "/internal/identity",
            {"customer_id": customer_id, "job_id": job_id, "identity_token": identity_token},
        )

    async def fetch_removal_identity(self, removal_request_id: str) -> dict:
        """Identity to submit an authorized removal, or to verify a submitted one that is due."""
        return await self._identity(f"/internal/removal-requests/{removal_request_id}/identity")

    async def fetch_monitoring_identity(self, exposure_id: str) -> dict:
        """Identity for a reappearance check on a removed exposure that is due."""
        return await self._identity(f"/internal/exposures/{exposure_id}/identity")

    async def _identity(self, path: str, body: dict | None = None) -> dict:
        async with self._client() as client:
            resp = await client.post(f"{self._base}{path}", json=body, headers=self._headers)
        if resp.status_code == 403:
            raise IdentityDenied()
        resp.raise_for_status()
        return resp.json()

    async def update_scan_status(
        self,
        job_id: str,
        status: ScanJobStatus,
        attempt: int | None = None,
        sources_completed: int | None = None,
        sources_failed: int | None = None,
        exposures_found: int | None = None,
        error_message: str | None = None,
    ) -> None:
        """
        Report progress (`running`, which also counts as a heartbeat), completion, or failure.
        Raises ScanJobLost if the backend says this worker no longer holds the job.
        """
        payload = {
            "job_id": job_id,
            "status": status.value,
            "attempt": attempt,
            "sources_completed": sources_completed,
            "sources_failed": sources_failed,
            "exposures_found": exposures_found,
            "error_message": error_message,
        }
        async with self._client() as client:
            resp = await client.put(
                f"{self._base}/internal/scan/status",
                json=payload,
                headers=self._headers,
            )
        if resp.status_code == 409:
            raise ScanJobLost(job_id)
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
        async with self._client() as client:
            resp = await client.post(
                f"{self._base}/internal/exposures",
                json=payload,
                headers=self._headers,
            )
            if resp.status_code == 409:  # the scan job is not running for this customer
                raise ScanJobLost(scan_job_id)
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
        async with self._client() as client:
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
        async with self._client() as client:
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
        async with self._client() as client:
            resp = await client.post(
                f"{self._base}/internal/removal-requests/{removal_request_id}/audit",
                json=payload,
                headers=self._headers,
            )
            resp.raise_for_status()
        log.debug("removal_audit_recorded", removal_request_id=removal_request_id, action=action)

    # ------------------------------------------------------------------
    # Monitoring: removal verification and reappearance detection
    # ------------------------------------------------------------------

    async def get_exposures_due_for_monitoring(self, limit: int = 25) -> list[dict]:
        """
        Removed exposures whose tier interval has elapsed (Premium weekly,
        Personal monthly). Keys: exposure_id, customer_id, source_name,
        source_url, subscription_tier.
        """
        async with self._client() as client:
            resp = await client.get(
                f"{self._base}/internal/exposures/due-for-monitoring",
                params={"limit": limit},
                headers=self._headers,
            )
            resp.raise_for_status()
            return resp.json().get("exposures", [])

    async def report_monitoring_result(
        self, exposure_id: str, customer_id: str, outcome: str
    ) -> None:
        """outcome: 'clear' | 'reappeared' | 'inconclusive'."""
        async with self._client() as client:
            resp = await client.post(
                f"{self._base}/internal/exposures/{exposure_id}/monitoring-result",
                json={"customer_id": customer_id, "outcome": outcome},
                headers=self._headers,
            )
            resp.raise_for_status()
        log.info("monitoring_result_reported", exposure_id=exposure_id, outcome=outcome)

    async def get_removals_due_for_verification(self, limit: int = 25) -> list[dict]:
        """
        Submitted/acknowledged removal requests whose check time has passed.
        Keys: removal_request_id, customer_id, exposure_id, source_name,
        source_url, status, days_since_submission.
        """
        async with self._client() as client:
            resp = await client.get(
                f"{self._base}/internal/removal-requests/due-for-verification",
                params={"limit": limit},
                headers=self._headers,
            )
            resp.raise_for_status()
            return resp.json().get("requests", [])

    async def report_verification_result(
        self, removal_request_id: str, customer_id: str, outcome: str
    ) -> None:
        """outcome: 'removed' | 'still_present' | 'inconclusive'."""
        async with self._client() as client:
            resp = await client.post(
                f"{self._base}/internal/removal-requests/{removal_request_id}/verification-result",
                json={"customer_id": customer_id, "outcome": outcome},
                headers=self._headers,
            )
            resp.raise_for_status()
        log.info(
            "verification_result_reported",
            removal_request_id=removal_request_id,
            outcome=outcome,
        )
