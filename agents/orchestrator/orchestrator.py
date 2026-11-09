"""
VectorScrub AI Orchestrator
Coordinates the full scan pipeline:
  1. Pull job from Redis queue
  2. Run Discovery Agent
  3. Run Detection Agent on candidates
  4. Record confirmed exposures via backend API
  5. Prepare Removal Agent procedures
  6. Update scan job status

This is the main entry point for the Python agent layer.
"""
import asyncio
import json
import uuid

import redis.asyncio as aioredis
import structlog
from tenacity import (
    retry,
    stop_after_attempt,
    wait_exponential,
    retry_if_exception_type,
    before_sleep_log,
)
import httpx

from detection import DetectionAgent
from discovery import DiscoveryAgent
from monitoring.agent import MonitoringAgent
from removal import RemovalAgent, RemovalExecutor
from shared.backend_client import BackendClient
from shared.config import get_config
from shared.models import IdentityContext, ScanJobPayload, ScanJobStatus

log = structlog.get_logger(__name__)


# Retry decorator for transient network/timeout errors
retry_transient = retry(
    stop=stop_after_attempt(3),
    wait=wait_exponential(multiplier=1, min=2, max=10),
    retry=retry_if_exception_type((
        httpx.TimeoutException,
        httpx.NetworkError,
        httpx.ConnectError,
        ConnectionError,
        TimeoutError,
    )),
    before_sleep=before_sleep_log(log, structlog.INFO),
    reraise=True,
)


class Orchestrator:
    def __init__(self) -> None:
        self.cfg = get_config()
        self.backend = BackendClient()
        self.removal_agent = RemovalAgent()
        self.removal_executor = RemovalExecutor()
        self.monitoring_agent = MonitoringAgent()

    async def process_job(self, payload: ScanJobPayload) -> None:
        job_id = str(payload.job_id)
        customer_id = str(payload.customer_id)

        log.info("job_started", job_id=job_id, customer_id=customer_id, tier=payload.tier)

        # Mark job as running
        await self.backend.update_scan_status(job_id, ScanJobStatus.RUNNING)

        try:
            # Retrieve decrypted identity from backend using the identity token
            identity = await self._fetch_identity(payload)

            # Phase 1a: Discovery
            discovery_agent = DiscoveryAgent(tier=payload.tier)
            candidates = await discovery_agent.run(identity)

            # Phase 1b: Detection
            detection_agent = DetectionAgent()
            confirmed_exposures = await detection_agent.run(candidates, identity)

            # Record each confirmed exposure via backend API
            for exposure in confirmed_exposures:
                await self.backend.record_exposure(customer_id, job_id, exposure)

            # Phase 2 prep: Identify removal procedures (stored for customer review)
            removal_pairs = self.removal_agent.prepare_removal_requests(confirmed_exposures)
            sources_with_procedures = sum(1 for _, p in removal_pairs if p is not None)

            log.info(
                "job_completed",
                job_id=job_id,
                candidates=len(candidates),
                confirmed=len(confirmed_exposures),
                removal_procedures_found=sources_with_procedures,
            )

            await self.backend.update_scan_status(
                job_id,
                ScanJobStatus.COMPLETED,
                sources_completed=len(candidates),
                exposures_found=len(confirmed_exposures),
            )

        except Exception as exc:
            log.error("job_failed", job_id=job_id, error=str(exc), exc_info=True)
            await self.backend.update_scan_status(
                job_id,
                ScanJobStatus.FAILED,
                error_message=str(exc)[:500],
            )

    @retry_transient
    async def _fetch_identity(self, payload: ScanJobPayload) -> IdentityContext:
        """
        The Python agent does NOT decrypt PII itself.
        It calls the Rust backend's internal identity endpoint,
        which decrypts using the master key and returns minimal fields.
        This keeps the master key out of the Python process entirely.

        Retries on transient network errors with exponential backoff.
        """
        async with httpx.AsyncClient(timeout=10) as client:
            resp = await client.post(
                f"{self.cfg.backend_api_url}/api/v1/internal/identity",
                json={
                    "customer_id": str(payload.customer_id),
                    "identity_token": payload.identity_token,
                },
                headers={"X-Agent-Key": self.cfg.agent_api_key},
            )
            resp.raise_for_status()
            data = resp.json()

        return IdentityContext(
            customer_id=payload.customer_id,
            full_name=data.get("full_name", ""),
            current_address=data.get("current_address", ""),
            previous_addresses=data.get("previous_addresses", []),
            phone_numbers=data.get("phone_numbers", []),
            email_addresses=data.get("email_addresses", []),
            date_of_birth=data.get("date_of_birth"),
        )

    async def run_worker(self) -> None:
        """
        Long-running worker loop. Blocks on Redis BRPOP and processes
        scan jobs as they arrive.
        """
        redis_client = await aioredis.from_url(self.cfg.redis_url)
        log.info("orchestrator_worker_started", queue=self.cfg.scan_queue_key)

        while True:
            try:
                # BRPOP blocks until a job is available (5-second timeout for clean shutdown)
                item = await redis_client.brpop(self.cfg.scan_queue_key, timeout=5)
                if item is None:
                    continue

                _, raw = item
                payload_data = json.loads(raw)
                payload = ScanJobPayload(**payload_data)
                await self.process_job(payload)

            except asyncio.CancelledError:
                log.info("orchestrator_worker_shutting_down")
                break
            except Exception as exc:
                log.error("worker_loop_error", error=str(exc), exc_info=True)
                await asyncio.sleep(2)

        await redis_client.aclose()

    async def process_removal_jobs(self) -> None:
        """
        Poll for authorized removal requests and execute them.
        Runs continuously alongside the scan worker.
        """
        log.info("removal_worker_started")

        while True:
            try:
                # Fetch pending removals from backend
                pending = await self.backend.get_pending_removals()

                if not pending:
                    await asyncio.sleep(60)  # Poll every 60 seconds
                    continue

                log.info("processing_removals", count=len(pending))

                for request in pending:
                    request_id = request["id"]
                    customer_id = request["customer_id"]
                    source_name = request["source_name"]
                    source_url = request["source_url"]

                    # Get procedure from removal_agent
                    procedure = self.removal_agent.get_procedure(source_name)
                    if not procedure:
                        log.warning("no_procedure", source=source_name, request_id=request_id)
                        await self.backend.record_removal_audit(
                            removal_request_id=request_id,
                            customer_id=customer_id,
                            action="skipped",
                            actor="removal_executor",
                            details={"error": "No removal procedure found for source"},
                        )
                        continue

                    # Fetch decrypted identity from backend
                    identity = await self._fetch_identity_for_customer(customer_id)

                    # Execute removal
                    result = await self.removal_executor.submit_removal_request(
                        removal_request_id=request_id,
                        customer_id=customer_id,
                        procedure=procedure,
                        identity=identity,
                        source_url=source_url,
                    )

                    # Update backend status
                    await self.backend.update_removal_status(
                        removal_request_id=request_id,
                        status=result["status"],
                        confirmation_ref=result.get("confirmation_ref"),
                        evidence_hash=result.get("evidence_hash"),
                        next_check_days=procedure.estimated_days if result["status"] == "submitted" else None,
                    )

                    # Log to audit trail
                    await self.backend.record_removal_audit(
                        removal_request_id=request_id,
                        customer_id=customer_id,
                        action=result["status"],
                        actor="removal_executor",
                        details={
                            "procedure_type": procedure.procedure_type,
                            "error": result.get("error"),
                            "confirmation_ref": result.get("confirmation_ref"),
                        },
                    )

                await asyncio.sleep(60)  # Poll every 60 seconds

            except asyncio.CancelledError:
                log.info("removal_worker_shutting_down")
                break
            except Exception as exc:
                log.error("removal_loop_error", error=str(exc), exc_info=True)
                await asyncio.sleep(5)

    @retry_transient
    async def _fetch_identity_for_customer(self, customer_id: str) -> IdentityContext:
        """
        Fetch decrypted identity for a customer by customer_id.
        Used by removal worker to get PII for form submission.

        Retries on transient network errors with exponential backoff.
        """
        async with httpx.AsyncClient(timeout=10) as client:
            resp = await client.get(
                f"{self.cfg.backend_api_url}/api/v1/internal/customers/{customer_id}/identity",
                headers={"X-Agent-Key": self.cfg.agent_api_key},
            )
            resp.raise_for_status()
            data = resp.json()

        return IdentityContext(
            customer_id=uuid.UUID(customer_id),
            first_name=data.get("first_name", ""),
            last_name=data.get("last_name", ""),
            full_name=data.get("full_name", ""),
            email=data.get("email"),
            current_address=data.get("current_address", ""),
            previous_addresses=data.get("previous_addresses", []),
            phone_numbers=data.get("phone_numbers", []),
            email_addresses=data.get("email_addresses", []),
            date_of_birth=data.get("date_of_birth"),
        )

    async def process_monitoring_jobs(self) -> None:
        """
        Periodically check removed exposures for reappearance.
        Tier-based frequency: Premium/Family (weekly), Personal (monthly).
        """
        log.info("monitoring_worker_started")

        while True:
            try:
                # Fetch removed exposures due for monitoring check
                # This would require a backend endpoint to query exposures where:
                # - is_active = false (removed)
                # - monitoring_enabled = true for the customer
                # - last_monitored_at is NULL or > frequency interval

                # For now, we'll poll every hour and check exposures
                # In production, the backend should provide an endpoint like:
                # GET /internal/exposures/due-for-monitoring

                # Placeholder: sleep for monitoring interval
                # Premium/Family: check weekly, Personal: check monthly
                await asyncio.sleep(3600)  # Check every hour for due exposures

                log.debug("monitoring_check_cycle")

                # TODO: Add backend endpoint and integrate full monitoring workflow
                # For now, this is a scaffold for Phase 3

            except asyncio.CancelledError:
                log.info("monitoring_worker_shutting_down")
                break
            except Exception as exc:
                log.error("monitoring_loop_error", error=str(exc), exc_info=True)
                await asyncio.sleep(60)


async def main() -> None:
    import structlog
    structlog.configure(
        processors=[
            structlog.processors.TimeStamper(fmt="iso"),
            structlog.processors.JSONRenderer(),
        ]
    )
    orchestrator = Orchestrator()

    # Run scan worker, removal worker, and monitoring worker concurrently
    await asyncio.gather(
        orchestrator.run_worker(),
        orchestrator.process_removal_jobs(),
        orchestrator.process_monitoring_jobs(),
    )


if __name__ == "__main__":
    asyncio.run(main())
