"""
Monitoring: "removed" is claimed only on positive evidence, and never because a check failed.
"""
import httpx
import pytest

import monitoring.agent as ma
from monitoring.agent import (
    MonitoringAgent, MonitoringOutcome, PageState, VerificationOutcome, _Fetch,
)
from shared import pii_guard as g
from tests.conftest import FakeChatModel

FILLER = "Directory of local residents and businesses. " * 40
RECORD = "Jane Smith, age 41, 123 Main St Austin TX 78701, 512-555-0147, jane.smith@example.com. Relatives: Robert Smith."


def html(body: str) -> str:
    return f"<html><body>{FILLER}{body}{FILLER}</body></html>"


class FakeBackend:
    def __init__(self):
        self.verification, self.monitoring = [], []

    async def report_verification_result(self, request_id, customer_id, outcome):
        self.verification.append((request_id, customer_id, outcome))

    async def report_monitoring_result(self, exposure_id, customer_id, outcome):
        self.monitoring.append((exposure_id, customer_id, outcome))


class NoNer:
    available = True
    def ensure_loaded(self): pass
    def person_spans(self, text): return []


def make_agent(page: _Fetch, llm=None) -> tuple[MonitoringAgent, FakeBackend, FakeChatModel]:
    agent = MonitoringAgent()
    agent.backend = FakeBackend()
    llm = llm or FakeChatModel()
    agent.detection_agent._llm = llm
    agent.detection_agent._ner = NoNer()

    async def fetch(url):
        return page

    agent._fetch_url = fetch
    return agent, agent.backend, llm


async def state(page, identity, llm=None):
    agent, _, model = make_agent(page, llm)
    return (await agent._inspect_page("peoplesearch.example", "https://peoplesearch.example/x", identity)), model


# ---- page states -----------------------------------------------------------------

async def test_a_gone_page_is_absent(identity):
    (s, _), llm = await state(_Fetch("gone"), identity)
    assert s is PageState.ABSENT and llm.sent == []


async def test_a_page_that_never_mentions_the_customer_is_absent_without_calling_the_model(identity):
    (s, _), llm = await state(_Fetch("ok", html("No record here, only Jones and Brown.")), identity)
    assert s is PageState.ABSENT
    assert llm.sent == [], "decided locally: no LLM call, no page text leaves the process"


async def test_a_page_showing_the_customer_is_present(identity):
    (s, conf), llm = await state(_Fetch("ok", html(RECORD)), identity)
    assert s is PageState.PRESENT and conf == 0.9
    assert len(llm.sent) == 1


async def test_a_record_deep_in_a_long_page_is_still_found(identity):
    deep = "<html><body>" + "Directory of residents. " * 800 + RECORD + "</body></html>"
    (s, _), _ = await state(_Fetch("ok", deep), identity)
    assert s is PageState.PRESENT


async def test_what_the_model_is_sent_contains_nothing_raw(identity):
    (_, _), llm = await state(_Fetch("ok", html(RECORD)), identity)
    sent = llm.payloads.lower()
    for raw in ["jane", "smith", "robert", "555-0147", "jane.smith@", "123 main", "78701", "peoplesearch.example/x"]:
        assert raw not in sent, raw


@pytest.mark.parametrize(
    "page",
    [
        _Fetch("error"),                                                              # 403 / 5xx / timeout
        _Fetch("ok", "<html><body>Please complete the CAPTCHA to verify you are human. Jane Smith " + "x " * 60 + "</body></html>"),
        _Fetch("ok", "<html><body>tiny</body></html>"),                               # JS-rendered shell
    ],
    ids=["fetch-error", "captcha-wall", "too-thin"],
)
async def test_checks_that_could_not_be_made_are_unknown_never_absent(page, identity):
    (s, _), _ = await state(page, identity)
    assert s is PageState.UNKNOWN


async def test_an_llm_outage_is_unknown_never_absent(identity):
    (s, _), _ = await state(_Fetch("ok", html(RECORD)), identity, FakeChatModel(error=RuntimeError("provider down")))
    assert s is PageState.UNKNOWN


async def test_a_guard_refusal_is_unknown_never_absent(identity, monkeypatch):
    import detection.agent as detection

    monkeypatch.setattr(detection, "redact_for_llm", lambda t, i, person_detector=None: g.Redacted(t, g.Evidence()))
    (s, _), llm = await state(_Fetch("ok", html(RECORD)), identity)
    assert s is PageState.UNKNOWN and llm.sent == []


async def test_a_customer_without_a_surname_cannot_be_checked(identity):
    from shared.models import IdentityContext
    import uuid

    nameless = IdentityContext(customer_id=uuid.uuid4())
    (s, _), _ = await state(_Fetch("ok", html(RECORD)), nameless)
    assert s is PageState.UNKNOWN


async def test_a_weak_match_is_still_present_but_a_non_match_is_absent(identity):
    (weak, _), _ = await state(_Fetch("ok", html(RECORD)), identity, FakeChatModel(confidence=0.55))
    (none, _), _ = await state(_Fetch("ok", html(RECORD)), identity, FakeChatModel(is_match=False))
    assert weak is PageState.PRESENT and none is PageState.ABSENT


# ---- verification: what gets reported ---------------------------------------------------

@pytest.mark.parametrize(
    "page, llm, expected",
    [
        (_Fetch("gone"), None, VerificationOutcome.REMOVED),
        (_Fetch("ok", html("Only Jones and Brown.")), None, VerificationOutcome.REMOVED),
        (_Fetch("ok", html(RECORD)), None, VerificationOutcome.STILL_PRESENT),
        (_Fetch("ok", html(RECORD)), FakeChatModel(confidence=0.55), VerificationOutcome.STILL_PRESENT),
        (_Fetch("error"), None, VerificationOutcome.INCONCLUSIVE),
        (_Fetch("ok", html(RECORD)), FakeChatModel(error=RuntimeError("down")), VerificationOutcome.INCONCLUSIVE),
    ],
)
async def test_verification_outcomes(page, llm, expected, identity):
    agent, backend, _ = make_agent(page, llm)
    outcome = await agent.verify_removal("req-1", "cust-1", "peoplesearch.example", "https://p.example/x", identity)
    assert outcome is expected
    assert backend.verification == [("req-1", "cust-1", expected.value)], "reported exactly once, to the backend"


# ---- reappearance: what gets reported ----------------------------------------------------

@pytest.mark.parametrize(
    "page, llm, expected",
    [
        (_Fetch("ok", html(RECORD)), None, MonitoringOutcome.REAPPEARED),
        (_Fetch("ok", html(RECORD)), FakeChatModel(confidence=0.6), MonitoringOutcome.CLEAR),     # below the alarm bar
        (_Fetch("gone"), None, MonitoringOutcome.CLEAR),
        (_Fetch("ok", html("Only Jones and Brown.")), None, MonitoringOutcome.CLEAR),
        (_Fetch("error"), None, MonitoringOutcome.INCONCLUSIVE),
    ],
)
async def test_reappearance_outcomes(page, llm, expected, identity):
    agent, backend, _ = make_agent(page, llm)
    outcome = await agent.check_reappearance("exp-1", "cust-1", "peoplesearch.example", "https://p.example/x", identity)
    assert outcome is expected
    assert backend.monitoring == [("exp-1", "cust-1", expected.value)]


# ---- fetching and the circuit breaker ----------------------------------------------------

REAL_ASYNC_CLIENT = httpx.AsyncClient  # captured once: patching twice must not stack wrappers


def patch_http(monkeypatch, handler):
    def factory(*args, **kwargs):
        kwargs["transport"] = httpx.MockTransport(handler)
        return REAL_ASYNC_CLIENT(*args, **kwargs)

    monkeypatch.setattr(ma.httpx, "AsyncClient", factory)


@pytest.mark.parametrize("status, kind", [(200, "ok"), (404, "gone"), (410, "gone"), (403, "error"), (500, "error"), (429, "error")])
async def test_fetch_maps_http_statuses(monkeypatch, status, kind):
    patch_http(monkeypatch, lambda req: httpx.Response(status, text="body"))
    assert (await MonitoringAgent()._fetch_url("https://broker.example/p")).kind == kind


async def test_a_failing_source_trips_its_breaker_and_is_then_left_alone(monkeypatch):
    calls = []

    def handler(req):
        calls.append(req.url.host)
        return httpx.Response(503)

    patch_http(monkeypatch, handler)
    agent = MonitoringAgent()
    for _ in range(ma.FETCH_BREAKER.failure_threshold):
        assert (await agent._fetch_url("https://flaky.example/p")).kind == "error"
    assert len(calls) == ma.FETCH_BREAKER.failure_threshold

    assert (await agent._fetch_url("https://flaky.example/p")).kind == "error"
    assert len(calls) == ma.FETCH_BREAKER.failure_threshold, "circuit open: no further requests to that host"

    # a healthy host is unaffected
    patch_http(monkeypatch, lambda req: httpx.Response(200, text="fine"))
    assert (await agent._fetch_url("https://healthy.example/p")).kind == "ok"


async def test_transport_errors_never_put_the_url_in_the_logs(monkeypatch, capsys):
    def boom(req):
        raise httpx.ConnectError("could not connect to https://broker.example/people/jane-smith-1234")

    patch_http(monkeypatch, boom)
    assert (await MonitoringAgent()._fetch_url("https://broker.example/people/jane-smith-1234")).kind == "error"
    out = capsys.readouterr()
    assert "jane-smith" not in out.out + out.err, "listing URLs embed the customer's name"
