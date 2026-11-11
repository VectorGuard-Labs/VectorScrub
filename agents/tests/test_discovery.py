"""
Discovery must hand the detection agent REAL page content. It used to pass a placeholder
string, so the detection model was never shown a page.
"""
import httpx
import pytest

import discovery.agent as da
from detection.agent import DetectionAgent
from discovery.agent import DiscoveryAgent
from shared.models import SourceOutcome
from tests.conftest import FakeChatModel
from tests.test_monitoring_agent import FILLER, RECORD, NoNer

REAL_CLIENT = httpx.AsyncClient


def page(body: str) -> str:
    return f"<html><head><script>var x=1</script></head><body>{FILLER}{body}{FILLER}</body></html>"


def client_for(handler) -> httpx.AsyncClient:
    return REAL_CLIENT(transport=httpx.MockTransport(handler))


async def fetch(handler, identity, url="https://www.spokeo.com/Jane-Smith"):
    async with client_for(handler) as c:
        return await DiscoveryAgent(tier="premium")._fetch_source(c, "spokeo.com", url, identity)


async def test_a_matching_page_yields_a_candidate_carrying_the_real_page_text(identity):
    results = await fetch(lambda r: httpx.Response(200, text=page(RECORD)), identity)
    assert len(results) == 1
    snippet = results[0].raw_text_snippet
    assert "[candidate match" not in snippet, "the old placeholder is gone"
    assert "Jane Smith, age 41" in snippet, "the record the model must judge is in the snippet"
    assert "var x=1" not in snippet, "scripts are stripped"
    assert len(snippet) <= da.CANDIDATE_TEXT_CHARS


async def test_the_snippet_is_centred_on_the_record_even_deep_in_a_long_page(identity):
    deep = "<html><body>" + "Directory of residents. " * 600 + RECORD + "</body></html>"
    results = await fetch(lambda r: httpx.Response(200, text=deep), identity)
    assert results and "Jane Smith, age 41" in results[0].raw_text_snippet


@pytest.mark.parametrize(
    "response",
    [
        httpx.Response(200, text=page("Only Jones and Brown live here.")),   # does not mention her
        httpx.Response(200, text="<html><body>tiny</body></html>"),           # JS shell
        httpx.Response(200, text="<html><body>Please complete the CAPTCHA to verify you are human. Jane Smith " + "x " * 60 + "</body></html>"),
        httpx.Response(404), httpx.Response(403),
    ],
    ids=["no-match", "too-thin", "captcha", "404", "403"],
)
async def test_pages_that_cannot_support_a_match_yield_nothing(response, identity):
    assert await fetch(lambda r: response, identity) == []


async def test_what_the_detection_model_receives_is_the_redacted_page_not_a_stub(identity):
    """End to end through the real guard: real content in, nothing raw out."""
    results = await fetch(lambda r: httpx.Response(200, text=page(RECORD)), identity)
    llm = FakeChatModel()
    detection = DetectionAgent()
    detection._llm, detection._ner = llm, NoNer()

    confirmed = await detection.run(results, identity)

    assert len(confirmed) == 1
    sent = llm.payloads
    excerpt = sent.split("Redacted page excerpt:\n", 1)[1].rsplit("\n\nAnalyze", 1)[0]
    assert "[CUSTOMER_NAME]" in excerpt and "[CUSTOMER_ADDRESS]" in excerpt, "the model saw the page's structure"
    for raw in ["jane", "smith", "robert", "555-0147", "123 main", "78701", "spokeo.com/jane"]:
        assert raw not in sent.lower(), raw


async def test_a_struggling_source_is_left_alone_after_repeated_failures(identity):
    calls = []

    def handler(req):
        calls.append(1)
        return httpx.Response(503)

    for _ in range(da.SOURCE_BREAKER.failure_threshold):
        assert await fetch(handler, identity) == []
    assert len(calls) == da.SOURCE_BREAKER.failure_threshold
    assert await fetch(handler, identity) == []
    assert len(calls) == da.SOURCE_BREAKER.failure_threshold, "breaker open: no more requests to that host"


async def test_transport_errors_do_not_log_the_listing_url(identity, capsys):
    def boom(req):
        raise httpx.ConnectError("cannot reach https://www.spokeo.com/Jane-Smith/Austin")

    assert await fetch(boom, identity, url="https://www.spokeo.com/Jane-Smith/Austin") == []
    out = capsys.readouterr()
    assert "Jane-Smith" not in out.out + out.err


# ---- what happened at each source (scan coverage) ---------------------------------------------

async def outcome(handler, identity) -> SourceOutcome:
    async with client_for(handler) as c:
        agent = DiscoveryAgent(tier="premium")
        await agent._fetch_source(c, "spokeo.com", "https://www.spokeo.com/Jane-Smith", identity)
        return agent._outcomes["spokeo.com"]


@pytest.mark.parametrize(
    "response, expected",
    [
        (httpx.Response(200, text=page(RECORD)), SourceOutcome.CHECKED),
        (httpx.Response(200, text=page("Only Jones and Brown live here.")), SourceOutcome.CHECKED),  # read; just not listed
        (httpx.Response(403), SourceOutcome.BLOCKED),
        (httpx.Response(401), SourceOutcome.BLOCKED),
        (httpx.Response(451), SourceOutcome.BLOCKED),
        (httpx.Response(200, text="<html><body>Please complete the CAPTCHA to verify you are human. " + "x " * 60 + "</body></html>"), SourceOutcome.BLOCKED),
        (httpx.Response(404), SourceOutcome.NOT_FOUND),
        (httpx.Response(410), SourceOutcome.NOT_FOUND),
        (httpx.Response(200, text="<html><body>tiny</body></html>"), SourceOutcome.UNREADABLE),
        (httpx.Response(503), SourceOutcome.UNAVAILABLE),
        (httpx.Response(429), SourceOutcome.UNAVAILABLE),
        (httpx.Response(418), SourceOutcome.UNAVAILABLE),
    ],
    ids=["match", "read-no-match", "403", "401", "451", "captcha", "404", "410", "js-shell", "503", "429", "other-status"],
)
async def test_each_source_is_classified_by_what_happened_to_it(response, expected, identity):
    assert await outcome(lambda r: response, identity) is expected


async def test_a_network_failure_is_unavailable(identity):
    def boom(req):
        raise httpx.ConnectError("unreachable")
    assert await outcome(boom, identity) is SourceOutcome.UNAVAILABLE


async def test_a_source_nobody_reported_on_counts_as_not_checked_and_every_source_is_listed(identity, monkeypatch):
    """A crash in one source, or a path that records nothing, must never read as 'checked'."""
    async def fake(self, client, name, url, ident):
        if name == "spokeo.com":
            self._outcomes[name] = SourceOutcome.CHECKED
        if name == "whitepages.com":
            raise ValueError("boom")
        return []                       # truepeoplesearch records nothing (peoplefinder is never fetched)

    monkeypatch.setattr(DiscoveryAgent, "_fetch_source", fake)
    report = await DiscoveryAgent(tier="free").run_report(identity)

    assert list(report.outcomes) == ["spokeo.com", "whitepages.com", "peoplefinder.com", "truepeoplesearch.com"]
    assert report.outcomes["spokeo.com"] is SourceOutcome.CHECKED
    assert all(report.outcomes[n] is SourceOutcome.UNAVAILABLE for n in ("whitepages.com", "truepeoplesearch.com"))
    assert report.outcomes["peoplefinder.com"] is SourceOutcome.NOT_SUPPORTED, "searchable only by hand"
    assert (report.checked, report.total) == (1, 4)
    assert report.coverage()[0] == {"source": "spokeo.com", "outcome": "checked"}


async def test_run_still_returns_just_the_candidates(identity, monkeypatch):
    async def fake(self, client, name, url, ident):
        self._outcomes[name] = SourceOutcome.CHECKED
        return []
    monkeypatch.setattr(DiscoveryAgent, "_fetch_source", fake)
    assert await DiscoveryAgent(tier="free").run(identity) == []


# ---- sources that can only be searched by hand --------------------------------------------------

MANUAL = {"peoplefinder.com", "intelius.com", "peoplelooker.com", "ownerly.com", "instantcheckmate.com"}


def test_the_sources_without_a_name_addressable_search_are_flagged_manual():
    flagged = {s["name"] for s in da.DISCOVERY_SOURCES if s.get("manual")}
    assert flagged == MANUAL


async def test_manual_sources_are_never_fetched_and_are_reported_as_needing_a_manual_search(identity, monkeypatch):
    fetched = []

    async def fake(self, client, name, url, ident):
        fetched.append(name)
        self._outcomes[name] = SourceOutcome.CHECKED
        return []

    monkeypatch.setattr(DiscoveryAgent, "_fetch_source", fake)
    report = await DiscoveryAgent(tier="premium").run_report(identity)

    assert not (set(fetched) & MANUAL), "no requests are wasted on pages known not to work"
    assert len(report.outcomes) == 19, "they still appear in the report"
    assert {n for n, o in report.outcomes.items() if o is SourceOutcome.NOT_SUPPORTED} == MANUAL
    assert report.checked == 14 and report.total == 19


async def test_a_free_scan_reports_its_one_manual_source(identity, monkeypatch):
    async def fake(self, client, name, url, ident):
        self._outcomes[name] = SourceOutcome.CHECKED
        return []
    monkeypatch.setattr(DiscoveryAgent, "_fetch_source", fake)
    report = await DiscoveryAgent(tier="free").run_report(identity)
    assert report.outcomes["peoplefinder.com"] is SourceOutcome.NOT_SUPPORTED
    assert (report.checked, report.total) == (3, 4)
