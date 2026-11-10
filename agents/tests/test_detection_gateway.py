"""
The detection agent is the ONLY place a model is called. These tests look at exactly what
the model is sent, and at what happens when the guard refuses.
"""
import pytest

from detection.agent import DetectionAgent
from shared import pii_guard as g
from shared.models import DiscoveryResult
from shared.pii_ner import NerUnavailableError, PersonDetector
from tests.conftest import FakeChatModel
from tests.test_pii_guard import PAGE

SOURCE_URL = "https://peoplesearch.example/people/jane-smith-5125550147/austin-tx"


def candidate(text: str = PAGE) -> DiscoveryResult:
    return DiscoveryResult(source_name="peoplesearch.example", source_url=SOURCE_URL, raw_text_snippet=text, confidence=0.8)


class FakeDetector:
    """Marks nothing: lets gateway tests run without the spaCy model."""

    available = True

    def ensure_loaded(self):
        pass

    def person_spans(self, text):
        return []


@pytest.fixture
def agent(fake_llm):
    a = DetectionAgent()
    a._llm = fake_llm
    a._ner = FakeDetector()
    return a


async def test_the_model_sees_no_customer_data_url_or_third_party_name(agent, fake_llm, identity):
    result = await agent.classify(candidate(), identity)

    assert result is not None and result.severity.value == "high"
    sent = fake_llm.payloads.lower()
    for raw in ["jane", "smith", "marie", "555", "example.com", "1984", "78701", "robert", "carol", "garcia", "linda"]:
        assert raw not in sent, raw
    assert "jane-smith" not in sent and "https://" not in sent and "/people/" not in sent, "the URL is never sent"


async def test_no_identity_hints_are_sent_not_even_masked_ones(agent, fake_llm, identity):
    await agent.classify(candidate(), identity)
    sent = fake_llm.payloads
    assert "J***" not in sent and "***-***" not in sent and "name_token" not in sent


async def test_the_excerpt_the_model_gets_is_vocabulary_only(agent, fake_llm, identity):
    await agent.classify(candidate(), identity)
    sent = fake_llm.payloads
    excerpt = sent.split("Redacted page excerpt:\n", 1)[1].rsplit("\n\nAnalyze", 1)[0]
    assert g.find_disallowed(excerpt) == 0
    assert "[CUSTOMER_NAME]" in excerpt


async def test_the_dashboard_summary_is_built_locally_and_masked(agent, identity):
    result = await agent.classify(candidate(), identity)
    assert result.pii_summary.startswith("J*** M*** S***")
    assert "jane" not in result.pii_summary.lower()


async def test_a_redaction_bug_blocks_the_call_instead_of_leaking(agent, fake_llm, identity, monkeypatch):
    import detection.agent as module

    monkeypatch.setattr(module, "redact_for_llm", lambda text, ident, person_detector=None: g.Redacted(text, g.Evidence()))
    assert await agent.classify(candidate(), identity) is None
    assert fake_llm.sent == [], "the model must not have been called"
    with pytest.raises(g.OutboundPiiError) as exc:
        await agent.assess_page(candidate(), identity)
    assert fake_llm.sent == []
    assert "jane" not in str(exc.value).lower(), "the error names categories, never values"


async def test_a_vocabulary_filter_bug_blocks_the_call(agent, fake_llm, identity, monkeypatch):
    import detection.agent as module

    monkeypatch.setattr(
        module, "redact_for_llm",
        lambda text, ident, person_detector=None: g.Redacted("Relatives: Zbigniew Kowalczyk", g.Evidence()),
    )
    assert await agent.classify(candidate(), identity) is None
    assert fake_llm.sent == []


async def test_ner_required_but_missing_blocks_every_call(fake_llm, identity):
    agent = DetectionAgent()
    agent._llm = fake_llm
    agent._ner = PersonDetector("en_core_web_this_model_does_not_exist")
    assert agent.cfg.pii_ner_required is True, "fail closed is the default"

    assert await agent.classify(candidate(), identity) is None
    with pytest.raises(NerUnavailableError):
        await agent.assess_page(candidate(), identity)
    assert fake_llm.sent == []


async def test_ner_optional_and_missing_degrades_to_the_vocabulary_filter(fake_llm, identity, monkeypatch):
    agent = DetectionAgent()
    agent._llm = fake_llm
    agent._ner = PersonDetector("en_core_web_this_model_does_not_exist")
    monkeypatch.setattr(agent.cfg, "pii_ner_required", False)

    result = await agent.classify(candidate(), identity)
    assert result is not None
    sent = fake_llm.payloads.lower()
    assert "robert" not in sent and "jane" not in sent, "still clean without NER"


async def test_a_customer_surnamed_page_is_not_blocked_by_our_own_wording(fake_llm):
    """Guard only inspects page-derived text, so the word 'page' in the prompt is harmless."""
    import uuid
    from shared.models import IdentityContext

    agent = DetectionAgent()
    agent._llm = fake_llm
    agent._ner = FakeDetector()
    ident = IdentityContext(customer_id=uuid.uuid4(), full_name="Larry Page")
    result = await agent.classify(candidate("Larry Page of Palo Alto. See the page for more. " * 3), ident)
    assert result is not None and "larry" not in fake_llm.payloads.lower()


async def test_llm_failures_are_none_for_discovery_but_raise_for_monitoring(identity):
    agent = DetectionAgent()
    agent._llm = FakeChatModel(error=RuntimeError("provider down"))
    agent._ner = FakeDetector()
    assert await agent.classify(candidate(), identity) is None
    with pytest.raises(RuntimeError):
        await agent.assess_page(candidate(), identity)


async def test_low_confidence_and_non_matches_are_not_exposures(identity):
    for model in (FakeChatModel(is_match=False), FakeChatModel(confidence=0.3)):
        agent = DetectionAgent()
        agent._llm = model
        agent._ner = FakeDetector()
        assert await agent.classify(candidate(), identity) is None


# ---------------------------------------------------------------------------
# "Could not check" must never look like "checked and clean"
# ---------------------------------------------------------------------------

def _agent(model):
    a = DetectionAgent()
    a._llm = model
    a._ner = FakeDetector()
    return a


async def test_run_report_counts_pages_that_could_not_be_checked(identity):
    report = await _agent(FakeChatModel(error=RuntimeError("provider down"))).run_report(
        [candidate(), candidate()], identity
    )
    assert (report.checked, report.failed, report.confirmed) == (2, 2, [])


async def test_a_clean_page_is_checked_not_failed(identity):
    report = await _agent(FakeChatModel(is_match=False)).run_report([candidate(), candidate()], identity)
    assert (report.checked, report.failed, report.confirmed) == (2, 0, [])


async def test_a_guard_block_counts_as_a_failed_check(identity, monkeypatch):
    import detection.agent as module

    monkeypatch.setattr(module, "redact_for_llm", lambda text, ident, person_detector=None: g.Redacted(text, g.Evidence()))
    llm = FakeChatModel()
    report = await _agent(llm).run_report([candidate()], identity)
    assert (report.checked, report.failed) == (1, 1) and llm.sent == []


async def test_a_malformed_confidence_is_a_failed_check_not_a_crashed_scan(identity):
    # 95 instead of 0.95 used to escape as a ValidationError and abort the whole scan
    report = await _agent(FakeChatModel(confidence=95)).run_report([candidate()], identity)
    assert (report.checked, report.failed, report.confirmed) == (1, 1, [])


async def test_failures_do_not_hide_real_findings_in_the_same_batch(identity):
    class Flaky(FakeChatModel):
        def with_structured_output(self, schema):
            outer, inner = self, super().with_structured_output(schema)

            class _S:
                async def ainvoke(self, messages):
                    if len(outer.sent) == 1:        # the second call blows up
                        outer.sent.append(messages)
                        raise RuntimeError("blip")
                    return await inner.ainvoke(messages)
            return _S()

    report = await _agent(Flaky()).run_report([candidate(), candidate(), candidate()], identity)
    assert (report.checked, report.failed, len(report.confirmed)) == (3, 1, 2)


async def test_failures_are_logged_without_the_listing_url_or_exception_text(identity):
    import structlog

    with structlog.testing.capture_logs() as logs:
        await _agent(FakeChatModel(error=RuntimeError("401 for jane-smith at https://secret.example/jane"))).run_report(
            [candidate()], identity
        )
    text = str(logs).lower()
    assert "detection_llm_error" in text and "runtimeerror" in text
    assert "jane" not in text and "secret.example" not in text and "peoplesearch.example/people" not in text
    assert "peoplesearch.example" in text, "the host is logged; the path is not"
