"""
The nightly check that brokers' public opt-out pages still load.

It fetches only public pages, submits nothing and carries no customer data. These tests pin
what each response means, that the check stays polite and on-site, and that a failure of the
check never takes anything else down.
"""
import asyncio

import httpx
import pytest

import orchestrator.orchestrator as orch_mod
from orchestrator.orchestrator import Orchestrator
from removal.agent import OPT_OUT_REGISTRY
from removal.health import (
    CAPTCHA_WIDGET_MARKERS, HEADERS, OptOutHealthChecker, _allowed_domain, classify,
)
from shared.backend_client import BackendClient
from shared.models import OptOutCheck, OptOutStatus, RemovalProcedure
from tests.fakes import FakeBackend

REAL_CLIENT = httpx.AsyncClient
FORM = "<html><body><h1>Opt out of our site</h1><form>" + "Enter your details to remove your listing. " * 20 + "</form></body></html>"


# ---- what a response means ------------------------------------------------------------------

@pytest.mark.parametrize(
    "status, html, expected",
    [
        (200, FORM, OptOutStatus.REACHABLE),
        (200, "<html><body>tiny js shell</body></html>", OptOutStatus.REACHABLE),   # a browser renders it
        (200, FORM.replace("<form>", '<form><div class="g-recaptcha" data-sitekey="k"></div>'), OptOutStatus.CAPTCHA),
        (200, FORM.replace("<form>", '<form><div class="h-captcha"></div>'), OptOutStatus.CAPTCHA),
        (200, FORM.replace("<form>", '<form><div class="cf-turnstile"></div>'), OptOutStatus.CAPTCHA),
        (200, "<html><body>Please verify you are human to continue. " + "x " * 40 + "</body></html>", OptOutStatus.BLOCKED),
        (403, "", OptOutStatus.BLOCKED),
        (401, "", OptOutStatus.BLOCKED),
        (451, "", OptOutStatus.BLOCKED),
        (404, "", OptOutStatus.NOT_FOUND),
        (410, "", OptOutStatus.NOT_FOUND),
        (429, "", OptOutStatus.UNAVAILABLE),
        (500, "", OptOutStatus.UNAVAILABLE),
        (503, "", OptOutStatus.UNAVAILABLE),
        (418, "", OptOutStatus.UNAVAILABLE),
    ],
    ids=["ok", "js-shell", "recaptcha", "hcaptcha", "turnstile", "200-bot-wall", "403", "401", "451", "404", "410", "429", "500", "503", "other"],
)
def test_each_response_means_one_thing(status, html, expected):
    assert classify(status, html) is expected


def test_an_ordinary_page_that_merely_mentions_captcha_in_its_text_is_not_flagged():
    # only the widget markers count, not the word in the prose of a long page
    page = "<html><body>" + "Our form may ask you to solve a puzzle to prove you are a person. " * 80 + "</body></html>"
    assert classify(200, page) is OptOutStatus.REACHABLE


def test_the_user_agent_says_what_it_is():
    assert "VectorScrubBot" in HEADERS["User-Agent"]


@pytest.mark.parametrize("url, domain", [
    ("https://www.spokeo.com/optout", "spokeo.com"),
    ("https://suppression.peopleconnect.us/login", "suppression.peopleconnect.us"),
    ("https://radaris.com/control/privacy", "radaris.com"),
])
def test_redirects_may_stay_on_the_site_not_leave_it(url, domain):
    assert _allowed_domain(url) == domain


# ---- the checker -----------------------------------------------------------------------------

def checker(handler, registry, monkeypatch):
    monkeypatch.setattr("removal.health.httpx.AsyncClient", lambda *a, **k: REAL_CLIENT(transport=httpx.MockTransport(handler)))
    return OptOutHealthChecker(delay_seconds=0, timeout_seconds=5), registry


def proc(url, kind="web_form", email=None):
    return RemovalProcedure(source_name="x", procedure_type=kind, opt_out_url=url, opt_out_email=email)


async def test_every_web_form_broker_is_checked_and_email_brokers_are_left_out(monkeypatch):
    seen = []

    def handler(req):
        seen.append(str(req.url))
        return httpx.Response(200, text=FORM, headers={"content-type": "text/html"})

    registry = {
        "a.com": proc("https://www.a.com/optout"),
        "b.com": proc("https://b.com/remove"),
        "mail.com": proc(None, kind="email", email="privacy@mail.com"),
    }
    c, reg = checker(handler, registry, monkeypatch)
    results = await c.check_all(reg)
    assert [(r.source, r.status, r.http_status) for r in results] == [
        ("a.com", OptOutStatus.REACHABLE, 200), ("b.com", OptOutStatus.REACHABLE, 200)]
    assert len(seen) == 2, "nothing is fetched for an email broker"


async def test_one_site_failing_does_not_stop_the_others_and_each_failure_is_classified(monkeypatch):
    def handler(req):
        host = req.headers["host"]
        if "dead" in host:
            return httpx.Response(404)
        if "wall" in host:
            return httpx.Response(403)
        if "boom" in host:
            raise httpx.ConnectError("unreachable")
        return httpx.Response(200, text=FORM, headers={"content-type": "text/html"})

    registry = {n: proc(f"https://www.{n}/optout") for n in ("boom.com", "dead.com", "fine.com", "wall.com")}
    c, reg = checker(handler, registry, monkeypatch)
    got = {r.source: (r.status, r.http_status) for r in await c.check_all(reg)}
    assert got == {
        "boom.com": (OptOutStatus.UNAVAILABLE, None),
        "dead.com": (OptOutStatus.NOT_FOUND, 404),
        "fine.com": (OptOutStatus.REACHABLE, 200),
        "wall.com": (OptOutStatus.BLOCKED, 403),
    }


async def test_a_redirect_to_another_companys_site_is_not_followed_and_is_reported_unavailable(monkeypatch):
    def handler(req):
        if req.headers["host"] == "www.a.com":
            return httpx.Response(302, headers={"location": "https://evil.example/phish"})
        return httpx.Response(200, text=FORM, headers={"content-type": "text/html"})

    c, reg = checker(handler, {"a.com": proc("https://www.a.com/optout")}, monkeypatch)
    [r] = await c.check_all(reg)
    assert r.status is OptOutStatus.UNAVAILABLE


async def test_a_redirect_within_the_same_site_is_followed(monkeypatch):
    def handler(req):
        if req.headers["host"] == "www.a.com":
            return httpx.Response(301, headers={"location": "https://a.com/optout/"})
        return httpx.Response(200, text=FORM, headers={"content-type": "text/html"})

    c, reg = checker(handler, {"a.com": proc("https://www.a.com/optout")}, monkeypatch)
    [r] = await c.check_all(reg)
    assert (r.status, r.http_status) == (OptOutStatus.REACHABLE, 200)


async def test_requests_are_spaced_out_and_carry_the_bot_user_agent(monkeypatch):
    sleeps, agents = [], []
    real_sleep = asyncio.sleep

    async def fake_sleep(s):
        sleeps.append(s)
        await real_sleep(0)

    def handler(req):
        agents.append(req.headers["user-agent"])
        return httpx.Response(200, text=FORM, headers={"content-type": "text/html"})

    monkeypatch.setattr("removal.health.asyncio.sleep", fake_sleep)
    registry = {n: proc(f"https://www.{n}/o") for n in ("a.com", "b.com", "c.com")}
    monkeypatch.setattr("removal.health.httpx.AsyncClient", lambda *a, **k: REAL_CLIENT(transport=httpx.MockTransport(handler)))
    await OptOutHealthChecker(delay_seconds=2.0, timeout_seconds=5).check_all(registry)
    assert sleeps == [2.0, 2.0], "a pause between sites, none before the first"
    assert all("VectorScrubBot" in a for a in agents)


async def test_the_real_registry_is_covered_and_reports_only_site_names(monkeypatch):
    c, _ = checker(lambda req: httpx.Response(200, text=FORM, headers={"content-type": "text/html"}), {}, monkeypatch)
    results = await c.check_all()
    web = {n for n, p in OPT_OUT_REGISTRY.items() if p.opt_out_url}
    assert {r.source for r in results} == web
    for r in results:
        assert "/" not in r.source and "http" not in r.source
        assert set(r.model_dump()) == {"source", "status", "http_status"}, "nothing else leaves the process"


# ---- reporting and the worker loop -----------------------------------------------------------

async def test_the_report_carries_only_names_and_statuses(monkeypatch):
    bodies = []

    def handler(req):
        bodies.append((req.method, req.url.path, req.headers.get("x-agent-key"), req.content))
        return httpx.Response(204)

    client = BackendClient(transport=httpx.MockTransport(handler))
    await client.report_optout_health([OptOutCheck(source="spokeo.com", status=OptOutStatus.CAPTCHA, http_status=200)])
    method, path, key, body = bodies[0]
    assert (method, path) == ("PUT", "/api/v1/internal/optout-health")
    assert key and key.startswith("test-agent-key")
    import json
    assert json.loads(body) == {"checks": [{"source": "spokeo.com", "status": "captcha", "http_status": 200}]}


@pytest.fixture
def orch(monkeypatch):
    o = Orchestrator()
    o.backend = FakeBackend()
    monkeypatch.setattr(o.cfg, "optout_health_startup_delay_seconds", 0)
    monkeypatch.setattr(o.cfg, "optout_health_interval_seconds", 0)
    return o


class FakeChecker:
    runs = 0
    error: Exception | None = None

    def __init__(self): pass

    async def check_all(self):
        type(self).runs += 1
        if type(self).error:
            raise type(self).error
        return [OptOutCheck(source="spokeo.com", status=OptOutStatus.REACHABLE, http_status=200)]


async def run_for_a_few_cycles(orch, cycles=3):
    task = asyncio.create_task(orch.run_optout_health_checks())
    for _ in range(50):
        await asyncio.sleep(0)
        if FakeChecker.runs >= cycles:
            break
    task.cancel()
    await asyncio.gather(task, return_exceptions=True)


async def test_the_loop_checks_and_reports_every_cycle(orch, monkeypatch):
    FakeChecker.runs, FakeChecker.error = 0, None
    monkeypatch.setattr(orch_mod, "OptOutHealthChecker", FakeChecker)
    await run_for_a_few_cycles(orch)
    assert FakeChecker.runs >= 3 and len(orch.backend.optout_reports) >= 3
    assert orch.backend.optout_reports[0][0].source == "spokeo.com"


async def test_a_failing_check_or_report_is_survived_and_tried_again(orch, monkeypatch):
    monkeypatch.setattr(orch_mod, "OptOutHealthChecker", FakeChecker)
    FakeChecker.runs, FakeChecker.error = 0, RuntimeError("GET https://x.example/Jane-Smith failed")
    await run_for_a_few_cycles(orch)
    assert FakeChecker.runs >= 3, "an error does not end the loop"

    FakeChecker.runs, FakeChecker.error = 0, None
    orch.backend.optout_report_error = httpx.ConnectError("backend down")
    await run_for_a_few_cycles(orch)
    assert FakeChecker.runs >= 3, "the backend being down does not end it either"


async def test_it_can_be_switched_off(orch, monkeypatch):
    FakeChecker.runs, FakeChecker.error = 0, None
    monkeypatch.setattr(orch_mod, "OptOutHealthChecker", FakeChecker)
    monkeypatch.setattr(orch.cfg, "optout_health_enabled", False)
    await asyncio.wait_for(orch.run_optout_health_checks(), timeout=1)   # returns at once
    assert FakeChecker.runs == 0


async def test_the_log_never_contains_the_error_text(orch, monkeypatch, capsys):
    import structlog.testing
    monkeypatch.setattr(orch_mod, "OptOutHealthChecker", FakeChecker)
    FakeChecker.runs, FakeChecker.error = 0, RuntimeError("password=hunter2 https://x.example/Jane-Smith")
    with structlog.testing.capture_logs() as logs:
        await run_for_a_few_cycles(orch, cycles=1)
    errors = [l for l in logs if l["event"] == "optout_health_error"]
    assert errors and all(l["error_type"] == "RuntimeError" for l in errors)
    assert "hunter2" not in str(logs) and "Jane-Smith" not in str(logs)
