"""
Outbound PII guard: the only path by which page text may reach an LLM.

Project rule: no customer PII is ever sent to an external LLM. Broker pages are
the problem case, because the page itself shows the customer's real name,
address, phone, and so on (and their relatives'). This module makes the rule
structural rather than a convention:

  1. redact_for_llm() replaces every customer value found in the text with a typed
     placeholder ([CUSTOMER_NAME], [CUSTOMER_ADDRESS], ...), and replaces other
     personal data (emails, phone numbers, SSNs, dates, street addresses, ZIP
     codes, ages, and person-like names such as relatives) with generic
     placeholders ([EMAIL], [PHONE], [NAME], ...). Matching is done here, locally,
     so the model never needs the customer's identity, not even masked hints.

  2. assert_clean() re-checks the final outbound string and raises
     OutboundPiiError if ANY customer value is still present. It runs after
     redaction and before the network call, so a redaction bug fails closed (the
     LLM is not called) instead of leaking.

Guarantee vs. best effort:
  * Customer values (name in common orderings, addresses, phones, emails, DOB)
    are GUARANTEED not to leave: redaction removes them and assert_clean() blocks
    the call if one survives.
  * Other people's details (relatives, neighbours, namesakes) are removed in three
    layers: pattern rules, then NER (shared/pii_ner.py), then a deny-by-default
    vocabulary filter that drops every word not on a short list of structural words
    (shared/pii_vocab.py). NER and patterns are probabilistic; the vocabulary filter
    is not: a third-party name survives only if it is spelled exactly like one of
    the allowed words, and that list is kept free of known names (checked against
    name corpora from every locale Faker ships). assert_vocabulary_only() re-checks
    the final excerpt and fails closed.

Nothing here logs or returns raw values: errors name categories only.
"""
from __future__ import annotations

import re
import unicodedata
from dataclasses import dataclass
from typing import Iterable

from shared.models import IdentityContext
from shared.pii_vocab import ALLOWED_WORDS, KNOWN_TOKENS


class OutboundPiiError(RuntimeError):
    """
    Raised, before any network call, when text bound for an LLM still contains a
    customer value. The message lists categories only, never the values.
    """


# --------------------------------------------------------------------------
# Folding: accent- and case-insensitive matching with positions preserved
# --------------------------------------------------------------------------

def _fold_char(c: str) -> str:
    base = "".join(x for x in unicodedata.normalize("NFKD", c) if not unicodedata.combining(x)).lower()
    if len(base) == 1:
        return base
    lowered = c.lower()
    return lowered if len(lowered) == 1 else c


def fold(text: str) -> str:
    """
    Lowercase and strip accents ('García' -> 'garcia') while keeping the string the
    same length, so match positions in the folded text index the original text.
    """
    return "".join(_fold_char(c) for c in text)


def _norm(text: str) -> str:
    """Folded, whitespace-normalized."""
    return " ".join(fold(text).split())


# --------------------------------------------------------------------------
# Patterns
# --------------------------------------------------------------------------

NAME_SUFFIXES = {"jr", "sr", "ii", "iii", "iv"}

_MONTHS = {
    "jan": 1, "feb": 2, "mar": 3, "apr": 4, "may": 5, "jun": 6,
    "jul": 7, "aug": 8, "sep": 9, "sept": 9, "oct": 10, "nov": 11, "dec": 12,
}
_MONTH_ALT = (
    r"jan(?:uary)?|feb(?:ruary)?|mar(?:ch)?|apr(?:il)?|may|june?|july?|aug(?:ust)?|"
    r"sept?(?:ember)?|oct(?:ober)?|nov(?:ember)?|dec(?:ember)?"
)

EMAIL_RE = re.compile(r"[a-z0-9._%+\-]+@[a-z0-9.\-]+\.[a-z]{2,}")
PHONE_RE = re.compile(r"(?<!\d)(?:\+?1[\s.\-]?)?(?:\(\d{3}\)|\d{3})[\s.\-]?\d{3}[\s.\-]?\d{4}(?!\d)")
SSN_RE = re.compile(r"(?<!\d)\d{3}-\d{2}-\d{4}(?!\d)")
ZIP_RE = re.compile(r"(?<!\d)\d{5}(?:-\d{4})?(?!\d)")
LONG_NUMBER_RE = re.compile(r"(?<!\d)\d{9,}(?!\d)")
AGE_RE = re.compile(
    r"\bages?\s*:?\s*\d{1,3}\b|\b\d{1,3}\s*(?:years?|yrs?)\s*old\b|\bborn\s+(?:in\s+)?\d{4}\b"
)
STREET_RE = re.compile(
    r"\b\d{1,6}\s+(?:[a-z0-9.'\-]+\s+){1,4}"
    r"(?:st|street|ave|avenue|rd|road|blvd|boulevard|dr|drive|ln|lane|ct|court|way|pl|place|"
    r"ter|terrace|cir|circle|hwy|highway|pkwy|parkway|trl|trail)\b\.?"
)
PO_BOX_RE = re.compile(r"\bp\.?\s?o\.?\s+box\s+\d+\b")

DATE_ISO_RE = re.compile(r"(?<!\d)(\d{4})-(\d{2})-(\d{2})(?!\d)")
DATE_NUM_RE = re.compile(r"(?<!\d)(\d{1,2})[/\-.](\d{1,2})[/\-.](\d{4}|\d{2})(?!\d)")
DATE_MDY_RE = re.compile(rf"\b({_MONTH_ALT})\.?\s+(\d{{1,2}})(?:st|nd|rd|th)?,?\s+(\d{{4}})\b")
DATE_DMY_RE = re.compile(rf"\b(\d{{1,2}})(?:st|nd|rd|th)?\s+({_MONTH_ALT})\.?,?\s+(\d{{4}})\b")
DATE_MY_RE = re.compile(rf"\b({_MONTH_ALT})\.?,?\s+(\d{{4}})\b")

# Person-like names, matched on the ORIGINAL text (capitalization matters): two to
# four consecutive capitalized words/initials, e.g. "Robert Smith", "Mary A. Jones".
_UP = "A-ZÀ-ÖØ-Þ"
_LOW = "a-zß-öø-ÿ"
_WORD = rf"[{_UP}][{_UP}{_LOW}'’\-]+"
NAME_RE = re.compile(rf"\b{_WORD}(?:\s+(?:[{_UP}]\.|{_WORD})){{1,3}}\b")


def _month(token: str) -> int:
    return _MONTHS[token[:4] if token.startswith("sept") else token[:3]]


def _dates_in(folded: str) -> Iterable[tuple[int, int, tuple[int, ...]]]:
    """
    Yield (start, end, candidates) for date-like strings. `candidates` is a tuple of
    (year, month, day) triples the string could mean (numeric dates are ambiguous
    between M/D and D/M); day is 0 for month-year strings.
    """
    for m in DATE_ISO_RE.finditer(folded):
        y, mo, d = int(m[1]), int(m[2]), int(m[3])
        yield m.start(), m.end(), ((y, mo, d),)
    for m in DATE_NUM_RE.finditer(folded):
        a, b, y = int(m[1]), int(m[2]), int(m[3])
        # Ambiguous between M/D/Y and D/M/Y: offer both readings.
        yield m.start(), m.end(), ((y, a, b), (y, b, a))
    for m in DATE_MDY_RE.finditer(folded):
        yield m.start(), m.end(), ((int(m[3]), _month(m[1]), int(m[2])),)
    for m in DATE_DMY_RE.finditer(folded):
        yield m.start(), m.end(), ((int(m[3]), _month(m[2]), int(m[1])),)
    for m in DATE_MY_RE.finditer(folded):
        yield m.start(), m.end(), ((int(m[2]), _month(m[1]), 0),)


def parse_dob(value: str | None) -> tuple[int, int, int] | None:
    """Parse the customer's stored date of birth into (year, month, day)."""
    if not value:
        return None
    folded = fold(value.strip())
    for _, _, candidates in _dates_in(folded):
        for y, mo, d in candidates:
            if d and 1 <= mo <= 12 and 1 <= d <= 31 and y > 31:
                return y, mo, d
    return None


def _date_is_dob(candidates: tuple[tuple[int, int, int], ...], dob: tuple[int, int, int]) -> bool:
    y, mo, d = dob
    for cy, cm, cd in candidates:
        year_ok = cy == y or (cy < 100 and cy == y % 100)
        if year_ok and cm == mo and (cd == d or cd == 0):
            return True
    return False


# --------------------------------------------------------------------------
# Customer value extraction
# --------------------------------------------------------------------------

def name_parts(full_name: str) -> tuple[str, str, list[str]]:
    """(first, last, middles) from a full name, ignoring generational suffixes."""
    parts = [p.strip(".") for p in re.split(r"[\s,]+", (full_name or "").strip())]
    parts = [p for p in parts if p]
    while len(parts) > 1 and parts[-1].lower() in NAME_SUFFIXES:
        parts.pop()
    if not parts:
        return "", "", []
    if len(parts) == 1:
        return parts[0], "", []
    return parts[0], parts[-1], parts[1:-1]


def _addresses(identity: IdentityContext) -> list[str]:
    values = [identity.current_address, *identity.previous_addresses]
    return [v for v in values if v and v.strip()]


def _emails(identity: IdentityContext) -> list[str]:
    values = [*identity.email_addresses, identity.email or ""]
    return [fold(v.strip()) for v in values if v and v.strip()]


def _phone_digits(identity: IdentityContext) -> set[str]:
    out = set()
    for p in identity.phone_numbers:
        digits = re.sub(r"\D", "", p or "")
        if len(digits) >= 10:
            out.add(digits[-10:])
    return out


_ADDRESS_EQUIV = {
    "st": "st|street", "street": "st|street", "ave": "ave|avenue", "avenue": "ave|avenue",
    "rd": "rd|road", "road": "rd|road", "blvd": "blvd|boulevard", "boulevard": "blvd|boulevard",
    "dr": "dr|drive", "drive": "dr|drive", "ln": "ln|lane", "lane": "ln|lane",
    "ct": "ct|court", "court": "ct|court", "pl": "pl|place", "place": "pl|place",
    "ter": "ter|terrace", "terrace": "ter|terrace", "cir": "cir|circle", "circle": "cir|circle",
    "hwy": "hwy|highway", "highway": "hwy|highway", "pkwy": "pkwy|parkway", "parkway": "pkwy|parkway",
    "trl": "trl|trail", "trail": "trl|trail",
    "n": "n|north", "north": "n|north", "s": "s|south", "south": "s|south",
    "e": "e|east", "east": "e|east", "w": "w|west", "west": "w|west",
}


def _address_pattern(address: str) -> re.Pattern[str] | None:
    """
    Tolerant pattern for an address: tokens in order, any run of space/comma/period
    between them, and street suffixes/directionals interchangeable with their long
    forms ("St" = "Street", "N" = "North").
    """
    tokens = [t.rstrip(".") for t in re.split(r"[\s,]+", fold(address).strip()) if t.rstrip(".")]
    if len(tokens) < 2:
        return None
    parts = [f"(?:{_ADDRESS_EQUIV[t]})" if t in _ADDRESS_EQUIV else re.escape(t) for t in tokens]
    return re.compile(r"\b" + r"[\s,.]+".join(parts) + r"\b")


# --------------------------------------------------------------------------
# Redaction
# --------------------------------------------------------------------------

# Lower number wins when spans overlap.
_P_CUSTOMER = 0   # full name, address, phone, email, DOB belonging to the customer
_P_GENERIC = 1    # other people's data (and person-like names, e.g. relatives)
_P_FRAGMENT = 2   # lone surname / first name; loses to a longer span such as "Bob Smith"


@dataclass(frozen=True)
class _Span:
    start: int
    end: int
    token: str
    priority: int


@dataclass
class Evidence:
    """What was found in the text, as flags and counts only (no values)."""
    name_full: bool = False
    surname: bool = False
    first_name: bool = False
    address: bool = False
    phone: bool = False
    email: bool = False
    dob: bool = False
    other_people: int = 0


@dataclass
class Redacted:
    text: str
    evidence: Evidence


def _customer_spans(folded: str, identity: IdentityContext) -> list[_Span]:
    spans: list[_Span] = []
    first, last, middles = name_parts(identity.full_name)
    f_first, f_last = fold(first), fold(last)
    f_middles = [fold(m) for m in middles]

    if f_first and f_last:
        mid_alt = "|".join(re.escape(m) for m in f_middles if len(m) > 1)
        mid = rf"(?:[\s.]+(?:{mid_alt + '|' if mid_alt else ''}[a-z]\.?))*"
        forward = re.compile(rf"\b{re.escape(f_first)}{mid}[\s.]+{re.escape(f_last)}\b")
        reverse = re.compile(rf"\b{re.escape(f_last)}\s*,\s*{re.escape(f_first)}{mid}\b")
        for rx in (forward, reverse):
            for m in rx.finditer(folded):
                spans.append(_Span(m.start(), m.end(), "[CUSTOMER_NAME]", _P_CUSTOMER))
    if f_last and len(f_last) >= 2:
        for m in re.finditer(rf"\b{re.escape(f_last)}\b", folded):
            spans.append(_Span(m.start(), m.end(), "[SURNAME]", _P_FRAGMENT))
    for part in [f_first, *f_middles]:
        if len(part) >= 2:
            for m in re.finditer(rf"\b{re.escape(part)}\b", folded):
                spans.append(_Span(m.start(), m.end(), "[FIRST_NAME]", _P_FRAGMENT))

    for address in _addresses(identity):
        patterns = [_address_pattern(address)]
        street = address.split(",")[0]
        if street.strip() != address.strip():
            patterns.append(_address_pattern(street))
        for rx in patterns:
            if rx:
                for m in rx.finditer(folded):
                    spans.append(_Span(m.start(), m.end(), "[CUSTOMER_ADDRESS]", _P_CUSTOMER))

    for email in _emails(identity):
        for m in re.finditer(re.escape(email), folded):
            spans.append(_Span(m.start(), m.end(), "[CUSTOMER_EMAIL]", _P_CUSTOMER))

    return spans


def _generic_spans(original: str, folded: str, identity: IdentityContext) -> list[_Span]:
    spans: list[_Span] = []
    phones = _phone_digits(identity)
    dob = parse_dob(identity.date_of_birth)

    for m in EMAIL_RE.finditer(folded):
        spans.append(_Span(m.start(), m.end(), "[EMAIL]", _P_GENERIC))

    for m in PHONE_RE.finditer(folded):
        digits = re.sub(r"\D", "", m[0])[-10:]
        if digits in phones:
            spans.append(_Span(m.start(), m.end(), "[CUSTOMER_PHONE]", _P_CUSTOMER))
        else:
            spans.append(_Span(m.start(), m.end(), "[PHONE]", _P_GENERIC))

    for start, end, candidates in _dates_in(folded):
        if dob and _date_is_dob(candidates, dob):
            spans.append(_Span(start, end, "[CUSTOMER_DOB]", _P_CUSTOMER))
        else:
            spans.append(_Span(start, end, "[DATE]", _P_GENERIC))

    for rx, token in (
        (SSN_RE, "[SSN]"), (AGE_RE, "[AGE]"), (STREET_RE, "[ADDRESS]"),
        (PO_BOX_RE, "[ADDRESS]"), (ZIP_RE, "[ZIP]"), (LONG_NUMBER_RE, "[NUMBER]"),
    ):
        for m in rx.finditer(folded):
            spans.append(_Span(m.start(), m.end(), token, _P_GENERIC))

    for m in NAME_RE.finditer(original):
        spans.append(_Span(m.start(), m.end(), "[NAME]", _P_GENERIC))

    return spans


def _resolve(spans: list[_Span]) -> list[_Span]:
    """Greedy non-overlapping selection: priority first, then longer span, then earlier."""
    chosen: list[_Span] = []
    for span in sorted(spans, key=lambda s: (s.priority, -(s.end - s.start), s.start)):
        if span.end <= span.start:
            continue
        if all(span.end <= c.start or span.start >= c.end for c in chosen):
            chosen.append(span)
    return sorted(chosen, key=lambda s: s.start)


def redact_for_llm(
    text: str,
    identity: IdentityContext,
    person_detector=None,
    restrict: bool = True,
) -> Redacted:
    """
    Replace the customer's values, and other personal data, with placeholder tokens.
    The result is safe to show to an LLM; `evidence` says what was matched.

    Layers, in order:
      1. pattern rules (customer values, emails, phones, dates, addresses, names);
      2. `person_detector` (shared.pii_ner.PersonDetector), if given: NER PERSON spans
         become [NAME];
      3. when `restrict` is true (the default, and what production uses): every word
         that is not structural vocabulary is dropped (see restrict_vocabulary). This
         is the layer that guarantees third-party names do not leave.
    """
    folded = fold(text)
    candidates = _customer_spans(folded, identity) + _generic_spans(text, folded, identity)
    if person_detector is not None:
        candidates += [
            _Span(start, end, "[NAME]", _P_GENERIC)
            for start, end in person_detector.person_spans(text)
        ]
    accepted = _resolve(candidates)

    out: list[str] = []
    cursor = 0
    ev = Evidence()
    for span in accepted:
        out.append(text[cursor:span.start])
        out.append(span.token)
        cursor = span.end
        match span.token:
            case "[CUSTOMER_NAME]":
                ev.name_full = ev.surname = ev.first_name = True
            case "[SURNAME]":
                ev.surname = True
            case "[FIRST_NAME]":
                ev.first_name = True
            case "[CUSTOMER_ADDRESS]":
                ev.address = True
            case "[CUSTOMER_PHONE]":
                ev.phone = True
            case "[CUSTOMER_EMAIL]":
                ev.email = True
            case "[CUSTOMER_DOB]":
                ev.dob = True
            case "[NAME]":
                ev.other_people += 1
    out.append(text[cursor:])
    redacted_text = "".join(out)
    if restrict:
        redacted_text = restrict_vocabulary(redacted_text)
    return Redacted(redacted_text, ev)


# --------------------------------------------------------------------------
# Deny-by-default vocabulary filter
# --------------------------------------------------------------------------

# Placeholder tokens | words (any script, with inner apostrophes) | digit runs | space | other
_TOKEN_RE = re.compile(r"\[[A-Z_]+\]|[^\W\d_]+(?:['’][^\W\d_]+)*|\d+|\s+|.", re.S)
_BRACKET_RE = re.compile(r"\[[A-Z_]+\]")
_GAP = "…"


def _word_key(word: str) -> str:
    return fold(word).replace("’", "'")


def restrict_vocabulary(text: str) -> str:
    """
    Keep only structural vocabulary (shared.pii_vocab.ALLOWED_WORDS), our own
    placeholder tokens, punctuation, and short numbers. Every other word, in any
    script and any casing, is dropped, and a run of dropped words becomes one "…".
    Digit runs of three or more become [NUMBER].

    This is the backstop behind the third-party-name guarantee: a name that neither
    patterns nor NER recognized is still removed, because it is not on the list.
    """
    out: list[str] = []
    gap = False
    for m in _TOKEN_RE.finditer(text):
        tok = m.group(0)
        if _BRACKET_RE.fullmatch(tok):
            if tok[1:-1] in KNOWN_TOKENS:
                out.append(tok)
                gap = False
            elif not gap:  # a bracket token we did not emit (e.g. copied from the page)
                out.append(_GAP)
                gap = True
        elif tok[0].isalpha():
            if _word_key(tok) in ALLOWED_WORDS:
                out.append(tok)
                gap = False
            elif not gap:
                out.append(_GAP)
                gap = True
        elif tok.isdigit():
            out.append(tok if len(tok) <= 2 else "[NUMBER]")
            gap = False
        elif tok.isspace():
            out.append(tok)
        elif gap and tok in "-–—'’":
            continue  # hyphenated/possessive fragments of a dropped word
        else:
            out.append(tok)
            gap = False
    # Dropped words leave the spaces that surrounded them; collapse those runs (newlines are
    # kept, since they carry record structure).
    return re.sub(r"[ \t]{2,}", " ", "".join(out))


def find_disallowed(text: str) -> int:
    """Count of tokens in `text` that restrict_vocabulary would have removed."""
    bad = 0
    for m in _TOKEN_RE.finditer(text):
        tok = m.group(0)
        if _BRACKET_RE.fullmatch(tok):
            bad += tok[1:-1] not in KNOWN_TOKENS
        elif tok[0].isalpha():
            bad += _word_key(tok) not in ALLOWED_WORDS
        elif tok.isdigit():
            bad += len(tok) > 2
    return bad


def assert_vocabulary_only(excerpt: str) -> None:
    """
    Raise OutboundPiiError if the page excerpt contains anything outside the
    structural vocabulary. Run on the final excerpt, so a bug in the filter fails
    closed instead of leaking third-party text.
    """
    bad = find_disallowed(excerpt)
    if bad:
        raise OutboundPiiError(
            f"LLM call blocked: page excerpt contains {bad} token(s) outside the allowed vocabulary"
        )


# --------------------------------------------------------------------------
# Fail-closed check
# --------------------------------------------------------------------------

_OWN_TOKEN_RE = re.compile(r"\[[A-Z_]+\]")


def find_customer_values(text: str, identity: IdentityContext) -> set[str]:
    """Categories of customer data present in `text` (empty set means clean)."""
    probe = _OWN_TOKEN_RE.sub(" ", text)
    folded = fold(probe)
    norm = " ".join(folded.split())
    found: set[str] = set()

    first, last, middles = name_parts(identity.full_name)
    full = _norm(identity.full_name)
    if len(full) >= 4 and full in norm:
        found.add("name")
    for part in (first, last, *middles):
        fp = fold(part)
        if len(fp) >= 3 and re.search(rf"\b{re.escape(fp)}\b", folded):
            found.add("name")

    for address in _addresses(identity):
        na = _norm(address)
        street = _norm(address.split(",")[0])
        if (len(na) >= 5 and na in norm) or (len(street) >= 5 and street in norm):
            found.add("address")

    for email in _emails(identity):
        if email in folded:
            found.add("email")

    phones = _phone_digits(identity)
    if phones:
        for m in PHONE_RE.finditer(folded):
            if re.sub(r"\D", "", m[0])[-10:] in phones:
                found.add("phone")
        compact = re.sub(r"\D", "", folded)
        if any(p in compact for p in phones):
            found.add("phone")

    dob = parse_dob(identity.date_of_birth)
    if dob:
        for _, _, candidates in _dates_in(folded):
            if _date_is_dob(candidates, dob):
                found.add("date_of_birth")
    return found


def assert_clean(outbound: str, identity: IdentityContext) -> None:
    """
    Raise OutboundPiiError if `outbound` still contains a customer value. Call this
    on the complete payload (prompt + page excerpt) immediately before an LLM call.
    """
    found = find_customer_values(outbound, identity)
    if found:
        raise OutboundPiiError(
            f"LLM call blocked: payload still contains customer data ({', '.join(sorted(found))})"
        )


# --------------------------------------------------------------------------
# Local, masked summary for the dashboard (never sent to a model)
# --------------------------------------------------------------------------

def mask_name(name: str) -> str:
    return " ".join(p[0] + "***" for p in name.split() if p) or "***"


def mask_address(address: str) -> str:
    street = address.split(",")[0].split()
    masked = []
    for token in street:
        if token[0].isdigit():
            masked.append(token[0] + "*" * (len(token) - 1))
        else:
            masked.append(token[0] + "***")
    return " ".join(masked) or "***"


def build_pii_summary(identity: IdentityContext, ev: Evidence) -> str:
    """
    Masked description of what was found, built from local evidence. Replaces the
    summary the LLM used to write, since the LLM no longer sees any identity.
    """
    parts: list[str] = []
    if ev.name_full or ev.surname:
        parts.append(mask_name(identity.full_name))
    if ev.address and identity.current_address:
        parts.append(mask_address(identity.current_address))
    if ev.phone:
        parts.append("phone number")
    if ev.email:
        parts.append("email address")
    if ev.dob:
        parts.append("date of birth")
    return (", ".join(parts) or "personal information (masked)")[:500]
