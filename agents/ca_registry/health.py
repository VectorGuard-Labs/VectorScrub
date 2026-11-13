"""
Do the privacy-request pages named in the California Data Broker Registry still open?

Each company in the registry names a page where consumers can send privacy requests. This
checks, once a day, that each of those pages still loads, so the directory in the opt-out guide
can say "this link worked on <date>" and show "link not working" for the ones that have died.

Same rules as the checks of our own brokers' pages (removal/health.py, which this reuses):
  * public pages only, one request per company, nothing submitted, no customer data;
  * the redirect rules and size limits of shared.safe_fetch; an honest bot user agent;
  * a plain HTTP request, so "opens" is a good sign and not a promise: some sites treat it
    differently from a person in a browser;
  * it does not try to get past bot protection. A site that refuses is reported as blocked.

A few companies in the registry run several sites or share one: requests to the same host are
serialised with a pause, and only a few hosts are fetched at once, so no site sees a burst.
"""
from __future__ import annotations

import asyncio
import json
from collections import defaultdict
from pathlib import Path
from urllib.parse import urlsplit

import httpx
import structlog

from removal.health import OptOutHealthChecker
from shared.config import get_config
from shared.models import OptOutCheck, OptOutKind, OptOutStatus

log = structlog.get_logger(__name__)

PAGES_FILE = Path(__file__).resolve().parent / "rights_pages.json"

# The backend accepts at most this many checks in one report.
REPORT_CHUNK = 200


def load_pages(path: Path = PAGES_FILE) -> list[tuple[str, str]]:
    """(registry id, url) pairs from the committed file. Anything that is not a plain
    id and an http(s) address is skipped."""
    try:
        raw = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return []
    pages: list[tuple[str, str]] = []
    for item in raw if isinstance(raw, list) else []:
        if not isinstance(item, dict):
            continue
        ident, url = item.get("id"), item.get("url")
        if isinstance(ident, str) and isinstance(url, str) and ident.isalnum() and url.startswith(("http://", "https://")):
            pages.append((ident, url))
    return pages


def chunks(items: list, size: int = REPORT_CHUNK):
    for i in range(0, len(items), size):
        yield items[i : i + size]


class CaRightsHealthChecker:
    def __init__(self, concurrency: int | None = None, delay_seconds: float | None = None) -> None:
        cfg = get_config()
        self._concurrency = max(1, cfg.ca_rights_health_concurrency if concurrency is None else concurrency)
        self._delay = cfg.ca_rights_health_delay_seconds if delay_seconds is None else delay_seconds
        self._pages = OptOutHealthChecker(delay_seconds=0)

    async def check_all(self, pages: list[tuple[str, str]] | None = None) -> list[OptOutCheck]:
        """Check every page. Never raises for one bad page; the result keeps the input order."""
        pages = load_pages() if pages is None else pages
        gate = asyncio.Semaphore(self._concurrency)
        host_locks: dict[str, asyncio.Lock] = defaultdict(asyncio.Lock)
        results: dict[str, OptOutCheck] = {}

        async with httpx.AsyncClient() as client:

            async def one(ident: str, url: str) -> None:
                host = (urlsplit(url).hostname or "").lower().removeprefix("www.")
                async with gate, host_locks[host]:
                    check = await self._pages.check_page(client, ident, url)
                    results[ident] = check.model_copy(update={"kind": OptOutKind.CA_RIGHTS})
                    if self._delay:
                        await asyncio.sleep(self._delay)

            await asyncio.gather(*(one(i, u) for i, u in pages), return_exceptions=True)

        # A page whose task raised something unexpected still gets an entry: unavailable.
        ordered = [
            results.get(i) or OptOutCheck(kind=OptOutKind.CA_RIGHTS, source=i, status=OptOutStatus.UNAVAILABLE)
            for i, _ in pages
        ]
        counts = {s.value: sum(1 for r in ordered if r.status is s) for s in OptOutStatus}
        log.info("ca_rights_health_checked", pages=len(ordered), **counts)
        return ordered
