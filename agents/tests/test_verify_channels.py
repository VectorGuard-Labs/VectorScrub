"""The check that each deletion channel's address is still on the company's own page."""
import json
from datetime import date

import httpx

from deletion.verify_channels import CATALOG, STALE_AFTER_DAYS, check_channel, exit_code, is_stale, load_catalog, verify

REAL = httpx.AsyncClient
CHANNEL = {
    "id": "acme", "address": "privacyrequest@acme.example", "source_url": "https://www.acme.example/privacy",
    "verified_on": "2026-11-12",
}
TODAY = date(2026, 11, 20)


def client(handler):
    return REAL(transport=httpx.MockTransport(handler))


def page(text, status=200):
    return lambda req: httpx.Response(status, text=text, headers={"content-type": "text/html"})


async def run(handler, channel=CHANNEL):
    async with client(handler) as c:
        return await check_channel(c, channel, TODAY)


async def test_an_address_in_visible_text_or_in_a_mailto_link_is_confirmed():
    for html in (
        "<html><body>Email PrivacyRequest@Acme.example to request deletion.</body></html>",
        '<html><body><a href="mailto:privacyrequest@acme.example">Contact us</a></body></html>',
    ):
        r = await run(page(html))
        assert (r.result, r.stale) == ("confirmed", False), html


async def test_a_page_that_loads_without_the_address_is_flagged_for_review():
    r = await run(page("<html><body>Use our web form instead. Nothing else.</body></html>"))
    assert r.result == "address_missing"
    assert exit_code([r]) == 1


async def test_a_page_that_refuses_or_fails_is_not_treated_as_the_address_being_gone():
    for handler in (page("", 403), page("", 404), page("", 503)):
        r = await run(handler)
        assert r.result == "could_not_check" and exit_code([r]) == 0
        assert exit_code([r], strict=True) == 1, "--strict also fails on a page that could not be checked"

    def boom(req):
        raise httpx.ConnectError("unreachable")
    assert (await run(boom)).result == "could_not_check"


async def test_a_redirect_off_the_companys_site_is_not_followed():
    def handler(req):
        if req.headers["host"] == "www.acme.example":
            return httpx.Response(302, headers={"location": "https://evil.example/page-that-mentions-privacyrequest@acme.example"})
        return httpx.Response(200, text="privacyrequest@acme.example", headers={"content-type": "text/html"})
    r = await run(handler)
    assert r.result == "could_not_check", "an address on another company's page proves nothing"


def test_staleness_follows_the_verified_date():
    assert not is_stale("2026-11-12", TODAY)
    assert is_stale("2026-01-01", date(2026, 11, 20))
    assert not is_stale("2026-05-25", date(2026, 11, 20)) and is_stale("2026-05-20", date(2026, 11, 20))
    assert is_stale("garbage", TODAY) and is_stale("", TODAY)
    assert STALE_AFTER_DAYS == 180


async def test_a_stale_entry_is_reported_and_fails_only_in_strict_mode():
    old = dict(CHANNEL, verified_on="2025-01-01")
    r = await run(page("privacyrequest@acme.example"), old)
    assert (r.result, r.stale) == ("confirmed", True)
    assert exit_code([r]) == 0 and exit_code([r], strict=True) == 1


async def test_every_channel_is_checked_in_turn():
    seen = []
    def handler(req):
        seen.append(req.headers["host"])
        return httpx.Response(200, text="a@one.example b@two.example", headers={"content-type": "text/html"})
    channels = [dict(CHANNEL, id="one", address="a@one.example", source_url="https://one.example/p"),
                dict(CHANNEL, id="two", address="b@two.example", source_url="https://two.example/p")]
    async with client(handler) as c:
        results = await verify(channels, TODAY, c)
    assert [r.id for r in results] == ["one", "two"] and seen == ["one.example", "two.example"]


def test_the_real_catalog_loads_and_is_what_the_backend_validates():
    channels = load_catalog()
    assert CATALOG.exists() and channels
    for c in channels:
        assert c["method"] == "email" and c["address"] in c["evidence"].lower()
    raw = json.loads(CATALOG.read_text(encoding="utf-8"))
    assert set(raw) == {"channels"}
