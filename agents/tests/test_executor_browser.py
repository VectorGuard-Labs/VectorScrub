"""
The web-form executor in real headless Chromium against a local fake broker.

Contract: it reports "submitted" ONLY with positive evidence the broker accepted the request,
and fails with a reason otherwise (changed layout, CAPTCHA, ambiguous result, nothing to fill).
Needs `playwright install chromium`; skipped if the browser cannot start.
"""
import functools
import http.server
import threading
import uuid

import pytest

from removal.executor import RemovalExecutor
from shared.models import IdentityContext, RemovalProcedure

pytestmark = pytest.mark.browser

PAGES = {
    "good.html": '<html><body><h1>Opt out</h1><form action="/thanks.html" method="get">'
                 '<input name="full_name"><input name="listing_url"><input type="email" name="email">'
                 '<button type="submit">Submit</button></form></body></html>',
    "thanks.html": "<html><head><title>Done</title></head><body><h1>Thank you</h1><p>Your request has been submitted. Reference: ABC-12345</p></body></html>",
    "captcha.html": '<html><body><form><input name="full_name"><div class="g-recaptcha" data-sitekey="k"></div><button type="submit">Submit</button></form></body></html>',
    "changed.html": "<html><body><h1>We moved!</h1><p>Our opt-out is now in a new portal.</p><a href='/x'>Go</a></body></html>",
    "ambiguous.html": '<html><body><form action="/blank.html" method="get"><input name="full_name"><button type="submit">Submit</button></form></body></html>',
    "blank.html": "<html><body><h1>Opt-out</h1><p>Nothing to see here.</p></body></html>",
    "errorpage.html": '<html><body><form action="/err.html" method="get"><input name="full_name"><button type="submit">Submit</button></form></body></html>',
    "err.html": "<html><body><p>Thank you for contacting us. However, an error occurred. Please try again.</p></body></html>",
}


@pytest.fixture(scope="module")
def broker(tmp_path_factory):
    root = tmp_path_factory.mktemp("broker")
    for name, body in PAGES.items():
        (root / name).write_text(body)
    class Quiet(http.server.SimpleHTTPRequestHandler):
        def log_message(self, *args, **kwargs):  # the fake broker's own access log is not the app's
            pass

    handler = functools.partial(Quiet, directory=str(root))
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    yield f"http://127.0.0.1:{server.server_address[1]}"
    server.shutdown()


@pytest.fixture(scope="module", autouse=True)
def _chromium_available():
    import asyncio
    from playwright.async_api import async_playwright

    async def probe():
        async with async_playwright() as p:
            await (await p.chromium.launch(headless=True)).close()

    try:
        asyncio.run(probe())
    except Exception as exc:  # pragma: no cover
        pytest.skip(f"Chromium cannot start ({type(exc).__name__}); run `playwright install chromium`")


def proc(broker, page):
    return RemovalProcedure(source_name="t", procedure_type="web_form", opt_out_url=f"{broker}/{page}", estimated_days=3)


@pytest.fixture
def jane():
    return IdentityContext(customer_id=uuid.uuid4(), first_name="Jane", last_name="Smith", full_name="Jane Smith", email="jane@example.com")


async def test_a_working_form_with_a_confirmation_is_submitted(broker, jane):
    r = await RemovalExecutor().submit_web_form(proc(broker, "good.html"), jane, "https://broker.example/rec/1")
    assert r["status"] == "submitted"
    assert r["confirmation_ref"] == "ABC-12345"
    assert len(r["evidence_hash"]) == 64


async def test_a_captcha_is_a_hard_stop_not_something_to_get_around(broker, jane):
    r = await RemovalExecutor().submit_web_form(proc(broker, "captcha.html"), jane, "u")
    assert r["status"] == "failed" and "CAPTCHA" in r["error"]


async def test_a_changed_page_with_nothing_to_fill_fails_instead_of_reporting_success(broker, jane):
    r = await RemovalExecutor().submit_web_form(proc(broker, "changed.html"), jane, "u")
    assert r["status"] == "failed" and "layout" in r["error"]


async def test_submitting_without_seeing_a_confirmation_is_a_failure_with_evidence_kept(broker, jane):
    r = await RemovalExecutor().submit_web_form(proc(broker, "ambiguous.html"), jane, "u")
    assert r["status"] == "failed" and "no confirmation" in r["error"].lower()
    assert r["evidence_hash"], "the screenshot hash is kept so the failure can be diagnosed"


async def test_error_wording_beats_polite_wording(broker, jane):
    r = await RemovalExecutor().submit_web_form(proc(broker, "errorpage.html"), jane, "u")
    assert r["status"] == "failed", "'Thank you ... an error occurred' is not an acceptance"


async def test_an_identity_with_nothing_to_fill_cannot_silently_succeed(broker):
    blank = IdentityContext(customer_id=uuid.uuid4())
    r = await RemovalExecutor().submit_web_form(proc(broker, "good.html"), blank, "")
    assert r["status"] == "failed"


async def test_unknown_procedure_types_and_exceptions_become_failed_results():
    ex = RemovalExecutor()
    ident = IdentityContext(customer_id=uuid.uuid4(), full_name="Jane Smith")
    r = await ex.submit_removal_request("r", "c", RemovalProcedure(source_name="t", procedure_type="carrier_pigeon"), ident, "u")
    assert r["status"] == "failed" and "Unknown procedure" in r["error"]


async def test_the_listing_url_is_never_logged(broker, jane, capsys):
    await RemovalExecutor().submit_removal_request("r", "c", proc(broker, "good.html"), jane, "https://broker.example/people/jane-smith-1234")
    out = capsys.readouterr()
    assert "jane-smith" not in out.out + out.err
