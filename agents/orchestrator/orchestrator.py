"""
VectorScrub AI Orchestrator
Coordinates the agent pipelines. Three concurrent workers share this process:

  Scan worker (Redis queue):
    1. Pull job from Redis queue
    2. Run Discovery Agent
    3. Run Detection Agent on candidates
    4. Record confirmed exposures via backend API
    5. Prepare Removal Agent procedures
    6. Update scan job status

  Removal worker: polls for customer-authorized removal requests and submits them.

  Monitoring worker: verifies submitted removals and watches removed records for
  reappearance (see process_monitoring_jobs).

This is the main entry point for the Python agent layer.
"""
import asyncio
import json
import logging
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
from shared.backend_client import BackendClient, IdentityDenied, ScanJobLost
from shared.config import get_config
from shared.models import IdentityContext, ScanJobPayload, ScanJobStatus

log = structlog.get_logger(__name__)

# Monitoring worker pacing (see Orchestrator.process_monitoring_jobs)
MONITORING_BUSY_DELAY_SECONDS = 5        # work was done; check for more shortly
MONITORING_FAILURE_BACKOFF_SECONDS = 300  # everything errored; back off

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
    before_sleep=before_sleep_log(log, logging.INFO),
    reraise=True,
)


class Orchestrator:
    def __init__(self) -> None:
        self.cfg = get_config()
        self.backend = BackendClient()
        self.removal_agent = RemovalAgent()
        self.removal_executor = RemovalExecutor()
        self.monitoring_agent = MonitoringAgent()
        # Removal requests already submitted by this process (duplicate-submission guard).
        self._submitted_request_ids: set[str] = set()

    async def process_job(self, payload: ScanJobPayload) -> None:
        """
        Process one scan job from the queue.

        The queue entry is only a hint. The worker first CLAIMS the job from the backend,
        which succeeds for exactly one worker, so a duplicate entry (the backend re-dispatches
        jobs it fears were lost) is skipped harmlessly. While working it sends a heartbeat;
        if the backend answers that the job is no longer ours (a presumed-dead worker whose
        job was requeued), it stops. Every update carries the attempt number so a stale
        worker cannot overwrite a newer one's result.
        """
        job_id = str(payload.job_id)
        customer_id = str(payload.customer_id)

        attempt = await self.backend.claim_scan(job_id)
        if attempt is None:
            log.info("job_not_claimable_skipping", job_id=job_id)
            return
        log.info("job_started", job_id=job_id, customer_id=customer_id, tier=payload.tier, attempt=attempt)

        work = asyncio.create_task(self._run_scan_pipeline(payload, attempt))
        beat = asyncio.create_task(self._heartbeat(job_id, attempt, work))
        try:
            await work
        except asyncio.CancelledError:
            if asyncio.current_task().cancelling():
                # This worker itself is being shut down. Leave the job as it is: the backend's
                # recovery loop requeues it once the heartbeat goes quiet.
                work.cancel()
                log.warning("job_interrupted", job_id=job_id, attempt=attempt)
                raise
            # Only the pipeline was cancelled, by the heartbeat: the backend took the job away.
            log.warning("job_lost_stopping", job_id=job_id, attempt=attempt)
        except ScanJobLost:
            log.warning("job_lost_stopping", job_id=job_id, attempt=attempt)
        finally:
            beat.cancel()

    async def _run_scan_pipeline(self, payload: ScanJobPayload, attempt: int) -> None:
        job_id = str(payload.job_id)
        customer_id = str(payload.customer_id)
        try:
            # Identity comes from the backend, scoped to this job by the token that was
            # minted for it. The Python layer never holds a key.
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
                attempt=attempt,
                sources_completed=len(candidates),
                exposures_found=len(confirmed_exposures),
            )

        except (ScanJobLost, asyncio.CancelledError):
            raise
        except Exception as exc:
            # Type only: exception text from HTTP/LLM libraries can embed URLs, and the URLs
            # embed the customer's name. This message is stored on the job and shown to them.
            log.error("job_failed", job_id=job_id, error_type=type(exc).__name__)
            await self.backend.update_scan_status(
                job_id,
                ScanJobStatus.FAILED,
                attempt=attempt,
                error_message=f"The scan could not be completed ({type(exc).__name__}).",
            )

    async def _heartbeat(self, job_id: str, attempt: int, work: "asyncio.Task") -> None:
        """
        Tell the backend this worker is alive, every `scan_heartbeat_seconds`. If the backend
        says the job is no longer ours, cancel the pipeline. Transient errors are ignored: the
        backend tolerates minutes of silence before it requeues anything.
        """
        while True:
            await asyncio.sleep(self.cfg.scan_heartbeat_seconds)
            try:
                await self.backend.update_scan_status(job_id, ScanJobStatus.RUNNING, attempt=attempt)
            except ScanJobLost:
                log.warning("heartbeat_job_lost", job_id=job_id, attempt=attempt)
                work.cancel()
                return
            except Exception as exc:
                log.warning("heartbeat_failed", job_id=job_id, error_type=type(exc).__name__)

    @retry_transient
    async def _fetch_identity(self, payload: ScanJobPayload) -> IdentityContext:
        """
        The Python agent does NOT decrypt PII itself. It asks the Rust backend, which checks
        the job-scoped token and that the job is still live, then decrypts with the master key
        and returns the fields. This keeps the master key out of the Python process entirely.

        Retries on transient network errors with exponential backoff.
        """
        data = await self.backend.fetch_scan_identity(
            str(payload.customer_id), str(payload.job_id), payload.identity_token
        )
        return self._identity_context(str(payload.customer_id), data)

    @staticmethod
    def _identity_context(customer_id: str, data: dict) -> IdentityContext:
        """
        Build the agents' view of an identity from the backend response. The backend returns
        full_name and email_addresses only; derive the rest so removal forms and emails are not
        filled with a blank name or missing email.
        """
        full_name = data.get("full_name", "")
        name_parts = full_name.split()
        email_addresses = data.get("email_addresses", [])
        return IdentityContext(
            customer_id=uuid.UUID(customer_id),
            first_name=data.get("first_name") or (name_parts[0] if name_parts else ""),
            last_name=data.get("last_name") or (name_parts[-1] if len(name_parts) > 1 else ""),
            full_name=full_name,
            email=data.get("email") or (email_addresses[0] if email_addresses else None),
            current_address=data.get("current_address", ""),
            previous_addresses=data.get("previous_addresses", []),
            phone_numbers=data.get("phone_numbers", []),
            email_addresses=email_addresses,
            date_of_birth=data.get("date_of_birth"),
        )

    @retry_transient
    async def _identity_for_removal(self, customer_id: str, removal_request_id: str) -> IdentityContext:
        """Identity for submitting or verifying one removal request (scoped to that request)."""
        return self._identity_context(customer_id, await self.backend.fetch_removal_identity(removal_request_id))

    @retry_transient
    async def _identity_for_exposure(self, customer_id: str, exposure_id: str) -> IdentityContext:
        """Identity for a reappearance check on one exposure (scoped to that exposure)."""
        return self._identity_context(customer_id, await self.backend.fetch_monitoring_identity(exposure_id))

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
                log.error("worker_loop_error", error_type=type(exc).__name__)
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

                    # Never submit the same request twice. If a previous status update
                    # failed after a successful submission, the request is still
                    # "authorized" in the backend and would otherwise be resubmitted.
                    if request_id in self._submitted_request_ids:
                        log.warning("skipping_already_submitted", request_id=request_id)
                        continue

                    # Get procedure from removal_agent
                    procedure = self.removal_agent.get_procedure(source_name)
                    if not procedure:
                        log.warning("no_procedure", source=source_name, request_id=request_id)
                        await self._fail_removal(
                            request_id, customer_id, "No removal procedure found for source"
                        )
                        continue

                    # Fetch decrypted identity from backend. A failure here is not
                    # going to fix itself on the next poll (e.g. no identity on file),
                    # so end the request as failed instead of retrying forever.
                    try:
                        identity = await self._identity_for_removal(customer_id, request_id)
                    except IdentityDenied:
                        # The backend only releases identity for a request that is actionable;
                        # refusing means it no longer is (for example the account was closed).
                        log.warning("identity_denied", request_id=request_id)
                        await self._fail_removal(
                            request_id, customer_id, "Identity could not be released for submission"
                        )
                        continue
                    except Exception as exc:
                        log.error("identity_fetch_failed", request_id=request_id, error_type=type(exc).__name__)
                        await self._fail_removal(
                            request_id, customer_id, "Could not load identity for submission"
                        )
                        continue

                    # Execute removal
                    result = await self.removal_executor.submit_removal_request(
                        removal_request_id=request_id,
                        customer_id=customer_id,
                        procedure=procedure,
                        identity=identity,
                        source_url=source_url,
                    )
                    self._submitted_request_ids.add(request_id)

                    try:
                        # Update backend status ("submitted" or "failed" — both are
                        # terminal for this poll, so the request leaves "authorized").
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
                    except Exception as exc:
                        log.error(
                            "removal_status_update_failed",
                            request_id=request_id,
                            status=result["status"],
                            error_type=type(exc).__name__,
                        )

                    # Space out submissions to third-party sites.
                    await asyncio.sleep(self.cfg.removal_request_delay_seconds)

                await asyncio.sleep(60)  # Poll every 60 seconds

            except asyncio.CancelledError:
                log.info("removal_worker_shutting_down")
                break
            except Exception as exc:
                log.error("removal_loop_error", error_type=type(exc).__name__)
                await asyncio.sleep(5)

    async def _fail_removal(self, request_id: str, customer_id: str, reason: str) -> None:
        """
        End a removal request as 'failed' so it stops being polled and the customer
        is alerted (and can re-authorize). Best-effort: if the backend is
        unreachable the request simply stays 'authorized' and is retried next poll.
        """
        try:
            await self.backend.update_removal_status(
                removal_request_id=request_id, status="failed"
            )
            await self.backend.record_removal_audit(
                removal_request_id=request_id,
                customer_id=customer_id,
                action="failed",
                actor="removal_executor",
                details={"error": reason},
            )
        except Exception as exc:
            log.error("fail_removal_report_failed", request_id=request_id, error_type=type(exc).__name__)

    async def process_monitoring_jobs(self) -> None:
        """
        Continuous monitoring worker. Each cycle:

          1. Verification: submitted removal requests whose check time has passed
             are re-checked. Only a page that is gone/clean moves a request to
             'verified_removed'; a request that never verifies ends as
             'rejected'/'unverifiable' (backend-enforced timeout) and the customer
             is alerted.
          2. Reappearance: removed exposures are re-checked on the customer's tier
             interval (Premium weekly, Personal monthly; Free is never monitored).
             A reappearance reactivates the exposure and alerts the customer. It
             never submits a removal — that needs fresh customer authorization.

        What is due, and how often, is decided entirely by the backend; this loop
        just asks for work and reports results.
        """
        log.info("monitoring_worker_started")

        while True:
            try:
                handled, failed = await self._run_monitoring_cycle()

                if handled:
                    delay = MONITORING_BUSY_DELAY_SECONDS   # more work may be waiting
                elif failed:
                    delay = MONITORING_FAILURE_BACKOFF_SECONDS
                else:
                    delay = self.cfg.monitoring_idle_seconds
                await asyncio.sleep(delay)

            except asyncio.CancelledError:
                log.info("monitoring_worker_shutting_down")
                break
            except Exception as exc:
                log.error("monitoring_loop_error", error_type=type(exc).__name__)
                await asyncio.sleep(60)

    async def _run_monitoring_cycle(self) -> tuple[int, int]:
        """
        Run one verification + reappearance pass. Returns (handled, failed) counts
        of targets processed and targets whose check raised an unexpected error.
        """
        limit = self.cfg.monitoring_batch_size
        delay = self.cfg.monitoring_request_delay_seconds
        handled = failed = 0

        # Each work item gets its OWN identity release (scoped to it, and audited by the
        # backend). Identity is deliberately not cached across items, even for the same
        # customer: reusing one item's release for another would skip that item's check.

        # --- 1. Removal verification ------------------------------------
        for item in await self.backend.get_removals_due_for_verification(limit):
            request_id = item["removal_request_id"]
            customer_id = item["customer_id"]
            try:
                identity = await self._identity_for_removal(customer_id, request_id)
                await self.monitoring_agent.verify_removal(
                    removal_request_id=request_id,
                    customer_id=customer_id,
                    source_name=item["source_name"],
                    source_url=item["source_url"],
                    identity=identity,
                )
                handled += 1
            except IdentityDenied:
                # No longer actionable (done, cancelled, account closed): nothing to report.
                log.info("verification_identity_denied", removal_request_id=request_id)
                handled += 1
            except httpx.HTTPStatusError as exc:
                if exc.response.status_code in (404, 409):
                    # Changed under us (already verified, cancelled, ...): not an error.
                    log.info("verification_target_stale", removal_request_id=request_id)
                    handled += 1
                else:
                    failed += 1
                    log.error("verification_report_failed", removal_request_id=request_id,
                              status_code=exc.response.status_code)
            except Exception as exc:
                failed += 1
                log.error("verification_failed", removal_request_id=request_id, error_type=type(exc).__name__)
                # Push it out a day so a persistent error can't spin this loop.
                await self._best_effort(
                    self.backend.report_verification_result(request_id, customer_id, "inconclusive")
                )
            await asyncio.sleep(delay)

        # --- 2. Reappearance monitoring ---------------------------------
        for item in await self.backend.get_exposures_due_for_monitoring(limit):
            exposure_id = item["exposure_id"]
            customer_id = item["customer_id"]
            try:
                identity = await self._identity_for_exposure(customer_id, exposure_id)
                await self.monitoring_agent.check_reappearance(
                    exposure_id=exposure_id,
                    customer_id=customer_id,
                    source_name=item["source_name"],
                    source_url=item["source_url"],
                    identity=identity,
                )
                handled += 1
            except IdentityDenied:
                log.info("monitoring_identity_denied", exposure_id=exposure_id)
                handled += 1
            except httpx.HTTPStatusError as exc:
                if exc.response.status_code in (404, 409):
                    log.info("monitoring_target_stale", exposure_id=exposure_id)
                    handled += 1
                else:
                    failed += 1
                    log.error("monitoring_report_failed", exposure_id=exposure_id,
                              status_code=exc.response.status_code)
            except Exception as exc:
                failed += 1
                log.error("monitoring_check_failed", exposure_id=exposure_id, error_type=type(exc).__name__)
                await self._best_effort(
                    self.backend.report_monitoring_result(exposure_id, customer_id, "inconclusive")
                )
            await asyncio.sleep(delay)

        if handled or failed:
            log.info("monitoring_cycle_complete", handled=handled, failed=failed)
        return handled, failed

    @staticmethod
    async def _best_effort(coro) -> None:
        """Await a reporting call, logging instead of raising if it fails."""
        try:
            await coro
        except Exception as exc:
            log.warning("best_effort_report_failed", error_type=type(exc).__name__)


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
