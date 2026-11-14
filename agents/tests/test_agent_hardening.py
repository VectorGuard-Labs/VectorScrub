"""
Logging hygiene, SMTP verification, email escaping, the backend-secrets guard, tier coverage.
"""
import smtplib
import ssl
import uuid

import httpx
import pytest
import structlog

import discovery.agent as da
import removal.executor as ex
from discovery.agent import DISCOVERY_SOURCES, DiscoveryAgent
from removal.agent import OPT_OUT_REGISTRY
from removal.executor import RemovalExecutor
from shared import config as cfgmod
from shared.models import IdentityContext, RemovalProcedure

SECRET_URL = "https://www.spokeo.com/Jane-Smith/Austin-TX/5125550147"
LEAKY = f"GET {SECRET_URL} failed for jane.smith@example.com: 401 body={{'name': 'Jane Smith'}}"


def proc(kind="web_form", **kw):
    return RemovalProcedure(source_name="spokeo.com", procedure_type=kind, opt_out_url="https://www.spokeo.com/optout?u=jane-smith",
                            opt_out_email="privacy@broker.example", estimated_days=7, **kw)


@pytest.fixture
def jane():
    return IdentityContext(customer_id=uuid.uuid4(), first_name="Jane", last_name="Smith", full_name="Jane Smith",
                           email="jane.smith@example.com", current_address="123 Main St, Austin, TX",
                           phone_numbers=["512-555-0147"])


def assert_clean(text):
    low = str(text).lower()
    for needle in ("jane", "smith", "5125550147", "512-555-0147", "401", "spokeo.com/", "optout?u=", "http"):
        assert needle not in low, f"{needle!r} leaked: {text}"


# ---- logging ---------------------------------------------------------------------------

async def test_a_crashing_submission_logs_and_returns_only_the_exception_type(jane, monkeypatch):
    async def boom(*a, **k):
        raise RuntimeError(LEAKY)
    e = RemovalExecutor()
    monkeypatch.setattr(e, "submit_web_form", boom)
    with structlog.testing.capture_logs() as logs:
        r = await e.submit_removal_request("req-1", "cust-1", proc(), jane, SECRET_URL)
    assert r["status"] == "failed" and r["error"] == "Submission failed (RuntimeError)"
    assert_clean(logs)
    assert any(l["event"] == "submission_failed" and l["error_type"] == "RuntimeError" for l in logs)


async def test_the_completion_log_does_not_dump_the_result(jane, monkeypatch):
    async def done(*a, **k):
        return {"status": "submitted", "confirmation_ref": "REF-12345", "evidence_hash": "abc", "error": None}
    e = RemovalExecutor()
    monkeypatch.setattr(e, "submit_web_form", done)
    with structlog.testing.capture_logs() as logs:
        await e.submit_removal_request("req-1", "cust-1", proc(), jane, SECRET_URL)
    done_log = next(l for l in logs if l["event"] == "submission_complete")
    assert set(done_log) <= {"event", "log_level", "status", "procedure_type"}


async def test_the_web_form_logs_the_host_not_the_opt_out_url(jane, monkeypatch):
    def no_browser():
        raise RuntimeError("no browser in this test")
    monkeypatch.setattr(ex, "async_playwright", no_browser)
    with structlog.testing.capture_logs() as logs:
        await RemovalExecutor().submit_removal_request("r", "c", proc(), jane, SECRET_URL)
    assert any(l["event"] == "submit_web_form" and l["opt_out_host"] == "www.spokeo.com" for l in logs)
    assert "optout?u=" not in str(logs) and "jane-smith" not in str(logs).lower()


async def test_an_email_failure_is_reported_by_type_only(jane, monkeypatch):
    e = RemovalExecutor()
    monkeypatch.setattr(e.config, "smtp_username", "u"); monkeypatch.setattr(e.config, "smtp_password", "p")
    async def boom(*a, **k):
        raise smtplib.SMTPRecipientsRefused({"privacy@broker.example": (550, b"no such user jane.smith")})
    monkeypatch.setattr(e, "_send_smtp_email", boom)
    with structlog.testing.capture_logs() as logs:
        r = await e.submit_email_request(proc("email"), jane, SECRET_URL)
    assert r["status"] == "failed" and "SMTPRecipientsRefused" in r["error"]
    assert_clean(r["error"]); assert_clean(logs)


async def test_a_drop_error_body_that_echoes_the_identity_is_not_logged_or_returned(jane, monkeypatch):
    e = RemovalExecutor()
    monkeypatch.setattr(e.config, "drop_api_key", "k"); monkeypatch.setattr(e.config, "drop_api_secret", "s")
    real = httpx.AsyncClient
    def handler(req):
        return httpx.Response(422, text='{"detail": "invalid name Jane Smith, phone 512-555-0147"}')
    monkeypatch.setattr(ex.httpx, "AsyncClient", lambda **kw: real(transport=httpx.MockTransport(handler), **kw))
    with structlog.testing.capture_logs() as logs:
        r = await e.submit_drop_api(proc("drop_api"), jane)
    assert r["status"] == "failed" and r["error"] == "DROP API error: HTTP 422"
    assert "jane" not in str(logs).lower() and "512" not in str(logs)


async def test_discovery_logs_a_failed_source_by_type(identity, monkeypatch):
    async def explode(self, client, name, url, ident):
        raise ValueError(LEAKY)
    monkeypatch.setattr(DiscoveryAgent, "_fetch_source", explode)
    with structlog.testing.capture_logs() as logs:
        await DiscoveryAgent(tier="free").run(identity)
    errors = [l for l in logs if l["event"] == "discovery_source_error"]
    assert errors and all(l["error_type"] == "ValueError" for l in errors)
    assert "jane" not in str(errors).lower() and "401" not in str(errors)


# ---- SMTP ------------------------------------------------------------------------------

class FakeSMTP:
    instances = []

    def __init__(self, host, port, timeout=None, context=None):
        self.host, self.port, self.context = host, port, context
        self.tls_context, self.logged_in, self.sent, self.quit_called = None, False, False, False
        self.fail_login = False
        FakeSMTP.instances.append(self)

    def starttls(self, context=None):
        self.tls_context = context

    def login(self, u, p):
        if self.fail_login:
            raise smtplib.SMTPAuthenticationError(535, b"bad credentials for jane")
        self.logged_in = True

    def send_message(self, msg):
        self.sent = True

    def quit(self):
        self.quit_called = True


@pytest.fixture
def smtp(monkeypatch):
    FakeSMTP.instances = []
    monkeypatch.setattr(ex.smtplib, "SMTP", FakeSMTP)
    monkeypatch.setattr(ex.smtplib, "SMTP_SSL", FakeSMTP)
    e = RemovalExecutor()
    for k, v in dict(smtp_host="smtp.example.com", smtp_port=587, smtp_use_tls=True, smtp_username="u", smtp_password="p").items():
        monkeypatch.setattr(e.config, k, v)
    return e


def message(jane, e):
    msg_proc = proc("email")
    from email.mime.text import MIMEText
    m = MIMEText("hi"); m["Message-ID"] = "<fixed@x>"
    return m


async def test_starttls_verifies_the_servers_certificate_and_hostname(smtp, jane):
    await smtp._send_smtp_email(message(jane, smtp), "privacy@broker.example")
    s = FakeSMTP.instances[0]
    assert isinstance(s.tls_context, ssl.SSLContext), "starttls() was called with no context, i.e. unverified"
    assert s.tls_context.verify_mode == ssl.CERT_REQUIRED and s.tls_context.check_hostname is True
    assert s.logged_in and s.sent and s.quit_called


async def test_port_465_uses_implicit_tls_with_verification(smtp, jane, monkeypatch):
    monkeypatch.setattr(smtp.config, "smtp_port", 465)
    await smtp._send_smtp_email(message(jane, smtp), "x@y.example")
    s = FakeSMTP.instances[0]
    assert isinstance(s.context, ssl.SSLContext) and s.context.verify_mode == ssl.CERT_REQUIRED
    assert s.tls_context is None, "no STARTTLS on an already-encrypted connection"


async def test_credentials_are_never_sent_in_clear_text_to_a_remote_server(smtp, jane, monkeypatch):
    monkeypatch.setattr(smtp.config, "smtp_use_tls", False)
    with pytest.raises(ssl.SSLError):
        await smtp._send_smtp_email(message(jane, smtp), "x@y.example")
    assert FakeSMTP.instances == [] or not FakeSMTP.instances[0].logged_in


async def test_plain_smtp_is_allowed_only_to_localhost(smtp, jane, monkeypatch):
    monkeypatch.setattr(smtp.config, "smtp_use_tls", False); monkeypatch.setattr(smtp.config, "smtp_host", "localhost")
    await smtp._send_smtp_email(message(jane, smtp), "x@y.example")
    assert FakeSMTP.instances[0].sent


async def test_the_connection_is_closed_even_when_login_fails(smtp, jane, monkeypatch):
    class Failing(FakeSMTP):
        def __init__(self, *a, **k):
            super().__init__(*a, **k); self.fail_login = True
    monkeypatch.setattr(ex.smtplib, "SMTP", Failing)
    with pytest.raises(smtplib.SMTPAuthenticationError):
        await smtp._send_smtp_email(message(jane, smtp), "x@y.example")
    assert FakeSMTP.instances[0].quit_called


async def test_the_message_id_is_stable_and_set_before_sending(smtp, jane):
    assert (await smtp._send_smtp_email(message(jane, smtp), "x@y.example")) == "<fixed@x>"


# ---- email escaping --------------------------------------------------------------------

def test_values_cannot_inject_markup_into_the_html_email():
    evil = IdentityContext(customer_id=uuid.uuid4(), first_name="<script>alert(1)</script>", last_name="Smith",
                           email="a@b.example", current_address='1 "><img src=x onerror=alert(2)>',
                           previous_addresses=["<b>old</b>"], phone_numbers=["<i>555</i>"])
    html_body, text_body = RemovalExecutor()._build_email_body(evil, 'https://x.example/?q="><svg onload=alert(3)>', proc("email"))
    for raw in ("<script>", "<img src=x", "<svg onload", "<b>old</b>", "<i>555</i>"):
        assert raw not in html_body, raw
    assert "&lt;script&gt;" in html_body and "&lt;b&gt;old&lt;/b&gt;" in html_body
    assert "<script>alert(1)</script>" in text_body, "the plain-text part is not HTML and stays literal"


# ---- backend secrets never reach the agents --------------------------------------------

@pytest.mark.parametrize("name", cfgmod.BACKEND_ONLY_SECRETS)
def test_the_agents_refuse_to_run_with_a_backend_secret_in_their_environment(name, monkeypatch):
    monkeypatch.setenv(name, "super-secret-value-123")
    with pytest.raises(cfgmod.ForbiddenSecretsError) as e:
        cfgmod.assert_no_backend_secrets(None)
    assert name in str(e.value) and "super-secret-value-123" not in str(e.value), "names only, never values"


def test_a_backend_secret_in_the_env_file_is_refused_too(tmp_path):
    f = tmp_path / ".env"
    f.write_text("AGENT_API_KEY=k\nMASTER_ENCRYPTION_KEY=abcdef==\n")
    with pytest.raises(cfgmod.ForbiddenSecretsError) as e:
        cfgmod.assert_no_backend_secrets(str(f))
    assert "MASTER_ENCRYPTION_KEY" in str(e.value) and "abcdef" not in str(e.value)


def test_an_empty_value_or_an_agent_only_file_is_fine(tmp_path, monkeypatch):
    f = tmp_path / ".env"
    f.write_text("AGENT_API_KEY=k\nREDIS_URL=redis://x\nJWT_SECRET=\n")
    cfgmod.assert_no_backend_secrets(str(f))
    cfgmod.assert_no_backend_secrets(str(tmp_path / "missing.env"))
    cfgmod.assert_no_backend_secrets(None)


def test_get_config_applies_the_guard(monkeypatch):
    monkeypatch.setenv("MASTER_ENCRYPTION_KEY", "x" * 44)
    cfgmod.get_config.cache_clear()
    try:
        with pytest.raises(cfgmod.ForbiddenSecretsError):
            cfgmod.get_config()
    finally:
        monkeypatch.delenv("MASTER_ENCRYPTION_KEY")
        cfgmod.get_config.cache_clear()


# ---- tiers -----------------------------------------------------------------------------

def names(tier):
    return {s["name"] for s in DiscoveryAgent(tier=tier)._sources_for_tier()}


def test_each_tier_gets_the_sources_at_or_below_its_level():
    free, personal, premium = names("free"), names("personal"), names("premium")
    assert (len(free), len(personal), len(premium)) == (4, 15, 34)
    assert free < personal < premium, "strict supersets: Premium has everything Personal has, plus more"


def test_premium_gets_the_expanded_sources_personal_does_not():
    extra = names("premium") - names("personal")
    # Phase 1 expansion added 15 new premium brokers + original 4 premium-only
    expected_premium_only = {
        # Original premium-only (4)
        "truthfinder.com", "thatsthem.com", "smartbackgroundchecks.com", "advancedbackgroundchecks.com",
        # Phase 1 expansion (15)
        "peekyou.com", "addresses.com", "usphonebook.com", "anywho.com", "411.com",
        "checkpeople.com", "peoplewiz.com", "infotracer.com", "publicrecordsnow.com",
        "myaddressbook.com", "neighbor.report", "pipl.com", "zoominfo.com",
        "contactout.com", "rocketreach.co"
    }
    assert extra == expected_premium_only


def test_every_discovery_source_has_a_removal_procedure():
    # otherwise a customer could be offered a removal that cannot run
    assert [s["name"] for s in DISCOVERY_SOURCES if s["name"] not in OPT_OUT_REGISTRY] == []


def test_an_unknown_tier_is_treated_as_free():
    assert names("enterprise") == names("free") == names("")


def test_every_source_url_stays_on_its_own_domain():
    from urllib.parse import urlparse
    for s in DISCOVERY_SOURCES:
        host = urlparse(s["url"]).hostname
        assert host == s["name"] or host.endswith("." + s["name"]), s["name"]
