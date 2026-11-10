"""
Discovery must hand the detection agent REAL page content. It used to pass a placeholder
string, so the detection model was never shown a page.
"""
import httpx
import pytest

import discovery.agent as da
from detection.agent import DetectionAgent
from discovery.agent import DiscoveryAgent
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
