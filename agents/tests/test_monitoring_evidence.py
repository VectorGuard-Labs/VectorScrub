"""
"Removed" needs positive evidence. These are the pages that used to be read as "gone" and are
not: a redirect to the homepage, a client-rendered shell, a stub, an unsure model.
"""
import httpx
import pytest

import monitoring.agent as ma
from monitoring.agent import MonitoringOutcome, PageState, VerificationOutcome, _Fetch
from tests.conftest import FakeChatModel
from tests.test_monitoring_agent import RECORD, html, make_agent, patch_http, state

NAV = "Home Search People Phone Address Privacy Terms Contact Help About Careers Blog Press Sitemap " * 8


async def test_a_redirect_to_a_different_page_is_unknown_not_removed(identity):
    # a removed profile commonly redirects to the homepage, which mentions no one
    (s, _), llm = await state(_Fetch("ok", html("Welcome to our people directory."), moved=True), identity)
    assert s is PageState.UNKNOWN and llm.sent == []


async def test_an_enable_javascript_page_is_unknown_not_removed(identity):
    shell = f"<html><body><noscript>You need to enable JavaScript to run this app.</noscript><div>{NAV}</div></body></html>"
    (s, _), _ = await state(_Fetch("ok", shell), identity)
    assert s is PageState.UNKNOWN


async def test_a_client_rendered_shell_with_a_lot_of_markup_is_unknown(identity):
    shell = "<html><head>" + "<script>var a=1;</script>" * 2000 + "</head><body><div id='root'>" + NAV + "</div></body></html>"
    assert len(shell) > 20_000
    (s, _), _ = await state(_Fetch("ok", shell), identity)
    assert s is PageState.UNKNOWN


async def test_a_stub_page_is_too_short_to_rule_anyone_out(identity):
    stub = "<html><body>" + "Please sign in to continue to your account and view search results here. " * 3 + "</body></html>"
    (s, _), _ = await state(_Fetch("ok", stub), identity)
    assert s is PageState.UNKNOWN


async def test_a_substantial_server_rendered_page_without_the_surname_is_still_absent(identity):
    (s, c), llm = await state(_Fetch("ok", html("No record here, only Jones and Brown.")), identity)
    assert (s, c) == (PageState.ABSENT, 1.0) and llm.sent == []


@pytest.mark.parametrize("is_match, confidence, expected", [
    (False, 0.9, PageState.ABSENT),      # confident non-match
    (False, 0.7, PageState.ABSENT),
    (False, 0.5, PageState.UNKNOWN),     # unsure non-match: not evidence of removal
    (False, 0.0, PageState.UNKNOWN),
    (True, 0.4, PageState.UNKNOWN),      # unsure match: used to be called ABSENT, i.e. "removed"
    (True, 0.6, PageState.PRESENT),
    (True, 0.95, PageState.PRESENT),
])
async def test_an_unsure_model_is_never_evidence_of_removal(is_match, confidence, expected, identity):
    (s, _), _ = await state(_Fetch("ok", html(RECORD)), identity, llm=FakeChatModel(is_match=is_match, confidence=confidence))
    assert s is expected


async def test_verification_reports_inconclusive_for_a_redirected_page(identity):
    agent, backend, _ = make_agent(_Fetch("ok", html("Welcome."), moved=True))
    out = await agent.verify_removal("r1", "c1", "peoplesearch.example", "https://peoplesearch.example/x", identity)
    assert out is VerificationOutcome.INCONCLUSIVE and backend.verification == [("r1", "c1", "inconclusive")]


async def test_a_weak_match_is_inconclusive_for_reappearance_not_clear(identity):
    agent, backend, _ = make_agent(_Fetch("ok", html(RECORD)), FakeChatModel(confidence=0.6))
    out = await agent.check_reappearance("e1", "c1", "peoplesearch.example", "https://peoplesearch.example/x", identity)
    assert out is MonitoringOutcome.INCONCLUSIVE


# ---- through the real fetch path -------------------------------------------------------

async def fetch(monkeypatch, handler, url="https://www.spokeo.com/p/jane", domain="spokeo.com"):
    patch_http(monkeypatch, handler)
    return await ma.MonitoringAgent()._fetch_url(url, allowed_domain=domain)


async def test_fetch_flags_a_redirect_to_another_page(monkeypatch):
    def handler(req):
        if req.url.path == "/p/jane":
            return httpx.Response(302, headers={"location": "/"})
        return httpx.Response(200, text="<html>home</html>", headers={"content-type": "text/html"})
    r = await fetch(monkeypatch, handler)
    assert r.kind == "ok" and r.moved is True


async def test_fetch_does_not_flag_http_to_https_or_www_changes(monkeypatch):
    def handler(req):
        if req.headers["host"] == "www.spokeo.com":
            return httpx.Response(301, headers={"location": "https://spokeo.com/p/jane/"})
        return httpx.Response(200, text="<html>profile</html>", headers={"content-type": "text/html"})
    r = await fetch(monkeypatch, handler)
    assert r.kind == "ok" and r.moved is False


@pytest.mark.parametrize("location", [
    "http://169.254.169.254/latest/meta-data/", "https://internal.test/admin", "https://evil.example/x", "file:///etc/passwd",
])
async def test_fetch_never_follows_a_redirect_somewhere_it_should_not(monkeypatch, location):
    seen = []
    def handler(req):
        seen.append(str(req.url))
        return httpx.Response(302, headers={"location": location})
    r = await fetch(monkeypatch, handler)
    assert r.kind == "error" and len(seen) == 1


async def test_fetch_refuses_a_source_url_on_another_domain(monkeypatch):
    seen = []
    r = await fetch(monkeypatch, lambda req: seen.append(req) or httpx.Response(200, text="x"), url="https://internal.test/x")
    assert r.kind == "error" and seen == []


async def test_fetch_treats_an_oversized_page_as_an_error(monkeypatch, capsys):
    import shared.safe_fetch as sf
    # max_bytes is a keyword default bound at import time, so patch the function's default
    monkeypatch.setattr(sf.safe_get, "__kwdefaults__", {**sf.safe_get.__kwdefaults__, "max_bytes": 1000})
    r = await fetch(monkeypatch, lambda req: httpx.Response(200, text="x" * 5000, headers={"content-type": "text/html"}))
    assert r.kind == "error"
    assert "fetch_response_too_large" in capsys.readouterr().out, "failed for the right reason"


async def test_fetch_logs_the_host_and_reason_never_the_url(monkeypatch, capsys):
    await fetch(monkeypatch, lambda req: httpx.Response(302, headers={"location": "https://internal.test/people/jane-smith-5125550147"}),
                url="https://www.spokeo.com/people/jane-smith-5125550147")
    out = capsys.readouterr()
    text = (out.out + out.err).lower()
    assert "jane" not in text and "5125550147" not in text and "internal.test/people" not in text
    assert "fetch_blocked" in text and "spokeo.com" in text
