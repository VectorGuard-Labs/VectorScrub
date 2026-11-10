"""
SSRF protection and response limits for every page the agents fetch.

The fake resolver (conftest) maps special names to private addresses; anything else is public.
"""
import gzip
import httpx
import pytest

from shared.safe_fetch import FetchBlocked, ResponseTooLarge, is_public_address, safe_get

HEADERS = {"User-Agent": "test"}


def client(handler):
    return httpx.AsyncClient(transport=httpx.MockTransport(handler))


async def get(handler, url="https://www.spokeo.com/a", **kw):
    async with client(handler) as c:
        return await safe_get(c, url, headers=HEADERS, timeout=5, **kw)


def ok(text="hello"):
    return lambda req: httpx.Response(200, text=text, headers={"content-type": "text/html; charset=utf-8"})


# ---- addresses ---------------------------------------------------------------------

@pytest.mark.parametrize("addr", [
    "127.0.0.1", "10.1.2.3", "172.16.0.9", "192.168.1.1", "169.254.169.254", "0.0.0.0",
    "100.64.0.1", "224.0.0.1", "240.0.0.1", "::1", "fe80::1", "fd00::1", "::", "::ffff:10.0.0.5",
    "::ffff:127.0.0.1", "not-an-ip",
])
def test_non_public_addresses_are_refused(addr):
    assert not is_public_address(addr)


@pytest.mark.parametrize("addr", ["93.184.216.34", "8.8.8.8", "2606:2800:220:1:248:1893:25c8:1946"])
def test_public_addresses_are_allowed(addr):
    assert is_public_address(addr)


# ---- URLs --------------------------------------------------------------------------

@pytest.mark.parametrize("url, reason", [
    ("file:///etc/passwd", "scheme"),
    ("ftp://www.spokeo.com/x", "scheme"),
    ("gopher://www.spokeo.com/", "scheme"),
    ("https://user:pw@www.spokeo.com/", "credentials_in_url"),
    ("https://www.spokeo.com:8443/", "port"),
    ("http://www.spokeo.com:6379/", "port"),
    ("https://www.spokeo.com:notaport/", "bad_url"),
    ("https:///nohost", "bad_url"),
])
async def test_bad_urls_are_refused_before_any_request(url, reason):
    calls = []
    with pytest.raises(FetchBlocked) as e:
        await get(lambda r: calls.append(r) or httpx.Response(200), url=url)
    assert e.value.reason == reason and calls == []


@pytest.mark.parametrize("host, reason", [
    ("internal.test", "private_address"),
    ("metadata.test", "private_address"),
    ("loopback.test", "private_address"),
    ("v6private.test", "private_address"),
    ("mapped.test", "private_address"),
    ("mixed.test", "private_address"),       # one private answer is enough to refuse
    ("nxdomain.test", "unresolvable"),
])
async def test_names_that_resolve_to_private_addresses_are_refused(host, reason):
    calls = []
    with pytest.raises(FetchBlocked) as e:
        await get(lambda r: calls.append(r) or httpx.Response(200), url=f"https://{host}/x")
    assert e.value.reason == reason and calls == []


@pytest.mark.parametrize("url", [
    "http://169.254.169.254/latest/meta-data/",
    "http://127.0.0.1:80/",
    "http://[::1]/",
    "http://10.0.0.5/",
    "http://[::ffff:10.0.0.5]/",
])
async def test_private_ip_literals_are_refused(url):
    with pytest.raises(FetchBlocked) as e:
        await get(lambda r: httpx.Response(200), url=url)
    assert e.value.reason == "private_address"


# ---- redirects ---------------------------------------------------------------------

async def test_a_redirect_to_the_metadata_service_is_never_followed():
    seen = []

    def handler(req):
        seen.append(str(req.url))
        return httpx.Response(302, headers={"location": "http://169.254.169.254/latest/meta-data/"})

    with pytest.raises(FetchBlocked) as e:
        await get(handler)
    assert e.value.reason == "private_address"
    assert len(seen) == 1, "only the first hop was ever requested"


async def test_a_redirect_to_a_name_that_resolves_privately_is_refused():
    with pytest.raises(FetchBlocked) as e:
        await get(lambda r: httpx.Response(301, headers={"location": "https://internal.test/admin"}))
    assert e.value.reason == "private_address"


async def test_redirects_are_followed_and_revalidated_per_hop():
    hops = []

    def handler(req):
        hops.append(req.headers["host"])
        if len(hops) == 1:
            return httpx.Response(301, headers={"location": "https://spokeo.com/a/"})
        return httpx.Response(200, text="final", headers={"content-type": "text/html"})

    r = await get(handler, allowed_domain="spokeo.com")
    assert r.text == "final" and r.status == 200
    assert hops == ["www.spokeo.com", "spokeo.com"]
    assert not r.moved, "www / trailing-slash changes are not 'a different page'"


async def test_a_redirect_off_the_source_domain_is_refused():
    with pytest.raises(FetchBlocked) as e:
        await get(lambda r: httpx.Response(302, headers={"location": "https://evil.example/x"}), allowed_domain="spokeo.com")
    assert e.value.reason == "redirect_off_domain"


async def test_the_first_request_must_be_on_the_source_domain_too():
    with pytest.raises(FetchBlocked) as e:
        await get(ok(), url="https://evil.example/x", allowed_domain="spokeo.com")
    assert e.value.reason == "host_not_allowed"
    with pytest.raises(FetchBlocked):
        await get(ok(), url="https://notspokeo.com/x", allowed_domain="spokeo.com")   # suffix, not subdomain


async def test_a_redirect_to_another_page_on_the_same_site_is_reported_as_moved():
    def handler(req):
        if req.url.path == "/profile/jane":
            return httpx.Response(302, headers={"location": "/"})
        return httpx.Response(200, text="homepage", headers={"content-type": "text/html"})

    r = await get(handler, url="https://www.spokeo.com/profile/jane", allowed_domain="spokeo.com")
    assert r.moved is True


async def test_redirect_loops_stop():
    with pytest.raises(FetchBlocked) as e:
        await get(lambda r: httpx.Response(302, headers={"location": "/again"}), max_redirects=3)
    assert e.value.reason == "too_many_redirects"


# ---- the connection is pinned to the address that was checked -------------------------

async def test_the_request_goes_to_the_validated_address_with_the_original_host_header():
    seen = {}

    def handler(req):
        seen["url_host"] = req.url.host
        seen["host_header"] = req.headers["host"]
        seen["sni"] = req.extensions.get("sni_hostname")
        return httpx.Response(200, text="x", headers={"content-type": "text/html"})

    await get(handler, url="https://www.spokeo.com/a")
    assert seen == {"url_host": "93.184.216.34", "host_header": "www.spokeo.com", "sni": "www.spokeo.com"}, (
        "connect to the checked IP, but name the original host so TLS verifies the right certificate"
    )


# ---- size and type limits -------------------------------------------------------------

async def test_a_body_over_the_cap_is_refused_not_read_into_memory():
    big = "x" * 5000
    with pytest.raises(ResponseTooLarge):
        await get(ok(big), max_bytes=1000)


async def test_a_declared_length_over_the_cap_is_refused_up_front():
    with pytest.raises(ResponseTooLarge):
        await get(lambda r: httpx.Response(200, headers={"content-length": "999999999", "content-type": "text/html"}, content=b"x"), max_bytes=1000)


async def test_the_cap_applies_to_the_decompressed_size():
    bomb = gzip.compress(b"A" * 5_000_000)   # tiny on the wire, huge when inflated
    assert len(bomb) < 10_000
    with pytest.raises(ResponseTooLarge):
        await get(lambda r: httpx.Response(200, content=bomb, headers={"content-encoding": "gzip", "content-type": "text/html"}), max_bytes=100_000)


async def test_binary_content_is_not_read():
    r = await get(lambda r: httpx.Response(200, content=b"\x00" * 100, headers={"content-type": "application/pdf"}))
    assert r.text == "" and r.status == 200


async def test_an_unknown_charset_does_not_crash():
    r = await get(lambda r: httpx.Response(200, content=b"caf\xc3\xa9", headers={"content-type": "text/html; charset=bogus-9"}))
    assert r.text == "café"


async def test_non_200_statuses_are_returned_not_raised():
    for status in (403, 404, 429, 500):
        assert (await get(lambda r, s=status: httpx.Response(s, text="no"))).status == status


async def test_errors_never_contain_the_url():
    url = "https://internal.test/people/jane-smith-5125550147"
    with pytest.raises(FetchBlocked) as e:
        await get(ok(), url=url)
    assert "jane" not in str(e.value).lower() and "internal.test" not in str(e.value)
