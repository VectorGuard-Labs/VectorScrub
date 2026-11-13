"""
Fetching third-party pages without letting the page (or a redirect) steer us at our own network.

Discovery and monitoring GET URLs that come from data brokers' sites and from the database.
A plain `httpx` call with `follow_redirects=True` will happily follow a redirect to
`http://169.254.169.254/` (cloud metadata), to the backend, to Redis, or to anything else on
the private network, and will read an unbounded response into memory. This module is the one
place that makes such a request, and it:

  * allows only http/https on the standard ports, with no credentials in the URL;
  * optionally confines every hop (the first request AND each redirect) to one domain;
  * resolves the host itself and refuses unless EVERY address is public (no loopback,
    private, link-local, CGNAT, multicast, reserved or unspecified ranges, including
    IPv4-mapped IPv6 forms);
  * connects to the address it validated, not to whatever the name resolves to a moment
    later, so DNS rebinding cannot swap in a private address between check and connect.
    TLS still verifies the certificate against the original hostname (SNI is set to it);
  * follows redirects itself, re-validating every hop, up to a small limit;
  * caps the size of the DECOMPRESSED body, so neither a huge page nor a zip bomb can
    exhaust memory, and caps total time so a slow drip cannot hold a worker.

Nothing in the exceptions raised here contains the URL or any page text: only a fixed
reason, so they are safe to log (log the host, never the URL).
"""
from __future__ import annotations

import asyncio
import ipaddress
import socket
from dataclasses import dataclass
from urllib.parse import urljoin, urlsplit, urlunsplit

import httpx

MAX_BODY_BYTES = 2_000_000
MAX_REDIRECTS = 5
TOTAL_TIMEOUT_SECONDS = 45.0

_ALLOWED_SCHEMES = {"http": 80, "https": 443}


class FetchBlocked(Exception):
    """The request was refused before or while being made. `reason` is a fixed, loggable code."""

    def __init__(self, reason: str) -> None:
        super().__init__(reason)
        self.reason = reason


class ResponseTooLarge(Exception):
    """The body is larger than the cap."""


@dataclass(frozen=True)
class Fetched:
    status: int
    text: str
    final_host: str
    # True when redirects led to a different page (path) than the one requested. Trailing
    # slashes, case, http->https and www changes do not count.
    moved: bool
    content_type: str
    # Path of the page finally served (lowercased). Callers use it only to test locally; it is
    # never logged (profile paths contain names).
    final_path: str = ""


# ---------------------------------------------------------------------------
# Address checks
# ---------------------------------------------------------------------------

def is_public_address(address: str) -> bool:
    """True only for a globally routable unicast address."""
    try:
        ip = ipaddress.ip_address(address.split("%", 1)[0])
    except ValueError:
        return False
    if isinstance(ip, ipaddress.IPv6Address) and ip.ipv4_mapped is not None:
        ip = ip.ipv4_mapped
    return ip.is_global and not ip.is_multicast


async def _resolve(host: str, port: int) -> list[str]:
    """Addresses for a hostname. Tests replace this; production uses the system resolver."""
    loop = asyncio.get_running_loop()
    infos = await loop.getaddrinfo(host, port, type=socket.SOCK_STREAM)
    return list(dict.fromkeys(info[4][0].split("%", 1)[0] for info in infos))


async def _validated_addresses(host: str, port: int) -> list[str]:
    try:
        ipaddress.ip_address(host)
        addresses = [host]  # an IP literal in the URL is checked as it is
    except ValueError:
        try:
            addresses = await _resolve(host, port)
        except (OSError, UnicodeError):
            raise FetchBlocked("unresolvable") from None
    if not addresses:
        raise FetchBlocked("unresolvable")
    # ALL answers must be public: a name that also resolves to a private address is refused
    # rather than hoping the client picks the safe one.
    if not all(is_public_address(a) for a in addresses):
        raise FetchBlocked("private_address")
    return addresses


# ---------------------------------------------------------------------------
# URL checks
# ---------------------------------------------------------------------------

def _in_domain(host: str, domain: str) -> bool:
    host, domain = host.lower().rstrip("."), domain.lower().rstrip(".")
    return host == domain or host.endswith("." + domain)


def _check_url(url: str, allowed_domain: str | None, hop: int):
    try:
        parts = urlsplit(url)
        port = parts.port  # raises ValueError on a malformed port
    except ValueError:
        raise FetchBlocked("bad_url") from None
    scheme = parts.scheme.lower()
    if scheme not in _ALLOWED_SCHEMES:
        raise FetchBlocked("scheme")
    if parts.username is not None or parts.password is not None:
        raise FetchBlocked("credentials_in_url")
    host = parts.hostname
    if not host:
        raise FetchBlocked("bad_url")
    if port is not None and port != _ALLOWED_SCHEMES[scheme]:
        raise FetchBlocked("port")
    if allowed_domain and not _in_domain(host, allowed_domain):
        raise FetchBlocked("host_not_allowed" if hop == 0 else "redirect_off_domain")
    return parts, scheme, host, port or _ALLOWED_SCHEMES[scheme]


def _norm_path(url: str) -> str:
    return urlsplit(url).path.lower().rstrip("/")


def _pinned(parts, scheme: str, address: str) -> str:
    """The same URL with the host replaced by the validated address."""
    netloc = f"[{address}]" if ":" in address else address
    return urlunsplit((scheme, netloc, parts.path or "/", parts.query, ""))


def _charset(content_type: str) -> str:
    for piece in content_type.split(";")[1:]:
        key, _, value = piece.strip().partition("=")
        if key.lower() == "charset" and value:
            return value.strip("\"'")
    return "utf-8"


def _decode(body: bytes, content_type: str) -> str:
    try:
        return body.decode(_charset(content_type), errors="replace")
    except LookupError:  # a charset name Python does not know
        return body.decode("utf-8", errors="replace")


# ---------------------------------------------------------------------------
# The fetch
# ---------------------------------------------------------------------------

async def safe_get(
    client: httpx.AsyncClient,
    url: str,
    *,
    headers: dict[str, str],
    timeout: float,
    allowed_domain: str | None = None,
    max_bytes: int = MAX_BODY_BYTES,
    max_redirects: int = MAX_REDIRECTS,
    total_timeout: float = TOTAL_TIMEOUT_SECONDS,
) -> Fetched:
    """
    GET `url` with the protections described in the module docstring.

    Raises FetchBlocked (refused), ResponseTooLarge, or an httpx error / asyncio.TimeoutError
    for transport trouble. A non-200 status is returned, not raised.
    """
    return await asyncio.wait_for(
        _safe_get(client, url, headers, timeout, allowed_domain, max_bytes, max_redirects),
        timeout=total_timeout,
    )


async def _safe_get(client, url, headers, timeout, allowed_domain, max_bytes, max_redirects) -> Fetched:
    original = url
    current = url
    for hop in range(max_redirects + 1):
        parts, scheme, host, port = _check_url(current, allowed_domain, hop)
        addresses = await _validated_addresses(host, port)

        host_header = host if parts.port is None else f"{host}:{parts.port}"
        extensions = {"sni_hostname": host} if scheme == "https" else {}
        response = None
        last_error: Exception | None = None
        for address in addresses:
            try:
                request = client.build_request(
                    "GET",
                    _pinned(parts, scheme, address),
                    headers={**headers, "Host": host_header},
                    timeout=timeout,
                    extensions=extensions,
                )
                response = await client.send(request, stream=True, follow_redirects=False)
                break
            except (httpx.ConnectError, httpx.ConnectTimeout) as exc:
                last_error = exc  # try the next address (e.g. IPv6 unreachable, IPv4 fine)
        if response is None:
            raise last_error  # type: ignore[misc]

        try:
            location = response.headers.get("location")
            if response.status_code in (301, 302, 303, 307, 308) and location:
                current = urljoin(current, location)
                continue

            content_type = response.headers.get("content-type", "")
            declared = response.headers.get("content-length")
            if declared and declared.isdigit() and int(declared) > max_bytes:
                raise ResponseTooLarge()
            # Only textual bodies are read. A binary download is not a page we can judge.
            textual = (not content_type) or any(t in content_type.lower() for t in ("text/", "html", "xml", "json"))
            body = bytearray()
            if textual:
                async for chunk in response.aiter_bytes():  # decoded (decompressed) bytes
                    body.extend(chunk)
                    if len(body) > max_bytes:
                        raise ResponseTooLarge()
            text = _decode(bytes(body), content_type)
            return Fetched(
                status=response.status_code,
                text=text,
                final_host=host,
                moved=_norm_path(current) != _norm_path(original),
                content_type=content_type,
                final_path=urlsplit(current).path.lower(),
            )
        finally:
            await response.aclose()
    raise FetchBlocked("too_many_redirects")
