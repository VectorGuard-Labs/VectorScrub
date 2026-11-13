"""
The daily check that the privacy-request page each California-registry company names still opens.
It reuses the page checker for our own brokers, so these tests cover what is new: one request per
company, an entry for every company, never two requests to one site at once, the report batches,
and that a failure anywhere is survived.
"""
import asyncio
import json

import httpx
import pytest

import orchestrator.orchestrator as orch_mod
from ca_registry.health import PAGES_FILE, REPORT_CHUNK, CaRightsHealthChecker, chunks, load_pages
from orchestrator.orchestrator import Orchestrator
from shared.models import OptOutCheck, OptOutKind, OptOutStatus
from tests.fakes import FakeBackend

REAL_CLIENT = httpx.AsyncClient
PAGE = "<html><body><h1>Privacy requests</h1>" + "Submit your request using the form below. " * 20 + "</body></html>"


def patch_http(monkeypatch, handler):
    monkeypatch.setattr("removal.health.httpx.AsyncClient", lambda *a, **k: REAL_CLIENT(transport=httpx.MockTransport(handler)))
    monkeypatch.setattr("ca_registry.health.httpx.AsyncClient", lambda *a, **k: REAL_CLIENT(transport=httpx.MockTransport(handler)))


def ok(req):
    return httpx.Response(200, text=PAGE, headers={"content-type": "text/html"})


# ---- the committed list ------------------------------------------------------------------------

def test_the_committed_list_has_every_company_once():
    pages = load_pages()
    assert len(pages) == 625
    assert len({i for i, _ in pages}) == 625
    assert all(u.startswith(("http://", "https://")) for _, u in pages)


def test_a_missing_or_damaged_file_gives_an_empty_list_not_a_crash(tmp_path):
    assert load_pages(tmp_path / "nope.json") == []
    bad = tmp_path / "bad.json"
    bad.write_text("{not json", encoding="utf-8")
    assert load_pages(bad) == []
    mixed = tmp_path / "mixed.json"
    mixed.write_text(json.dumps([
        {"id": "1", "url": "https://a.com/x"}, {"id": "../2", "url": "https://b.com"},
        {"id": "3", "url": "javascript:alert(1)"}, {"id": "4"}, "junk", {"id": 5, "url": "https://c.com"},
    ]), encoding="utf-8")
    assert load_pages(mixed) == [("1", "https://a.com/x")], "only a plain id and an http(s) page survive"


# ---- checking ------------------------------------------------------------------------------------

async def test_every_company_gets_an_entry_marked_as_a_registry_page_in_input_order(monkeypatch):
    def handler(req):
        host = req.headers["host"]
        if "dead" in host:
            return httpx.Response(404)
        if "wall" in host:
            return httpx.Response(403)
        if "boom" in host:
            raise httpx.ConnectError("unreachable")
        return ok(req)

    patch_http(monkeypatch, handler)
    pages = [("10", "https://www.fine.com/privacy"), ("11", "https://dead.com/x"), ("12", "https://wall.com"),
             ("13", "https://boom.com/p"), ("14", "https://fine2.com")]
    results = await CaRightsHealthChecker(concurrency=3, delay_seconds=0).check_all(pages)
    assert [(r.source, r.status.value, r.http_status) for r in results] == [
        ("10", "reachable", 200), ("11", "not_found", 404), ("12", "blocked", 403),
        ("13", "unavailable", None), ("14", "reachable", 200)]
    assert all(r.kind is OptOutKind.CA_RIGHTS for r in results)


async def test_one_request_per_company_and_the_report_holds_ids_not_urls(monkeypatch):
    seen = []
    patch_http(monkeypatch, lambda req: (seen.append(str(req.url)), ok(req))[1])
    results = await CaRightsHealthChecker(delay_seconds=0).check_all([("1", "https://a.com/p"), ("2", "https://b.com/q")])
    assert len(seen) == 2
    for r in results:
        assert set(r.model_dump(mode="json")) == {"kind", "source", "status", "http_status"}
        assert "/" not in r.source and "http" not in r.source


async def test_requests_to_the_same_site_are_never_concurrent_and_total_concurrency_is_capped(monkeypatch):
    live = {"all": 0, "max_all": 0, "per_host": {}, "max_host": 0}

    async def handler(req):
        host = req.headers["host"]
        live["all"] += 1
        live["per_host"][host] = live["per_host"].get(host, 0) + 1
        live["max_all"] = max(live["max_all"], live["all"])
        live["max_host"] = max(live["max_host"], live["per_host"][host])
        await asyncio.sleep(0.01)
        live["all"] -= 1
        live["per_host"][host] -= 1
        return httpx.Response(200, text=PAGE, headers={"content-type": "text/html"})

    patch_http(monkeypatch, handler)
    pages = [(str(i), f"https://shared.com/{i}") for i in range(6)] + [(str(10 + i), f"https://site{i}.com") for i in range(10)]
    results = await CaRightsHealthChecker(concurrency=4, delay_seconds=0).check_all(pages)
    assert len(results) == 16 and all(r.status is OptOutStatus.REACHABLE for r in results)
    assert live["max_host"] == 1, "never two requests to one site at once (www. counts as the same site)"
    assert 1 < live["max_all"] <= 4, f"a few sites at a time, never more than 4: {live['max_all']}"


async def test_www_and_bare_hosts_share_a_lock(monkeypatch):
    live = {"n": 0, "max": 0}

    async def handler(req):
        live["n"] += 1
        live["max"] = max(live["max"], live["n"])
        await asyncio.sleep(0.01)
        live["n"] -= 1
        return httpx.Response(200, text=PAGE, headers={"content-type": "text/html"})

    patch_http(monkeypatch, handler)
    await CaRightsHealthChecker(concurrency=4, delay_seconds=0).check_all(
        [("1", "https://www.same.com/a"), ("2", "https://same.com/b"), ("3", "https://WWW.SAME.COM/c")])
    assert live["max"] == 1


async def test_an_empty_list_is_fine(monkeypatch):
    patch_http(monkeypatch, ok)
    assert await CaRightsHealthChecker(delay_seconds=0).check_all([]) == []


def test_reports_are_cut_into_batches_the_backend_accepts():
    assert REPORT_CHUNK == 200
    batches = list(chunks(list(range(625))))
    assert [len(b) for b in batches] == [200, 200, 200, 25]
    assert [x for b in batches for x in b] == list(range(625))
    assert list(chunks([])) == []


# ---- the loop ----------------------------------------------------------------------------------------

@pytest.fixture
def orch(monkeypatch):
    o = Orchestrator()
    o.backend = FakeBackend()
    monkeypatch.setattr(o.cfg, "ca_rights_health_startup_delay_seconds", 0)
    monkeypatch.setattr(o.cfg, "ca_rights_health_interval_seconds", 0)
    return o


class FakeChecker:
    runs = 0
    error: Exception | None = None
    count = 450

    def __init__(self): pass

    async def check_all(self):
        type(self).runs += 1
        if type(self).error:
            raise type(self).error
        return [OptOutCheck(kind=OptOutKind.CA_RIGHTS, source=str(i), status=OptOutStatus.REACHABLE, http_status=200)
                for i in range(type(self).count)]


async def run_for(orch, cycles=2):
    task = asyncio.create_task(orch.run_ca_rights_health_checks())
    for _ in range(80):
        await asyncio.sleep(0)
        if FakeChecker.runs >= cycles:
            break
    task.cancel()
    await asyncio.gather(task, return_exceptions=True)


async def test_the_loop_reports_in_batches_each_cycle(orch, monkeypatch):
    FakeChecker.runs, FakeChecker.error = 0, None
    monkeypatch.setattr(orch_mod, "CaRightsHealthChecker", FakeChecker)
    await run_for(orch)
    sizes = [len(r) for r in orch.backend.optout_reports]
    assert sizes[:3] == [200, 200, 50], "450 checks go as 200 + 200 + 50"
    assert all(c.kind is OptOutKind.CA_RIGHTS for r in orch.backend.optout_reports for c in r)


async def test_the_loop_survives_a_failing_check_and_a_failing_report_without_leaking_text(orch, monkeypatch):
    import structlog.testing
    monkeypatch.setattr(orch_mod, "CaRightsHealthChecker", FakeChecker)
    FakeChecker.runs, FakeChecker.error = 0, RuntimeError("password=hunter2 https://x.example/Jane-Smith")
    with structlog.testing.capture_logs() as logs:
        await run_for(orch, cycles=3)
    assert FakeChecker.runs >= 3
    errors = [l for l in logs if l["event"] == "ca_rights_health_error"]
    assert errors and all(l["error_type"] == "RuntimeError" for l in errors)
    assert "hunter2" not in str(logs) and "Jane-Smith" not in str(logs)

    FakeChecker.runs, FakeChecker.error = 0, None
    orch.backend.optout_report_error = httpx.ConnectError("backend down")
    await run_for(orch, cycles=3)
    assert FakeChecker.runs >= 3


async def test_it_can_be_switched_off(orch, monkeypatch):
    FakeChecker.runs, FakeChecker.error = 0, None
    monkeypatch.setattr(orch_mod, "CaRightsHealthChecker", FakeChecker)
    monkeypatch.setattr(orch.cfg, "ca_rights_health_enabled", False)
    await asyncio.wait_for(orch.run_ca_rights_health_checks(), timeout=1)
    assert FakeChecker.runs == 0
