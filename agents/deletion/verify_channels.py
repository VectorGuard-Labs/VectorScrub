"""
Re-checks the deletion-request channel catalog against the companies' own pages.

A channel is only usable because the company published its address on a page we cite. Companies
change their pages. This fetches each channel's `source_url` and confirms the address is still
there, so a dead or changed channel is found by us before a customer's request goes to it.

    python -m deletion.verify_channels            # check every channel
    python -m deletion.verify_channels --strict   # also fail when a page could not be checked

Results, per channel:
  confirmed       the page loaded and still shows the address
  address_missing the page loaded and does NOT show the address: review it, and remove the channel
                  or fix the entry. Exit status 1.
  could_not_check the page refused us, timed out or was unreadable (many sites block automated
                  requests). This says nothing about the address; check the page by hand.
  stale           the entry's verified_on date is older than STALE_AFTER_DAYS: re-verify by hand
                  and update `verified_on`. (Reported in addition to the above.)

It never edits the catalog: a person confirms a channel and writes the date.
"""
from __future__ import annotations

import asyncio
import json
import sys
from dataclasses import dataclass
from datetime import date
from pathlib import Path

import httpx

from shared.page_text import extract_text
from shared.safe_fetch import FetchBlocked, ResponseTooLarge, safe_get

CATALOG = Path(__file__).resolve().parents[2] / "backend" / "data" / "broker_channels.json"
HEADERS = {"User-Agent": "Mozilla/5.0 (compatible; VectorScrubBot/1.0; +https://vectorscrub.com/bot)"}
STALE_AFTER_DAYS = 180


@dataclass
class ChannelCheck:
    id: str
    result: str          # confirmed | address_missing | could_not_check
    stale: bool
    detail: str


def load_catalog(path: Path = CATALOG) -> list[dict]:
    return list(json.loads(path.read_text(encoding="utf-8")).get("channels", []))


def is_stale(verified_on: str, today: date | None = None) -> bool:
    try:
        return ((today or date.today()) - date.fromisoformat(verified_on)).days > STALE_AFTER_DAYS
    except ValueError:
        return True


async def check_channel(client: httpx.AsyncClient, channel: dict, today: date | None = None) -> ChannelCheck:
    cid, address = channel["id"], channel["address"].lower()
    stale = is_stale(channel.get("verified_on", ""), today)
    host = channel["source_url"].split("/")[2].removeprefix("www.")
    try:
        fetched = await safe_get(client, channel["source_url"], headers=HEADERS, timeout=30, allowed_domain=host)
    except FetchBlocked as exc:
        return ChannelCheck(cid, "could_not_check", stale, f"refused ({exc.reason})")
    except ResponseTooLarge:
        return ChannelCheck(cid, "could_not_check", stale, "page too large")
    except (httpx.HTTPError, asyncio.TimeoutError) as exc:
        return ChannelCheck(cid, "could_not_check", stale, type(exc).__name__)
    if fetched.status != 200:
        return ChannelCheck(cid, "could_not_check", stale, f"HTTP {fetched.status}")
    # The address may sit in markup (a mailto: link) or in visible text; either counts.
    if address in fetched.text.lower() or address in extract_text(fetched.text).lower():
        return ChannelCheck(cid, "confirmed", stale, "address found on the page")
    return ChannelCheck(cid, "address_missing", stale, "the page loaded but does not show the address")


async def verify(channels: list[dict], today: date | None = None, client: httpx.AsyncClient | None = None) -> list[ChannelCheck]:
    async def run(c: httpx.AsyncClient) -> list[ChannelCheck]:
        out = []
        for ch in channels:                    # one at a time: a few pages, no burst
            out.append(await check_channel(c, ch, today))
        return out

    if client is not None:
        return await run(client)
    async with httpx.AsyncClient() as c:
        return await run(c)


def exit_code(checks: list[ChannelCheck], strict: bool = False) -> int:
    if any(c.result == "address_missing" for c in checks):
        return 1
    if strict and any(c.result == "could_not_check" or c.stale for c in checks):
        return 1
    return 0


def main(argv: list[str]) -> int:
    strict = "--strict" in argv[1:]
    channels = load_catalog()
    checks = asyncio.run(verify(channels))
    for c in checks:
        flag = "  [STALE: re-verify by hand and update verified_on]" if c.stale else ""
        print(f"{c.id:20} {c.result:16} {c.detail}{flag}")
    return exit_code(checks, strict)


if __name__ == "__main__":
    sys.exit(main(sys.argv))
