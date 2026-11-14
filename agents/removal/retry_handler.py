"""
Removal Retry Handler

Processes customer-initiated retries of failed removal requests.
Enforces retry limits, cooldowns, and daily rate limits to prevent abuse.
"""
import asyncio
from dataclasses import dataclass
from typing import Optional
from uuid import UUID

import structlog

from removal.executor import RemovalExecutor
from shared.backend_client import BackendClient
from shared.models import RemovalProcedure, IdentityContext

log = structlog.get_logger(__name__)


@dataclass
class RetryEligibility:
    """Result of retry eligibility check."""
    allowed: bool
    reason: str
    retry_count: int
    max_retries: int
    cooldown_until: Optional[str]
    daily_limit_reached: bool


class RemovalRetryHandler:
    """
    Handles customer-initiated retries of failed removal requests.

    Workflow:
    1. Customer clicks "Retry" on failed removal in dashboard
    2. Backend validates eligibility and creates retry history record
    3. This handler picks up the retry request
    4. Creates fresh browser session and re-attempts removal
    5. Updates retry history with result
    """

    def __init__(self):
        self.backend = BackendClient()
        self.executor = RemovalExecutor()

    async def check_eligibility(
        self,
        removal_request_id: str,
        customer_id: str,
    ) -> RetryEligibility:
        """
        Check if a removal request is eligible for retry.

        Args:
            removal_request_id: UUID of the removal request
            customer_id: UUID of the customer

        Returns:
            RetryEligibility with details
        """
        try:
            # Call backend to check eligibility
            result = await self.backend.check_retry_eligibility(
                removal_request_id,
                customer_id,
            )

            return RetryEligibility(
                allowed=result.get("allowed", False),
                reason=result.get("reason", "Unknown"),
                retry_count=result.get("retry_count", 0),
                max_retries=result.get("max_retries", 3),
                cooldown_until=result.get("cooldown_until"),
                daily_limit_reached=result.get("daily_limit_reached", False),
            )

        except Exception as exc:
            log.error(
                "eligibility_check_failed",
                removal_request_id=removal_request_id,
                error_type=type(exc).__name__,
            )
            return RetryEligibility(
                allowed=False,
                reason=f"Eligibility check failed: {type(exc).__name__}",
                retry_count=0,
                max_retries=0,
                cooldown_until=None,
                daily_limit_reached=False,
            )

    async def initiate_retry(
        self,
        removal_request_id: str,
        customer_id: str,
        initiated_by: str = "customer",
    ) -> dict:
        """
        Initiate a retry for a failed removal request.

        This creates a retry history record and marks the request as pending.
        The actual retry execution happens in process_pending_retries().

        Args:
            removal_request_id: UUID of the removal request
            customer_id: UUID of the customer
            initiated_by: 'customer' or 'system'

        Returns:
            {
                "success": bool,
                "retry_history_id": str | None,
                "message": str,
            }
        """
        log.info(
            "initiating_retry",
            removal_request_id=removal_request_id,
            customer_id=customer_id,
            initiated_by=initiated_by,
        )

        # Check eligibility first
        eligibility = await self.check_eligibility(removal_request_id, customer_id)

        if not eligibility.allowed:
            log.warning(
                "retry_not_allowed",
                removal_request_id=removal_request_id,
                reason=eligibility.reason,
            )
            return {
                "success": False,
                "retry_history_id": None,
                "message": eligibility.reason,
                "eligibility": eligibility.__dict__,
            }

        try:
            # Call backend to initiate retry
            result = await self.backend.initiate_removal_retry(
                removal_request_id,
                customer_id,
                initiated_by,
            )

            log.info(
                "retry_initiated",
                removal_request_id=removal_request_id,
                retry_history_id=result.get("retry_history_id"),
            )

            return {
                "success": True,
                "retry_history_id": result.get("retry_history_id"),
                "message": "Retry initiated successfully",
            }

        except Exception as exc:
            log.error(
                "retry_initiation_failed",
                removal_request_id=removal_request_id,
                error_type=type(exc).__name__,
            )
            return {
                "success": False,
                "retry_history_id": None,
                "message": f"Failed to initiate retry: {type(exc).__name__}",
            }

    async def execute_retry(
        self,
        removal_request_id: str,
        retry_history_id: str,
        customer_id: str,
    ) -> dict:
        """
        Execute a retry attempt for a removal request.

        This is the actual retry logic that runs the removal submission again.

        Args:
            removal_request_id: UUID of the removal request
            retry_history_id: UUID of the retry history record
            customer_id: UUID of the customer

        Returns:
            {
                "status": "submitted" | "failed",
                "confirmation_ref": str | None,
                "evidence_hash": str | None,
                "error": str | None,
            }
        """
        log.info(
            "executing_retry",
            removal_request_id=removal_request_id,
            retry_history_id=retry_history_id,
        )

        try:
            # Fetch removal request details from backend
            request_details = await self.backend.get_removal_request(
                removal_request_id,
                customer_id,
            )

            # Extract necessary info
            procedure_data = request_details.get("procedure", {})
            procedure = RemovalProcedure(
                source_name=procedure_data.get("source_name"),
                procedure_type=procedure_data.get("procedure_type"),
                opt_out_url=procedure_data.get("opt_out_url"),
                opt_out_email=procedure_data.get("opt_out_email"),
                instructions=procedure_data.get("instructions", ""),
                requires_id_verification=procedure_data.get("requires_id_verification", False),
                estimated_days=procedure_data.get("estimated_days", 7),
            )

            # Get identity context
            identity_data = request_details.get("identity", {})
            identity = IdentityContext(
                customer_id=UUID(customer_id),
                full_name=identity_data.get("full_name", ""),
                email=identity_data.get("email"),
                current_address=identity_data.get("current_address"),
                phone_numbers=identity_data.get("phone_numbers", []),
                # ... other fields as needed
            )

            source_url = request_details.get("source_url", "")

            # Execute the removal submission (fresh attempt)
            result = await self.executor.submit_removal_request(
                removal_request_id=removal_request_id,
                customer_id=customer_id,
                procedure=procedure,
                identity=identity,
                source_url=source_url,
            )

            # Update retry history with result
            await self.backend.update_retry_history(
                retry_history_id,
                status="success" if result["status"] == "submitted" else "failed",
                confirmation_ref=result.get("confirmation_ref"),
                evidence_hash=result.get("evidence_hash"),
                error_message=result.get("error"),
            )

            log.info(
                "retry_executed",
                removal_request_id=removal_request_id,
                retry_history_id=retry_history_id,
                status=result["status"],
            )

            return result

        except Exception as exc:
            log.error(
                "retry_execution_failed",
                removal_request_id=removal_request_id,
                retry_history_id=retry_history_id,
                error_type=type(exc).__name__,
            )

            # Update retry history with failure
            try:
                await self.backend.update_retry_history(
                    retry_history_id,
                    status="failed",
                    error_message=f"Execution error: {type(exc).__name__}",
                )
            except Exception:
                pass  # Best effort

            return {
                "status": "failed",
                "confirmation_ref": None,
                "evidence_hash": None,
                "error": f"Retry execution failed: {type(exc).__name__}",
            }

    async def process_pending_retries(self, max_concurrent: int = 3) -> dict:
        """
        Process all pending retry requests.

        This is called by a worker to execute retries that have been initiated
        but not yet executed.

        Args:
            max_concurrent: Maximum concurrent retry executions

        Returns:
            {
                "processed": int,
                "successful": int,
                "failed": int,
            }
        """
        log.info("processing_pending_retries_started")

        try:
            # Fetch pending retries from backend
            pending_retries = await self.backend.get_pending_retries(limit=100)

            if not pending_retries:
                log.debug("no_pending_retries")
                return {
                    "processed": 0,
                    "successful": 0,
                    "failed": 0,
                }

            log.info("pending_retries_found", count=len(pending_retries))

            # Process retries with semaphore for concurrency control
            semaphore = asyncio.Semaphore(max_concurrent)
            results = {"successful": 0, "failed": 0}

            async def process_retry(retry):
                async with semaphore:
                    result = await self.execute_retry(
                        removal_request_id=retry["removal_request_id"],
                        retry_history_id=retry["retry_history_id"],
                        customer_id=retry["customer_id"],
                    )
                    if result["status"] == "submitted":
                        results["successful"] += 1
                    else:
                        results["failed"] += 1

            # Execute all retries
            tasks = [process_retry(retry) for retry in pending_retries]
            await asyncio.gather(*tasks, return_exceptions=True)

            log.info(
                "pending_retries_processed",
                total=len(pending_retries),
                successful=results["successful"],
                failed=results["failed"],
            )

            return {
                "processed": len(pending_retries),
                "successful": results["successful"],
                "failed": results["failed"],
            }

        except Exception as exc:
            log.error(
                "pending_retries_processing_failed",
                error_type=type(exc).__name__,
            )
            return {
                "processed": 0,
                "successful": 0,
                "failed": 0,
            }


# Helper function for orchestrator integration
async def run_retry_worker():
    """
    Main loop for the retry worker.

    Continuously processes pending retries with a delay between iterations.
    Run this as a separate worker process.
    """
    handler = RemovalRetryHandler()

    while True:
        try:
            result = await handler.process_pending_retries(max_concurrent=3)

            # If we processed retries, check again soon
            # Otherwise, wait longer before checking
            if result["processed"] > 0:
                await asyncio.sleep(30)  # 30 seconds
            else:
                await asyncio.sleep(300)  # 5 minutes

        except Exception as exc:
            log.error("retry_worker_error", error_type=type(exc).__name__)
            await asyncio.sleep(60)  # 1 minute backoff on error


if __name__ == "__main__":
    """Run the retry worker."""
    asyncio.run(run_retry_worker())
