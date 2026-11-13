"""
Finds CANDIDATE deletion-request email addresses on the companies' own privacy-rights pages.

This only proposes. It reads the privacy-request page(s) each company named in the California
registry, pulls out email addresses on the company's own sites with the text around them, and
ranks them by how much the surrounding words look like "email us to delete your data". A person
must still read the page and decide, because the words around an address are easy to misjudge:
the same page often has a DPO address for EU residents, a support address, and a press contact.

    python -m deletion.find_channels [--limit N] [--out candidates.json]

A candidate becomes a channel only when someone adds it to backend/data/broker_channels.json with
a quote from the page that contains the address; the backend refuses an entry without one.

Same conduct as the other page checks: public pages only, an honest bot user agent, the redirect
and size limits of shared.safe_fetch, a few sites at a time and never two requests to one site at
once. It does not try to get past a site that refuses it.
"""
from __future__ import annotations

import asyncio
import json
import re
import sys
from collections import defaultdict
from dataclasses import asdict, dataclass, field
from urllib.parse import urlsplit

import httpx
from bs4 import BeautifulSoup

from ca_registry.importer import read_committed
from shared.page_text import extract_text
from shared.safe_fetch import FetchBlocked, ResponseTooLarge, safe_get

HEADERS = {"User-Agent": "Mozilla/5.0 (compatible; VectorScrubBot/1.0; +https://vectorscrub.com/bot)"}

EMAIL = re.compile(r"[A-Za-z0-9._%+\-]+@[A-Za-z0-9\-]+(?:\.[A-Za-z0-9\-]+)+")
DELETE = re.compile(r"\b(delet\w*|eras\w*|remov\w*)", re.I)
RIGHTS = re.compile(r"(privacy request|consumer request|your (privacy )?rights|ccpa|cpra|data subject|opt[- ]?out|do not sell|verifiable)", re.I)
AGENT = re.compile(r"authorized agent", re.I)
EU = re.compile(r"\b(gdpr|eea|european|data protection officer|dpo|uk gdpr|united kingdom|lgpd|pipeda|eu\b)", re.I)
GENERAL = re.compile(r"\b(customer (service|support|care)|general (inquir|question)|press|media|billing|sales|advertis\w*|partner\w*|jobs|careers|abuse)\b", re.I)
JUNK_LOCAL = re.compile(r"^(no-?reply|donotreply|webmaster|postmaster|abuse|noc|admin|support|info|help|sales|press|media|jobs|careers|billing|hello|contact)$", re.I)

def registrable(host: str) -> str:
    labels = host.lower().strip(".").split(".")
    return ".".join(labels[-2:])


@dataclass
class Candidate:
    email: str
    page: str
    context: str
    delete: bool
    rights: bool
    agent: bool
    eu: bool
    general: bool
    generic_local: bool
    mailto: bool
    score: int = 0


@dataclass
class CompanyResult:
    id: str
    name: str
    pages_read: int = 0
    pages_failed: int = 0
    candidates: list[Candidate] = field(default_factory=list)


def score(c: Candidate) -> int:
    s = 0
    s += 3 if c.delete else 0
    s += 2 if c.rights else 0
    s += 1 if c.agent else 0
    s += 1 if c.mailto else 0
    s -= 3 if c.eu else 0
    s -= 2 if c.general else 0
    s -= 1 if c.generic_local else 0
    return s


BLOCK_TAGS = ("p", "li", "td", "dd", "blockquote", "section", "article", "div")
MIN_BLOCK_CHARS = 60
MAX_BLOCK_CHARS = 450


def block_text(node) -> str:
    """The paragraph-sized block a piece of text sits in, as one line: the nearest enclosing block
    with enough words to be a sentence, so neighbouring paragraphs (a DPO line, a press contact)
    are not mixed into the judgement."""
    block = node.parent if getattr(node, "parent", None) is not None else None
    while block is not None:
        text = " ".join(block.get_text(" ", strip=True).split())
        if block.name in BLOCK_TAGS and len(text) >= MIN_BLOCK_CHARS:
            return text[:MAX_BLOCK_CHARS]
        if block.name in ("body", "html", "[document]"):
            return text[:MAX_BLOCK_CHARS]
        block = block.parent
    return ""


def extract_candidates(html: str, page_url: str, own_domains: set[str]) -> list[Candidate]:
    """Email addresses on the company's own domains, each with the paragraph it appears in."""
    try:
        soup = BeautifulSoup(html, "html.parser")
        for element in soup(["script", "style", "noscript"]):
            element.decompose()
    except Exception:
        return []

    found: dict[str, Candidate] = {}

    def add(email: str, context: str, mailto: bool) -> None:
        email = email.strip(".").lower()
        if email in found or registrable(email.rpartition("@")[2]) not in own_domains:
            return
        c = Candidate(
            email=email, page=page_url, context=context,
            delete=bool(DELETE.search(context)), rights=bool(RIGHTS.search(context)), agent=bool(AGENT.search(context)),
            eu=bool(EU.search(context)), general=bool(GENERAL.search(context)),
            generic_local=bool(JUNK_LOCAL.match(email.partition("@")[0])), mailto=mailto,
        )
        c.score = score(c)
        found[email] = c

    mailto_addresses: set[str] = set()
    for a in soup.find_all("a", href=True):
        if a["href"].lower().startswith("mailto:"):
            mailto_addresses.add(a["href"][7:].split("?")[0].strip().lower())
    for node in soup.find_all(string=EMAIL):
        context = block_text(node)
        for m in EMAIL.finditer(str(node)):
            add(m.group(0), context, m.group(0).lower() in mailto_addresses)
    # an address only present in a mailto: link (the visible text is "click here")
    for a in soup.find_all("a", href=True):
        if a["href"].lower().startswith("mailto:"):
            email = a["href"][7:].split("?")[0].strip()
            if EMAIL.fullmatch(email):
                add(email, block_text(a), True)
    return list(found.values())


async def scan(
    brokers: list[dict], concurrency: int = 4, delay: float = 0.25, client: httpx.AsyncClient | None = None
) -> list[CompanyResult]:
    gate = asyncio.Semaphore(concurrency)
    host_locks: dict[str, asyncio.Lock] = defaultdict(asyncio.Lock)
    results = {b["id"]: CompanyResult(id=b["id"], name=b["name"]) for b in brokers}

    async def one(c: httpx.AsyncClient, broker: dict, url: str) -> None:
        own = {registrable(s) for s in broker["sites"]} | {registrable(urlsplit(u).hostname or "") for u in broker["rights"]}
        host = (urlsplit(url).hostname or "").lower().removeprefix("www.")
        res = results[broker["id"]]
        async with gate, host_locks[host]:
            try:
                fetched = await safe_get(c, url, headers=HEADERS, timeout=25, allowed_domain=host, total_timeout=40)
            except (FetchBlocked, ResponseTooLarge, httpx.HTTPError, asyncio.TimeoutError):
                res.pages_failed += 1
            else:
                if fetched.status == 200:
                    res.pages_read += 1
                    for cand in extract_candidates(fetched.text, url, own):
                        if all(cand.email != k.email for k in res.candidates):
                            res.candidates.append(cand)
                else:
                    res.pages_failed += 1
            await asyncio.sleep(delay)

    async def run(c: httpx.AsyncClient) -> None:
        await asyncio.gather(*(one(c, b, u) for b in brokers for u in b["rights"]), return_exceptions=True)

    if client is not None:
        await run(client)
    else:
        async with httpx.AsyncClient() as c:
            await run(c)
    for r in results.values():
        r.candidates.sort(key=lambda c: -c.score)
    return list(results.values())


def main(argv: list[str]) -> int:
    limit = int(argv[argv.index("--limit") + 1]) if "--limit" in argv else None
    out = argv[argv.index("--out") + 1] if "--out" in argv else "candidates.json"
    brokers, _ = read_committed()
    if limit:
        brokers = brokers[:limit]
    results = asyncio.run(scan(brokers))
    with open(out, "w", encoding="utf-8") as fh:
        json.dump([asdict(r) for r in results], fh, indent=1, ensure_ascii=False)
    read = sum(r.pages_read for r in results)
    failed = sum(r.pages_failed for r in results)
    with_cands = sum(1 for r in results if r.candidates)
    print(f"{len(results)} companies; {read} pages read, {failed} could not be read; "
          f"{with_cands} companies with at least one address on their own domain. Written to {out}.")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
