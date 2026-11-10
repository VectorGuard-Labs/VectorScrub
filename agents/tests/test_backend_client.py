"""The client's wire format, and the errors it turns into typed exceptions."""
import json

import httpx
import pytest

from shared.backend_client import BackendClient, IdentityDenied, ScanJobLost
from shared.models import ScanJobStatus

KEY = "test-agent-key-" + "x" * 30


def client(handler):
    return BackendClient(transport=httpx.MockTransport(handler))


async def test_every_request_carries_the_agent_key():
    seen = []

    def handler(req):
        seen.append(req.headers.get("x-agent-key"))
        return httpx.Response(200, json={"attempt": 1})

    c = client(handler)
    await c.claim_scan("job-1")
    assert seen == [KEY]


async def test_claim_returns_the_attempt_or_none_when_someone_else_holds_it():
    assert await client(lambda r: httpx.Response(200, json={"attempt": 3})).claim_scan("j") == 3
    assert await client(lambda r: httpx.Response(409, text="no")).claim_scan("j") is None
    with pytest.raises(httpx.HTTPStatusError):
        await client(lambda r: httpx.Response(500)).claim_scan("j")


async def test_identity_is_requested_per_work_item_never_per_customer():
    paths = []

    def handler(req):
        paths.append((req.method, req.url.path, json.loads(req.content) if req.content else None))
        return httpx.Response(200, json={"full_name": "Jane Smith"})

    c = client(handler)
    await c.fetch_scan_identity("cust-1", "job-1", "tok")
    await c.fetch_removal_identity("req-1")
    await c.fetch_monitoring_identity("exp-1")
    assert paths == [
        ("POST", "/api/v1/internal/identity", {"customer_id": "cust-1", "job_id": "job-1", "identity_token": "tok"}),
        ("POST", "/api/v1/internal/removal-requests/req-1/identity", None),
        ("POST", "/api/v1/internal/exposures/exp-1/identity", None),
    ]
    # the blanket "identity for any customer" call no longer exists on the client
    assert not hasattr(c, "fetch_identity_for_customer")
    assert not any("customers" in p[1] for p in paths)


async def test_a_refusal_to_release_identity_is_a_typed_non_retryable_error():
    with pytest.raises(IdentityDenied):
        await client(lambda r: httpx.Response(403, text="Identity access denied")).fetch_removal_identity("r")
    with pytest.raises(httpx.HTTPStatusError):  # anything else is an ordinary HTTP error
        await client(lambda r: httpx.Response(502)).fetch_removal_identity("r")


async def test_status_updates_carry_the_attempt_and_a_409_means_the_job_is_lost():
    bodies = []

    def ok(req):
        bodies.append(json.loads(req.content))
        return httpx.Response(204)

    await client(ok).update_scan_status("job-1", ScanJobStatus.COMPLETED, attempt=2, exposures_found=3)
    assert bodies[0]["attempt"] == 2 and bodies[0]["status"] == "completed" and bodies[0]["exposures_found"] == 3

    with pytest.raises(ScanJobLost):
        await client(lambda r: httpx.Response(409)).update_scan_status("job-1", ScanJobStatus.RUNNING, attempt=1)


async def test_recording_an_exposure_for_a_job_that_is_not_running_means_the_job_is_lost():
    from shared.models import DetectionResult, ExposureSeverity, PiiCategory

    result = DetectionResult(source_name="s", source_url="https://s.example/x", pii_category=PiiCategory.FULL_NAME,
                             severity=ExposureSeverity.HIGH, pii_summary="J*** S***", confidence=0.9)
    with pytest.raises(ScanJobLost):
        await client(lambda r: httpx.Response(409)).record_exposure("c", "j", result)


async def test_monitoring_endpoints():
    seen = []

    def handler(req):
        seen.append((req.method, req.url.path, dict(req.url.params)))
        if req.method == "GET":
            return httpx.Response(200, json={"exposures": [{"exposure_id": "e"}], "requests": [{"removal_request_id": "r"}]})
        return httpx.Response(204)

    c = client(handler)
    assert await c.get_exposures_due_for_monitoring(10) == [{"exposure_id": "e"}]
    assert await c.get_removals_due_for_verification(5) == [{"removal_request_id": "r"}]
    await c.report_monitoring_result("e1", "c1", "reappeared")
    await c.report_verification_result("r1", "c1", "removed")
    assert seen[0] == ("GET", "/api/v1/internal/exposures/due-for-monitoring", {"limit": "10"})
    assert seen[1] == ("GET", "/api/v1/internal/removal-requests/due-for-verification", {"limit": "5"})
    assert seen[2][1] == "/api/v1/internal/exposures/e1/monitoring-result"
    assert seen[3][1] == "/api/v1/internal/removal-requests/r1/verification-result"
