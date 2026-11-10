"""
The orchestrator's three workers against a fake backend that records every call.

The properties under test:
  * scans: claim first; heartbeat while working; stop when the backend says the job is gone;
    every update carries the attempt; failure messages never contain raw exception text;
  * identity is only ever requested for a specific work item;
  * removals that cannot proceed end as 'failed' instead of being retried forever;
  * a check that errors is rescheduled, not retried in a hot loop.
"""
import asyncio
import uuid

import httpx
import pytest

import orchestrator.orchestrator as orch_mod
from orchestrator.orchestrator import Orchestrator
from shared.backend_client import IdentityDenied, ScanJobLost
from shared.models import DetectionResult, ExposureSeverity, PiiCategory, ScanJobPayload
from tests.fakes import FakeBackend

CUSTOMER = str(uuid.uuid4())


def payload(job_id=None) -> ScanJobPayload:
    return ScanJobPayload(
        job_id=job_id or uuid.uuid4(), customer_id=uuid.UUID(CUSTOMER),
        identity_token="scoped-token-for-this-job", tier="premium",
    )


def detection() -> DetectionResult:
    return DetectionResult(source_name="spokeo.com", source_url="https://www.spokeo.com/x", pii_category=PiiCategory.FULL_NAME,
                           severity=ExposureSeverity.HIGH, pii_summary="J*** S***", confidence=0.9)


@pytest.fixture
def orch(monkeypatch):
    o = Orchestrator()
    o.backend = FakeBackend()
    o.monitoring_agent.backend = o.backend
    monkeypatch.setattr(o.cfg, "scan_heartbeat_seconds", 0.05)
    monkeypatch.setattr(o.cfg, "removal_request_delay_seconds", 0)
    monkeypatch.setattr(o.cfg, "monitoring_request_delay_seconds", 0)

    class Discovery:
        def __init__(self, tier): pass
        async def run(self, identity): return ["candidate-1", "candidate-2"]

    class Detection:
        async def run(self, candidates, identity): return [detection()]

    monkeypatch.setattr(orch_mod, "DiscoveryAgent", Discovery)
    monkeypatch.setattr(orch_mod, "DetectionAgent", Detection)
    return o


# ===========================================================================
# Scans
# ===========================================================================

async def test_a_job_that_cannot_be_claimed_is_skipped(orch):
    orch.backend.claim_result = None  # a duplicate queue entry; someone else has it
    await orch.process_job(payload())
    assert orch.backend.names() == ["claim_scan"], "no identity fetched, nothing done"


async def test_the_happy_path_claims_then_fetches_scoped_identity_then_completes_with_the_attempt(orch):
    p = payload()
    orch.backend.claim_result = 4
    await orch.process_job(p)

    names = orch.backend.names()
    assert names[0] == "claim_scan"
    assert ("fetch_scan_identity", CUSTOMER, str(p.job_id), "scoped-token-for-this-job") in orch.backend.calls
    assert "record_exposure" in names
    final = orch.backend.scan_updates[-1]
    assert (final["status"], final["attempt"], final["exposures_found"]) == ("completed", 4, 1)


async def test_a_failing_scan_reports_failed_with_the_attempt_and_a_message_that_leaks_nothing(orch, monkeypatch):
    class Boom:
        def __init__(self, tier): pass
        async def run(self, identity):
            raise RuntimeError("GET https://www.spokeo.com/Jane-Smith/Austin failed: password=hunter2")

    monkeypatch.setattr(orch_mod, "DiscoveryAgent", Boom)
    orch.backend.claim_result = 2
    await orch.process_job(payload())

    failed = orch.backend.scan_updates[-1]
    assert failed["status"] == "failed" and failed["attempt"] == 2
    msg = failed["error_message"]
    assert msg == "The scan could not be completed (RuntimeError)."
    assert "Jane" not in msg and "spokeo" not in msg and "hunter2" not in msg


async def test_losing_the_job_while_recording_stops_quietly_without_reporting_failure(orch):
    orch.backend.record_exposure_error = ScanJobLost("job")
    await orch.process_job(payload())
    statuses = [u["status"] for u in orch.backend.scan_updates]
    assert "failed" not in statuses and "completed" not in statuses, statuses


async def test_an_identity_refusal_fails_the_scan_cleanly(orch):
    orch.backend.identity_error = IdentityDenied()
    await orch.process_job(payload())
    assert orch.backend.scan_updates[-1]["status"] == "failed"
    assert "record_exposure" not in orch.backend.names()


async def test_heartbeats_are_sent_with_the_attempt_while_work_is_slow(orch, monkeypatch):
    class Slow:
        def __init__(self, tier): pass
        async def run(self, identity):
            await asyncio.sleep(0.4)
            return []

    monkeypatch.setattr(orch_mod, "DiscoveryAgent", Slow)
    orch.backend.claim_result = 7
    await orch.process_job(payload())

    beats = [u for u in orch.backend.scan_updates if u["status"] == "running"]
    assert len(beats) >= 3, f"expected several heartbeats during 0.4s of work, got {len(beats)}"
    assert all(b["attempt"] == 7 for b in beats)
    assert orch.backend.scan_updates[-1]["status"] == "completed"


async def test_when_the_backend_says_the_job_is_lost_the_worker_stops_working(orch, monkeypatch):
    started, finished = [], []

    class Hang:
        def __init__(self, tier): pass
        async def run(self, identity):
            started.append(1)
            await asyncio.sleep(30)   # would run for 30s if not stopped
            finished.append(1)
            return []

    monkeypatch.setattr(orch_mod, "DiscoveryAgent", Hang)
    orch.backend.update_errors = [ScanJobLost("job")]  # the first heartbeat is refused

    await asyncio.wait_for(orch.process_job(payload()), timeout=5)

    assert started and not finished, "the pipeline was cancelled, not allowed to finish"
    assert all(u["status"] == "running" for u in orch.backend.scan_updates), "a lost worker reports no result"


async def test_transient_heartbeat_failures_do_not_kill_the_scan(orch, monkeypatch):
    class Slow:
        def __init__(self, tier): pass
        async def run(self, identity):
            await asyncio.sleep(0.3)
            return []

    monkeypatch.setattr(orch_mod, "DiscoveryAgent", Slow)
    orch.backend.update_errors = [httpx.ConnectError("blip"), httpx.ConnectError("blip")]
    await orch.process_job(payload())
    assert orch.backend.scan_updates[-1]["status"] == "completed"


async def test_shutting_down_mid_scan_leaves_the_job_for_recovery(orch, monkeypatch):
    class Hang:
        def __init__(self, tier): pass
        async def run(self, identity):
            await asyncio.sleep(30)

    monkeypatch.setattr(orch_mod, "DiscoveryAgent", Hang)
    task = asyncio.create_task(orch.process_job(payload()))
    await asyncio.sleep(0.15)
    task.cancel()
    with pytest.raises(asyncio.CancelledError):
        await task
    statuses = [u["status"] for u in orch.backend.scan_updates]
    assert "failed" not in statuses and "completed" not in statuses, "no verdict: the backend requeues it when heartbeats stop"


# ===========================================================================
# Removal worker
# ===========================================================================

async def run_removal_worker_until(orch, condition, timeout=3):
    task = asyncio.create_task(orch.process_removal_jobs())
    try:
        deadline = asyncio.get_event_loop().time() + timeout
        while not condition() and asyncio.get_event_loop().time() < deadline:
            await asyncio.sleep(0.02)
    finally:
        task.cancel()
        await task  # the worker treats cancellation as a graceful shutdown and returns


def removal(source="spokeo.com"):
    return {"id": str(uuid.uuid4()), "customer_id": CUSTOMER, "source_name": source, "source_url": f"https://www.{source}/x"}


async def test_removal_identity_is_requested_for_the_request_not_the_customer(orch, monkeypatch):
    req = removal()
    orch.backend.pending_removals = [req]

    async def fake_submit(**kw):
        return {"status": "submitted", "confirmation_ref": "REF", "evidence_hash": "abc", "error": None}

    monkeypatch.setattr(orch.removal_executor, "submit_removal_request", lambda **kw: fake_submit(**kw))
    await run_removal_worker_until(orch, lambda: orch.backend.removal_updates)

    assert ("fetch_removal_identity", req["id"]) in orch.backend.calls
    assert orch.backend.removal_updates[0]["status"] == "submitted"
    assert orch.backend.removal_updates[0]["confirmation_ref"] == "REF"


async def test_the_identity_the_executor_receives_has_a_real_name_and_email(orch, monkeypatch):
    got = {}

    async def fake_submit(**kw):
        got["identity"] = kw["identity"]
        return {"status": "submitted", "confirmation_ref": None, "evidence_hash": None, "error": None}

    orch.backend.pending_removals = [removal()]
    monkeypatch.setattr(orch.removal_executor, "submit_removal_request", lambda **kw: fake_submit(**kw))
    await run_removal_worker_until(orch, lambda: orch.backend.removal_updates)
    ident = got["identity"]
    assert (ident.first_name, ident.last_name) == ("Jane", "Smith"), "removal forms used to be filled with a blank name"
    assert ident.email == "jane.smith@example.com"


async def test_a_refused_identity_ends_the_request_as_failed_instead_of_retrying_forever(orch):
    req = removal()
    orch.backend.pending_removals = [req]
    orch.backend.identity_error = IdentityDenied()
    await run_removal_worker_until(orch, lambda: orch.backend.removal_updates)
    assert orch.backend.removal_updates == [{"id": req["id"], "status": "failed"}]
    assert orch.backend.audits[0]["action"] == "failed"


async def test_a_source_with_no_procedure_ends_as_failed_with_one_audit_entry(orch):
    req = removal("unknownbroker.example")
    orch.backend.pending_removals = [req]
    await run_removal_worker_until(orch, lambda: orch.backend.removal_updates)
    assert orch.backend.removal_updates[0]["status"] == "failed"
    assert len(orch.backend.audits) == 1, "the append-only log must not be spammed on every poll"
    assert "fetch_removal_identity" not in orch.backend.names()


async def test_a_failed_submission_is_reported_failed_and_not_resubmitted(orch, monkeypatch):
    req = removal()
    calls = []

    async def fake_submit(**kw):
        calls.append(1)
        return {"status": "failed", "confirmation_ref": None, "evidence_hash": None, "error": "CAPTCHA"}

    monkeypatch.setattr(orch.removal_executor, "submit_removal_request", lambda **kw: fake_submit(**kw))
    # the backend (wrongly) keeps listing it as pending, as it would if the status update failed
    orch.backend.pending_removals = [req]
    await run_removal_worker_until(orch, lambda: orch.backend.removal_updates)
    orch.backend.pending_removals = [req]
    orch._submitted_request_ids.add(req["id"])
    await run_removal_worker_until(orch, lambda: False, timeout=0.2)
    assert len(calls) == 1, "the duplicate-submission guard stops a second attempt at the broker"
    assert orch.backend.removal_updates[0]["status"] == "failed"


# ===========================================================================
# Monitoring worker
# ===========================================================================

def due_removal():
    return {"removal_request_id": str(uuid.uuid4()), "customer_id": CUSTOMER, "source_name": "spokeo.com", "source_url": "https://www.spokeo.com/x"}


def due_exposure():
    return {"exposure_id": str(uuid.uuid4()), "customer_id": CUSTOMER, "source_name": "spokeo.com", "source_url": "https://www.spokeo.com/x"}


@pytest.fixture
def agent_outcomes(orch, monkeypatch):
    """Replace the page-checking with canned outcomes so these tests are about orchestration."""
    seen = {"verify": [], "check": []}

    async def verify_removal(removal_request_id, customer_id, source_name, source_url, identity):
        seen["verify"].append((removal_request_id, identity.full_name))
        await orch.backend.report_verification_result(removal_request_id, customer_id, "removed")

    async def check_reappearance(exposure_id, customer_id, source_name, source_url, identity):
        seen["check"].append((exposure_id, identity.full_name))
        await orch.backend.report_monitoring_result(exposure_id, customer_id, "clear")

    monkeypatch.setattr(orch.monitoring_agent, "verify_removal", verify_removal)
    monkeypatch.setattr(orch.monitoring_agent, "check_reappearance", check_reappearance)
    return seen


async def test_monitoring_asks_for_identity_per_item_not_per_customer(orch, agent_outcomes):
    r, e = due_removal(), due_exposure()
    orch.backend.due_removals, orch.backend.due_exposures = [r], [e]

    handled, failed = await orch._run_monitoring_cycle()

    assert (handled, failed) == (2, 0)
    assert ("fetch_removal_identity", r["removal_request_id"]) in orch.backend.calls
    assert ("fetch_monitoring_identity", e["exposure_id"]) in orch.backend.calls
    assert agent_outcomes["verify"] == [(r["removal_request_id"], "Jane Marie Smith")]


async def test_an_item_the_backend_will_not_release_identity_for_is_skipped_quietly(orch, agent_outcomes):
    orch.backend.due_removals, orch.backend.due_exposures = [due_removal()], [due_exposure()]
    orch.backend.identity_error = IdentityDenied()

    handled, failed = await orch._run_monitoring_cycle()

    assert (handled, failed) == (2, 0), "stale work is not an error"
    assert agent_outcomes == {"verify": [], "check": []}
    assert not [n for n in orch.backend.names() if n.startswith("report_")], "nothing reported for stale items"


async def test_a_check_that_errors_is_rescheduled_not_retried_in_a_hot_loop(orch, monkeypatch):
    async def explode(**kw):
        raise RuntimeError("anything")

    monkeypatch.setattr(orch.monitoring_agent, "check_reappearance", lambda **kw: explode(**kw))
    e = due_exposure()
    orch.backend.due_exposures = [e]

    handled, failed = await orch._run_monitoring_cycle()

    assert (handled, failed) == (0, 1)
    assert ("report_monitoring_result", e["exposure_id"], "inconclusive") in orch.backend.calls, "pushed out a day so it cannot spin"


async def test_stale_targets_409_404_are_not_errors(orch, monkeypatch):
    req = httpx.Request("POST", "http://x")

    async def stale(**kw):
        raise httpx.HTTPStatusError("conflict", request=req, response=httpx.Response(409, request=req))

    monkeypatch.setattr(orch.monitoring_agent, "check_reappearance", lambda **kw: stale(**kw))
    orch.backend.due_exposures = [due_exposure()]
    assert await orch._run_monitoring_cycle() == (1, 0)


async def test_identity_is_not_shared_between_work_items_of_the_same_customer(orch, agent_outcomes):
    r1, r2 = due_removal(), due_removal()
    orch.backend.due_removals = [r1, r2]
    await orch._run_monitoring_cycle()
    fetched = [c[1] for c in orch.backend.calls if c[0] == "fetch_removal_identity"]
    assert fetched == [r1["removal_request_id"], r2["removal_request_id"]], "each item is authorized and audited on its own"


async def test_an_idle_cycle_does_nothing_and_reports_nothing(orch, agent_outcomes):
    assert await orch._run_monitoring_cycle() == (0, 0)
    assert orch.backend.names() == ["get_removals_due_for_verification", "get_exposures_due_for_monitoring"]
