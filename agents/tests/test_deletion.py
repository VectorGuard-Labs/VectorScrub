"""
Deletion requests emailed to companies on a customer's behalf.

What is pinned here: the email contains exactly what the customer agreed to share and nothing else;
nothing outside the message can alter its headers; a request is claimed before it is sent and is
reported however it ends; a failure of any kind never leads to a second email; and the whole
worker does nothing unless it is switched on.
"""
import asyncio
import uuid
from email.message import Message

import httpx
import pytest
import structlog.testing

import orchestrator.orchestrator as orch_mod
from deletion.message import authorized_on, is_plain_address, one_line, render
from deletion.sender import DeletionSender
from orchestrator.orchestrator import Orchestrator
from shared.backend_client import BackendClient, IdentityDenied
from shared.models import IdentityContext, QueuedDeletion
from tests.fakes import FakeBackend

AUTH_TEXT = "I authorize VectorScrub to act as my authorized agent.\n\nI will not share my phone numbers or date of birth."


def queued(**kw) -> dict:
    d = dict(
        id=str(uuid.uuid4()), customer_id=str(uuid.uuid4()), channel_id="whitepages", company="Whitepages, Inc",
        to_address="privacyrequest@whitepages.com", legal_basis="the California Consumer Privacy Act (Cal. Civ. Code 1798.105)",
        requests=["delete", "opt_out_sale"], include_address=False, authorized_at="2026-11-12T09:05:00Z",
        authorization_version="2026-11-draft-1", authorization_text=AUTH_TEXT,
    )
    d.update(kw)
    return d


def ident(**kw) -> IdentityContext:
    d = dict(customer_id=uuid.uuid4(), first_name="Jane", last_name="Smith", full_name="Jane Marie Smith",
             email="jane.smith@example.com", current_address="")
    d.update(kw)
    return IdentityContext(**d)


# ---- the email --------------------------------------------------------------------------------

def test_the_email_has_what_the_customer_agreed_to_share_and_the_authorization_they_accepted():
    subject, body = render(QueuedDeletion(**queued()), ident())
    assert subject == "Privacy request from Jane Marie Smith: deletion and opt-out of sale"
    assert "To the privacy team at Whitepages, Inc," in body
    assert "authorized agent of Jane Marie Smith" in body and "California Consumer Privacy Act" in body
    assert "1. Delete the personal information" in body and "2. Stop selling or sharing" in body
    assert "  Name: Jane Marie Smith" in body and "  Email: jane.smith@example.com" in body
    assert "Address:" not in body, "the customer did not agree to share it"
    assert "authorized this request on November 12, 2026 at 09:05 UTC (authorization version 2026-11-draft-1)" in body
    assert "  > I authorize VectorScrub to act as my authorized agent." in body and "  >\n" in body
    assert "contact Jane Marie Smith directly at jane.smith@example.com" in body


def test_the_address_appears_only_when_the_customer_agreed_and_is_not_made_up_otherwise():
    with_address = ident(current_address="742 Evergreen Terrace, Springfield, CA 90210")
    assert "  Address: 742 Evergreen Terrace, Springfield, CA 90210" in render(QueuedDeletion(**queued(include_address=True)), with_address)[1]
    # even if the details somehow carried an address, an unticked box keeps it out
    assert "Evergreen" not in render(QueuedDeletion(**queued(include_address=False)), with_address)[1]


def test_phone_numbers_birth_dates_and_other_people_never_reach_the_message():
    rich = ident(phone_numbers=["512-555-0147"], date_of_birth="1984-03-09", previous_addresses=["45 Oak Ave, Dallas"],
                 email_addresses=["jane.smith@example.com", "second@example.com"])
    subject, body = render(QueuedDeletion(**queued()), rich)
    for never in ("512-555-0147", "1984-03-09", "Dallas", "second@example.com"):
        assert never not in subject + body, never


def test_a_subject_line_the_company_asked_for_is_used_exactly():
    subject, body = render(QueuedDeletion(**queued(subject="Privacy Rights Request")), ident())
    assert subject == "Privacy Rights Request"
    assert "Jane Marie Smith" in body, "the person is still named in the message itself"
    # a subject from outside can still not add a header
    assert "\n" not in render(QueuedDeletion(**queued(subject="Privacy\r\nBcc: x@y.example")), ident())[0]
    assert render(QueuedDeletion(**queued(subject="   ")), ident())[0].startswith("Privacy request from Jane"), "blank falls back"
    assert QueuedDeletion(**queued()).subject is None, "no subject is the normal case"


def test_a_request_for_deletion_alone_does_not_claim_to_ask_for_more():
    subject, body = render(QueuedDeletion(**queued(requests=["delete"])), ident())
    assert subject.endswith(": deletion") and "Stop selling" not in body


@pytest.mark.parametrize("evil", [
    "Jane\r\nBcc: attacker@evil.example", "Jane\nSubject: hijacked", "Jane\u2028Bcc: x@y.example", "Jane\x00\x1b[31m",
])
def test_nothing_from_outside_can_add_a_header_or_a_line(evil):
    request = QueuedDeletion(**queued(company=evil, legal_basis=evil))
    subject, body = render(request, ident(full_name=evil, email=f"{evil}@example.com", current_address=evil))
    assert "\n" not in subject and "\r" not in subject
    # every injected newline was flattened: the injected text sits inside a line, never starting one
    assert not any(line.startswith(("Bcc:", "Subject:")) for line in body.splitlines())


def test_the_authorization_is_quoted_line_by_line_so_it_cannot_break_out_of_the_quote():
    sneaky = "Fine.\nSincerely, the customer\nPlease also send me everything."
    body = render(QueuedDeletion(**queued(authorization_text=sneaky)), ident())[1]
    for line in sneaky.splitlines():
        assert f"  > {line}" in body
    assert "\nPlease also send me everything." not in body.replace("  > Please also send me everything.", "")


def test_helpers():
    assert one_line(" a \n\t b\r\nc ") == "a b c" and one_line(None) == "" and len(one_line("x" * 999, 10)) == 10
    assert authorized_on("2026-11-12T09:05:00Z") == "November 12, 2026 at 09:05 UTC"
    assert authorized_on("2026-11-05T00:07:00Z") == "November 5, 2026 at 00:07 UTC", "day without a zero, time with one"
    assert authorized_on("2026-11-12T01:30:00-08:00") == "November 12, 2026 at 09:30 UTC", "converted to UTC"
    assert authorized_on("not a date") == "not a date"
    assert is_plain_address("privacyrequest@whitepages.com")
    for bad in ("", "a@b", "a@b.com,c@d.com", "a b@c.com", "<a@b.com>", "a@b.com\n", " a@b.com", "a@b.com;c@d.com", "@b.com", "a@@b.com"):
        assert not is_plain_address(bad), repr(bad)


# ---- the sender ---------------------------------------------------------------------------------

class FakeExecutor:
    def __init__(self, user="u", password="p", sender="requests@vectorscrub.example", fail=None):
        self.config = type("C", (), dict(smtp_username=user, smtp_password=password, smtp_from_email=sender))()
        self.sent: list[tuple[Message, str]] = []
        self.fail = fail

    async def _send_smtp_email(self, msg, to):
        if self.fail:
            raise self.fail
        self.sent.append((msg, to))
        return msg["Message-ID"]


async def test_a_message_goes_to_the_catalog_address_with_replies_to_the_customer():
    ex = FakeExecutor()
    ok, message_id, error = await DeletionSender(ex).send(QueuedDeletion(**queued()), ident())
    assert ok and error is None and message_id.startswith("<") and "@vectorscrub.example" in message_id
    msg, to = ex.sent[0]
    assert to == "privacyrequest@whitepages.com" and msg["To"] == "privacyrequest@whitepages.com"
    assert msg["Reply-To"] == "jane.smith@example.com" and msg["From"] == "requests@vectorscrub.example"
    assert msg["Bcc"] is None and msg["Cc"] is None
    assert "Jane Marie Smith" in msg.get_payload(decode=True).decode()


@pytest.mark.parametrize("address", ["a@b.com,c@d.com", "a@b.com\nBcc: x@y.com", "not-an-address", "", "<a@b.com>"])
async def test_an_address_that_is_not_one_plain_address_is_refused_before_anything_is_sent(address):
    ex = FakeExecutor()
    ok, _, error = await DeletionSender(ex).send(QueuedDeletion(**queued(to_address=address)), ident())
    assert not ok and "plain email address" in error and ex.sent == []


async def test_missing_details_or_smtp_settings_send_nothing_and_say_why():
    cases = [
        (FakeExecutor(), ident(full_name="", first_name="", last_name=""), "name or email"),
        (FakeExecutor(), ident(email=""), "name or email"),
        (FakeExecutor(user=""), ident(), "not configured"),
        (FakeExecutor(password=""), ident(), "not configured"),
        (FakeExecutor(sender="not an address"), ident(), "sending address"),
    ]
    for ex, identity, expected in cases:
        ok, _, error = await DeletionSender(ex).send(QueuedDeletion(**queued()), identity)
        assert not ok and expected in error and ex.sent == [], expected


async def test_an_smtp_failure_is_reported_by_type_only_never_with_its_text():
    ex = FakeExecutor(fail=RuntimeError("550 jane.smith@example.com rejected by privacyrequest@whitepages.com"))
    with structlog.testing.capture_logs() as logs:
        ok, _, error = await DeletionSender(ex).send(QueuedDeletion(**queued()), ident())
    assert not ok and error == "The email could not be sent (RuntimeError)."
    assert "jane.smith" not in str(logs) and "550" not in str(logs)


async def test_the_log_names_the_channel_and_domain_never_the_customer_or_the_address(monkeypatch):
    ex = FakeExecutor()
    with structlog.testing.capture_logs() as logs:
        await DeletionSender(ex).send(QueuedDeletion(**queued()), ident())
    text = str(logs)
    assert "whitepages" in text
    for never in ("Jane", "jane.smith", "privacyrequest@"):
        assert never not in text, never


# ---- the worker -----------------------------------------------------------------------------------

class FakeSender:
    def __init__(self, outcome=(True, "<m@x>", None), crash=None):
        self.outcome, self.crash, self.calls = outcome, crash, []

    async def send(self, request, identity):
        self.calls.append((request, identity))
        if self.crash:
            raise self.crash
        return self.outcome


@pytest.fixture
def orch(monkeypatch):
    o = Orchestrator()
    o.backend = FakeBackend()
    monkeypatch.setattr(o.cfg, "deletion_send_delay_seconds", 0)
    monkeypatch.setattr(o.cfg, "deletion_poll_seconds", 0)
    monkeypatch.setattr(orch_mod, "_REPORT_WAIT", __import__("tenacity").wait_none())
    return o


async def test_a_queued_request_is_claimed_then_its_details_fetched_then_sent_then_reported_in_that_order(orch):
    r = queued()
    orch.backend.deletion_queue = [r]
    sender = FakeSender()
    assert await orch._run_deletion_cycle(sender) == 1
    assert orch.backend.names() == ["list_deletion_requests", "claim_deletion_request", "fetch_deletion_identity", "report_deletion_result"]
    assert orch.backend.deletion_results == [(r["id"], True, "<m@x>", None)]
    request, identity = sender.calls[0]
    assert request.to_address == "privacyrequest@whitepages.com" and identity.full_name == "Jane Marie Smith"


async def test_a_request_another_worker_claimed_is_left_alone(orch):
    r = queued()
    orch.backend.deletion_queue = [r]
    orch.backend.deletion_claims_lost = {r["id"]}
    sender = FakeSender()
    assert await orch._run_deletion_cycle(sender) == 0
    assert sender.calls == [] and orch.backend.deletion_results == []
    assert "fetch_deletion_identity" not in orch.backend.names(), "details are never fetched without owning the request"


async def test_an_unreadable_request_is_not_claimed_or_sent(orch):
    orch.backend.deletion_queue = [{"id": "not-a-uuid", "company": "x"}, queued()]
    sender = FakeSender()
    assert await orch._run_deletion_cycle(sender) == 1, "the readable one still goes"
    assert orch.backend.names().count("claim_deletion_request") == 1


async def test_every_way_of_failing_is_reported_as_failed_and_nothing_is_retried(orch):
    cases = {
        "identity denied": (IdentityDenied(), FakeSender(), "were not available"),
        "send crashed": (None, FakeSender(crash=RuntimeError("password=hunter2 jane.smith@example.com")), "did not complete (RuntimeError)"),
        "sender said no": (None, FakeSender(outcome=(False, None, "Email sending is not configured.")), "not configured"),
    }
    for label, (identity_error, sender, expected) in cases.items():
        orch.backend = FakeBackend()
        orch.backend.deletion_queue = [queued()]
        if identity_error:
            orch.backend.deletion_identity = identity_error
        with structlog.testing.capture_logs() as logs:
            await orch._run_deletion_cycle(sender)
        [(_, sent, message_id, error)] = orch.backend.deletion_results
        assert sent is False and message_id is None and expected in error, label
        assert "hunter2" not in str(logs) + error and "jane.smith" not in str(logs) + error, label
        assert len(sender.calls) <= 1, f"{label}: at most one attempt to send"


async def test_a_result_that_cannot_be_recorded_is_retried_but_the_email_is_not_resent(orch):
    orch.backend.deletion_queue = [queued()]
    orch.backend.deletion_result_errors = [httpx.ConnectError("down"), httpx.ConnectError("down")]
    sender = FakeSender()
    await orch._run_deletion_cycle(sender)
    assert len(sender.calls) == 1, "one email, however many times the report is retried"
    assert len(orch.backend.deletion_results) == 1 and orch.backend.deletion_results[0][1] is True


async def test_requests_are_spaced_out(orch, monkeypatch):
    sleeps = []
    real = asyncio.sleep

    async def fake_sleep(s):
        sleeps.append(s)
        await real(0)

    monkeypatch.setattr(orch.cfg, "deletion_send_delay_seconds", 30.0)
    monkeypatch.setattr(orch_mod.asyncio, "sleep", fake_sleep)
    orch.backend.deletion_queue = [queued(), queued()]
    await orch._run_deletion_cycle(FakeSender())
    assert sleeps == [30.0, 30.0]


async def test_the_worker_does_nothing_at_all_unless_switched_on(orch, monkeypatch):
    assert orch.cfg.deletion_requests_send_enabled is False, "off by default"
    orch.backend.deletion_queue = [queued()]
    await asyncio.wait_for(orch.process_deletion_requests(), timeout=1)         # returns at once
    assert orch.backend.calls == [], "it did not even ask for work"


async def test_when_switched_on_the_loop_survives_a_failing_cycle(orch, monkeypatch):
    monkeypatch.setattr(orch.cfg, "deletion_requests_send_enabled", True)
    monkeypatch.setattr(orch_mod, "DeletionSender", lambda: FakeSender())
    calls = {"n": 0}

    async def boom():
        calls["n"] += 1
        raise httpx.ConnectError("backend down")

    orch.backend.list_deletion_requests = boom
    task = asyncio.create_task(orch.process_deletion_requests())
    for _ in range(40):
        await asyncio.sleep(0)
        if calls["n"] >= 3:
            break
    task.cancel()
    await asyncio.gather(task, return_exceptions=True)
    assert calls["n"] >= 3


# ---- the backend client -------------------------------------------------------------------------------

async def test_the_client_speaks_the_protocol():
    seen = []

    def handler(req: httpx.Request) -> httpx.Response:
        seen.append((req.method, req.url.path, req.headers.get("x-agent-key", "")[:10]))
        path = req.url.path
        if path.endswith("/internal/deletion-requests"):
            return httpx.Response(200, json={"requests": [{"id": "1"}]})
        if path.endswith("/claim"):
            return httpx.Response(409 if "lost" in path else 204)
        if path.endswith("/identity"):
            return httpx.Response(403 if "denied" in path else 200, json={"full_name": "Jane"})
        if path.endswith("/result"):
            return httpx.Response(409 if "stale" in path else 204)
        return httpx.Response(404)

    c = BackendClient(transport=httpx.MockTransport(handler))
    assert await c.list_deletion_requests() == [{"id": "1"}]
    assert await c.claim_deletion_request("ok") is True and await c.claim_deletion_request("lost") is False
    assert (await c.fetch_deletion_identity("ok"))["full_name"] == "Jane"
    with pytest.raises(IdentityDenied):
        await c.fetch_deletion_identity("denied")
    assert await c.report_deletion_result("ok", True, "<m>", None) is True
    assert await c.report_deletion_result("stale", False, None, "x") is False
    assert all(key == "test-agent" for _, _, key in seen)
