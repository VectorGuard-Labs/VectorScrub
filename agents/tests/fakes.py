"""A fake backend that records exactly which calls the agents make."""
from shared.backend_client import IdentityDenied, ScanJobLost

IDENTITY = {
    "full_name": "Jane Marie Smith",
    "current_address": "123 Main St, Austin, TX 78701",
    "previous_addresses": [],
    "phone_numbers": ["512-555-0147"],
    "email_addresses": ["jane.smith@example.com"],
    "date_of_birth": "1984-03-09",
}


class FakeBackend:
    def __init__(self):
        self.calls: list[tuple] = []          # (method, *args) in order
        self.claim_result: int | None = 1
        self.update_errors: list[Exception] = []   # raised by successive update_scan_status calls
        self.record_exposure_error: Exception | None = None
        self.identity_error: Exception | None = None
        self.pending_removals: list[dict] = []
        self.sticky_pending: list[dict] = []
        self.claimed_removals: set[str] = set()
        self.claim_removal_error: Exception | None = None
        self.removal_status_errors: list[Exception] = []   # raised by successive update_removal_status calls
        self.removal_audit_errors: list[Exception] = []    # raised by successive record_removal_audit calls
        self.due_removals: list[dict] = []
        self.due_exposures: list[dict] = []
        self.scan_updates: list[dict] = []
        self.removal_updates: list[dict] = []
        self.audits: list[dict] = []
        self.optout_reports: list[list] = []
        self.optout_report_error: Exception | None = None
        self.deletion_queue: list[dict] = []
        self.deletion_claims_lost: set[str] = set()
        self.deletion_identity: dict | Exception = {
            "full_name": "Jane Marie Smith", "email_addresses": ["jane.smith@example.com"], "current_address": ""}
        self.deletion_results: list[tuple] = []
        self.deletion_result_errors: list[Exception] = []

    def _log(self, *call):
        self.calls.append(call)

    def names(self):
        return [c[0] for c in self.calls]

    # --- deletion requests
    async def list_deletion_requests(self):
        self._log("list_deletion_requests")
        return list(self.deletion_queue)

    async def claim_deletion_request(self, request_id):
        self._log("claim_deletion_request", request_id)
        return request_id not in self.deletion_claims_lost

    async def fetch_deletion_identity(self, request_id):
        self._log("fetch_deletion_identity", request_id)
        if isinstance(self.deletion_identity, Exception):
            raise self.deletion_identity
        return dict(self.deletion_identity)

    async def report_deletion_result(self, request_id, sent, message_id=None, error=None):
        self._log("report_deletion_result", request_id, sent)
        if self.deletion_result_errors:
            raise self.deletion_result_errors.pop(0)
        self.deletion_results.append((request_id, sent, message_id, error))
        return True

    # --- opt-out page health
    async def report_optout_health(self, checks):
        self._log("report_optout_health", len(checks))
        if self.optout_report_error:
            raise self.optout_report_error
        self.optout_reports.append(list(checks))

    # --- scans
    async def claim_scan(self, job_id):
        self._log("claim_scan", job_id)
        return self.claim_result

    async def fetch_scan_identity(self, customer_id, job_id, identity_token):
        self._log("fetch_scan_identity", customer_id, job_id, identity_token)
        if self.identity_error:
            raise self.identity_error
        return dict(IDENTITY)

    async def update_scan_status(self, job_id, status, attempt=None, **kw):
        self._log("update_scan_status", job_id, status.value, attempt)
        self.scan_updates.append({"job_id": job_id, "status": status.value, "attempt": attempt, **kw})
        if self.update_errors:
            raise self.update_errors.pop(0)

    async def record_exposure(self, customer_id, scan_job_id, result):
        self._log("record_exposure", customer_id, scan_job_id)
        if self.record_exposure_error:
            raise self.record_exposure_error

    # --- removals
    async def get_pending_removals(self):
        self._log("get_pending_removals")
        if self.sticky_pending:   # the backend keeps listing these, as it would before the status lands
            return list(self.sticky_pending)
        pending, self.pending_removals = self.pending_removals, []
        return pending

    async def claim_removal(self, removal_request_id):
        """Like the real backend: succeeds once per request, ever."""
        self._log("claim_removal", removal_request_id)
        if self.claim_removal_error:
            raise self.claim_removal_error
        if removal_request_id in self.claimed_removals:
            return False
        self.claimed_removals.add(removal_request_id)
        return True

    async def fetch_removal_identity(self, removal_request_id):
        self._log("fetch_removal_identity", removal_request_id)
        if self.identity_error:
            raise self.identity_error
        return dict(IDENTITY)

    async def update_removal_status(self, removal_request_id, status, **kw):
        self._log("update_removal_status", removal_request_id, status)
        if self.removal_status_errors:
            raise self.removal_status_errors.pop(0)
        self.removal_updates.append({"id": removal_request_id, "status": status, **kw})

    async def record_removal_audit(self, removal_request_id, customer_id, action, actor, details):
        self._log("record_removal_audit", removal_request_id, action)
        if self.removal_audit_errors:
            raise self.removal_audit_errors.pop(0)
        self.audits.append({"id": removal_request_id, "action": action, "details": details})

    # --- monitoring
    async def get_removals_due_for_verification(self, limit=25):
        self._log("get_removals_due_for_verification")
        due, self.due_removals = self.due_removals, []
        return due

    async def get_exposures_due_for_monitoring(self, limit=25):
        self._log("get_exposures_due_for_monitoring")
        due, self.due_exposures = self.due_exposures, []
        return due

    async def fetch_monitoring_identity(self, exposure_id):
        self._log("fetch_monitoring_identity", exposure_id)
        if self.identity_error:
            raise self.identity_error
        return dict(IDENTITY)

    async def report_verification_result(self, request_id, customer_id, outcome):
        self._log("report_verification_result", request_id, outcome)

    async def report_monitoring_result(self, exposure_id, customer_id, outcome):
        self._log("report_monitoring_result", exposure_id, outcome)
