"""
Turning a fetched broker page into text we can reason about.

Shared by the discovery agent (which finds candidate pages) and the monitoring agent
(which re-checks them), so both use one implementation of: HTML to text, bot-wall
detection, and selecting the part of a long page that is about the customer.

Nothing here talks to a model. Whatever is chosen is redacted by shared.pii_guard before
any LLM sees it.
"""
from __future__ import annotations

import re

import structlog
from bs4 import BeautifulSoup

from shared.pii_guard import fold, name_parts

log = structlog.get_logger(__name__)

# Pages with less text than this are almost certainly JS-rendered or blocked.
MIN_PAGE_TEXT_CHARS = 100
# Bot-wall / challenge pages are short; only treat marker words as a block when the page is
# also short, so an ordinary page that mentions "captcha" is fine.
BLOCK_PAGE_MAX_CHARS = 3000
BLOCK_MARKERS = (
    "captcha",
    "access denied",
    "verify you are human",
    "unusual traffic",
    "are you a robot",
    "checking your browser",
    "just a moment",
    "attention required",
)
# Characters of context kept before the first occurrence of the surname.
LEAD_IN_CHARS = 500


def extract_text(html: str) -> str:
    """Clean, whitespace-normalized visible text from HTML ('' if it cannot be parsed)."""
    try:
        soup = BeautifulSoup(html, "html.parser")
        for element in soup(["script", "style", "noscript"]):
            element.decompose()
        text = soup.get_text(separator=" ", strip=True)
        return " ".join(text.split())
    except Exception as exc:
        log.warning("text_extraction_failed", error_type=type(exc).__name__)
        return ""


def looks_blocked(text: str) -> bool:
    """True for short bot-wall / CAPTCHA / challenge pages."""
    if len(text) > BLOCK_PAGE_MAX_CHARS:
        return False
    lowered = text.lower()
    return any(marker in lowered for marker in BLOCK_MARKERS)


def surname_of(full_name: str) -> str:
    """Last name of a full name, ignoring generational suffixes; a single name is returned as is."""
    first, last, _ = name_parts(full_name)
    return last or first


def window_around_surname(page_text: str, surname: str, max_chars: int) -> str | None:
    """
    Up to max_chars of the ORIGINAL page text, starting just before the first occurrence of
    the surname (matched accent- and case-insensitively), or None if it never appears.

    Done locally so (a) a page that never mentions the customer is "absent" without an LLM
    call, and (b) the excerpt is the part of a long page that is about the customer rather
    than only the first screenful.
    """
    needle = fold(surname)
    if not needle:
        return None
    match = re.search(rf"\b{re.escape(needle)}\b", fold(page_text))
    if not match:
        return None
    start = max(0, match.start() - LEAD_IN_CHARS)
    return page_text[start : start + max_chars]
