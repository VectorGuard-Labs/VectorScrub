"""The finder only PROPOSES addresses. These tests pin what it reads, what it ignores, and that it behaves."""
import httpx

from deletion.find_channels import Candidate, extract_candidates, registrable, scan, score

OWN = {"acme.example"}
PAGE = """<html><body>
<p>To request deletion of your personal information, email privacyrequest@acme.example and we will respond within 45 days.
An authorized agent may also write to us.</p>
<p>Residents of the EEA may contact our Data Protection Officer at dpo@acme.example under the GDPR.</p>
<p>Press inquiries: press@acme.example. General customer support: support@acme.example.</p>
<p>A third party, partner@elsewhere.example, is not ours. <a href="mailto:hidden@acme.example">click here</a></p>
</body></html>"""


def by_email(cands):
    return {c.email: c for c in cands}


def test_it_finds_only_addresses_on_the_companys_own_domains_with_their_context():
    got = by_email(extract_candidates(PAGE, "https://www.acme.example/privacy", OWN))
    assert set(got) == {"privacyrequest@acme.example", "dpo@acme.example", "press@acme.example", "support@acme.example", "hidden@acme.example"}
    assert "partner@elsewhere.example" not in got, "another company's address is never proposed"
    assert "request deletion of your personal information" in got["privacyrequest@acme.example"].context


def test_the_ranking_prefers_a_deletion_request_address_over_dpo_press_and_support():
    got = by_email(extract_candidates(PAGE, "https://www.acme.example/privacy", OWN))
    ranked = sorted(got.values(), key=lambda c: -c.score)
    assert ranked[0].email == "privacyrequest@acme.example"
    c = got["privacyrequest@acme.example"]
    assert (c.delete, c.rights, c.agent, c.eu) == (True, False, True, False)
    assert got["dpo@acme.example"].eu and got["dpo@acme.example"].score < c.score
    assert got["press@acme.example"].general and got["support@acme.example"].generic_local
    assert got["support@acme.example"].score < c.score


def test_an_address_that_only_exists_in_a_mailto_link_is_still_reported():
    got = by_email(extract_candidates(PAGE, "u", OWN))
    assert got["hidden@acme.example"].mailto, "found through the link, though the visible text is 'click here'"


def test_the_context_is_the_paragraph_of_the_address_not_its_neighbours():
    c = by_email(extract_candidates(PAGE, "u", OWN))["privacyrequest@acme.example"]
    assert "within 45 days" in c.context
    assert "Data Protection Officer" not in c.context and "Press inquiries" not in c.context
    assert not c.eu and not c.general, "the DPO and press paragraphs next to it do not count against it"
    assert len(c.context) <= 450


def test_subdomain_addresses_count_as_the_companys_own_but_lookalike_domains_do_not():
    html = "<p>Delete requests: a@privacy.acme.example, b@acme.example.evil.net, c@notacme.example</p>"
    assert set(by_email(extract_candidates(html, "u", OWN))) == {"a@privacy.acme.example"}
    assert registrable("privacy.acme.example") == "acme.example"


def test_markup_noise_is_ignored_and_broken_html_does_not_crash():
    html = "<script>var e='x@acme.example'</script><style>.a{}</style><p>real: y@acme.example</p><div><p>unclosed"
    assert set(by_email(extract_candidates(html, "u", OWN))) == {"y@acme.example"}
    assert extract_candidates("", "u", OWN) == []
    assert extract_candidates("<<<>>>", "u", OWN) == []


def test_score_arithmetic():
    base = dict(email="a@b.example", page="u", context="", delete=False, rights=False, agent=False, eu=False,
                general=False, generic_local=False, mailto=False)
    assert score(Candidate(**base)) == 0
    assert score(Candidate(**{**base, "delete": True, "rights": True, "agent": True, "mailto": True})) == 7
    assert score(Candidate(**{**base, "eu": True, "general": True, "generic_local": True})) == -6


BROKERS = [
    {"id": "1", "name": "Acme", "sites": ["acme.example"], "rights": ["https://www.acme.example/privacy"]},
    {"id": "2", "name": "Walled", "sites": ["wall.example"], "rights": ["https://wall.example/p"]},
    {"id": "3", "name": "Two pages", "sites": ["two.example"], "rights": ["https://two.example/a", "https://two.example/b"]},
]


async def test_scan_reads_each_companys_pages_survives_failures_and_keeps_the_best_first(monkeypatch):
    seen = []

    def handler(req):
        seen.append(req.headers["host"] + req.url.path)
        host = req.headers["host"]
        if host == "wall.example":
            return httpx.Response(403)
        if host == "two.example":
            body = "<p>delete: a@two.example</p>" if req.url.path == "/a" else "<p>Our press office: press@two.example</p>"
            return httpx.Response(200, text=body, headers={"content-type": "text/html"})
        return httpx.Response(200, text=PAGE, headers={"content-type": "text/html"})

    async with httpx.AsyncClient(transport=httpx.MockTransport(handler)) as c:
        results = {r.id: r for r in await scan(BROKERS, concurrency=3, delay=0, client=c)}
    assert (results["1"].pages_read, results["1"].pages_failed) == (1, 0)
    assert results["1"].candidates[0].email == "privacyrequest@acme.example"
    assert (results["2"].pages_read, results["2"].pages_failed, results["2"].candidates) == (0, 1, [])
    assert {c.email for c in results["3"].candidates} == {"a@two.example", "press@two.example"}, "both pages are read"
    assert len(seen) == 4


async def test_scan_never_has_two_requests_in_flight_to_one_site(monkeypatch):
    import asyncio
    live = {"n": 0, "max": 0}

    async def handler(req):
        live["n"] += 1
        live["max"] = max(live["max"], live["n"])
        await asyncio.sleep(0.01)
        live["n"] -= 1
        return httpx.Response(200, text="<p>x</p>", headers={"content-type": "text/html"})

    brokers = [{"id": str(i), "name": f"C{i}", "sites": ["same.example"], "rights": [f"https://www.same.example/{i}"]} for i in range(6)]
    async with httpx.AsyncClient(transport=httpx.MockTransport(handler)) as c:
        await scan(brokers, concurrency=4, delay=0, client=c)
    assert live["max"] == 1
