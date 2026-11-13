"""
Imports the California Data Broker Registry (the CSV published by the California Privacy
Protection Agency / Attorney General) into a directory the dashboard's opt-out guide can search.

What it keeps, and what it deliberately does not:
  * KEPT: the company name, the sites it runs, the page(s) where it says consumers can exercise
    their privacy rights, and its self-reported 2024 deletion-request statistics.
  * NEVER KEPT: contact email addresses, phone numbers, street addresses, or anything that names
    a person. The registry's contact fields exist for the regulator. Several are individuals'
    addresses, and nothing here needs them. Dropping them is what keeps the output public data
    about companies, safe to commit and to ship to browsers.

The CSV is data from outside, so every cell is treated as untrusted: hosts must look like
hostnames, links must be http(s) with a hostname (never javascript:, mailto:, data: ...), numbers
must parse, and lengths are capped. Columns are found by their header text, not their position,
so a reordered export still imports and a missing column fails loudly by name.

Usage, from agents/:
    python -m ca_registry.importer "/path/to/California Data Broker Registry 2026.csv"
        Imports the file and prints what changed since the data now in the repo.
    python -m ca_registry.importer --check "/path/to/new file.csv"
        Writes nothing. Prints the same summary and exits 1 if the file differs from what is in
        the repo, 0 if it is identical. Use it to find out whether a re-import is needed.

Writes frontend/src/lib/ca-brokers.generated.ts (the guide's directory) and
agents/ca_registry/rights_pages.json (the pages the nightly check visits).

WHEN TO RE-IMPORT. The registry is re-filed every year (brokers must register each January) and
companies can update their entries in between. Download the current CSV from the California
Attorney General's / CPPA data broker registry page, run `--check` to see what changed, then
import and commit both generated files. The guide shows the import date, so stale data is visible.
"""
from __future__ import annotations

import csv
import json
import re
import sys
from dataclasses import asdict, dataclass, field
from datetime import date
from pathlib import Path
from urllib.parse import urlsplit

OUTPUT = Path(__file__).resolve().parents[2] / "frontend" / "src" / "lib" / "ca-brokers.generated.ts"
PAGES_OUTPUT = Path(__file__).resolve().parent / "rights_pages.json"

SOURCE_LABEL = "California Data Broker Registry 2026 (California Privacy Protection Agency)"

MAX_NAME = 200
MAX_URL = 500
MAX_RIGHTS_LINKS = 3
MAX_SITES = 60
MAX_COUNT = 10**10

_HOSTNAME = re.compile(r"^(?:[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.)+[a-z]{2,63}$")
_TOKEN_SPLIT = re.compile(r"[;,\s<>\"'()\[\]]+")


class RegistryFormatError(Exception):
    """The CSV is not shaped like the registry. The message names what is missing."""


@dataclass
class Deletion:
    """A company's own report of the deletion requests it received in 2024."""
    received: int
    whole: int
    part: int
    denied: int
    medianDays: float | None


@dataclass
class CaBroker:
    id: str
    name: str
    dba: str | None
    sites: list[str]
    rights: list[str]
    fcra: bool
    us: bool
    deletion: Deletion | None = field(default=None)


# ---------------------------------------------------------------------------
# Cell parsing
# ---------------------------------------------------------------------------

def clean_text(value: str | None, limit: int = MAX_NAME) -> str:
    """One line of plain text: control characters and runs of whitespace collapsed, capped."""
    text = re.sub(r"[\x00-\x1f\x7f]+", " ", value or "")
    return " ".join(text.split())[:limit]


def parse_count(value: str | None) -> int | None:
    """A non-negative whole number ('1,234' ok), or None for blank, negative or nonsense."""
    text = (value or "").strip().replace(",", "")
    if not re.fullmatch(r"\d+(\.0+)?", text):
        return None
    n = int(float(text))
    return n if 0 <= n <= MAX_COUNT else None


def parse_days(value: str | None) -> float | None:
    text = (value or "").strip().replace(",", "")
    if not re.fullmatch(r"\d+(\.\d+)?", text):
        return None
    n = float(text)
    return round(n, 1) if 0 <= n <= 3650 else None


def parse_yes(value: str | None) -> bool:
    return (value or "").strip().lower() in {"yes", "y", "true"}


def parse_hosts(cell: str | None) -> list[str]:
    """
    The site hostnames in a 'website' cell. The registry lists every site a company runs in one
    cell, separated by ';', ',' or spaces, with or without a scheme, 'www.' or a path.
    Anything that is not a hostname is dropped.
    """
    hosts: list[str] = []
    for token in _TOKEN_SPLIT.split((cell or "").strip().lower()):
        host = re.sub(r"^[a-z][a-z0-9+.-]*://", "", token).split("/")[0].split("?")[0].split("#")[0]
        host = host.split("@")[-1].split(":")[0].removeprefix("www.").strip(".")
        if _HOSTNAME.match(host) and host not in hosts:
            hosts.append(host)
    return hosts[:MAX_SITES]


def parse_rights_links(cell: str | None) -> list[str]:
    """
    The pages in a 'how to exercise your privacy rights' cell. Cells hold one or several links,
    sometimes with prose around them and often without a scheme ('example.com/privacy').
    Only http(s) links to a real hostname survive; at most MAX_RIGHTS_LINKS are kept.
    """
    links: list[str] = []
    for token in _TOKEN_SPLIT.split((cell or "").strip()):
        token = token.rstrip(".:!?")
        if not token or len(token) > MAX_URL:
            continue
        if re.match(r"^[a-z][a-z0-9+.-]*:", token, re.I) and not re.match(r"^https?://", token, re.I):
            continue  # javascript:, mailto:, data:, ftp: ... are never links we show
        url = token if re.match(r"^https?://", token, re.I) else "https://" + token
        try:
            parts = urlsplit(url)
            port = parts.port
        except ValueError:
            continue
        host = (parts.hostname or "").lower()
        if parts.username is not None or parts.password is not None or port not in (None, 80, 443):
            continue
        if not _HOSTNAME.match(host):
            continue
        url = parts._replace(netloc=host if port is None else f"{host}:{port}").geturl()
        if url not in links:
            links.append(url)
    return links[:MAX_RIGHTS_LINKS]


# ---------------------------------------------------------------------------
# Finding the columns
# ---------------------------------------------------------------------------

def _norm(header: str) -> str:
    return (
        header.strip().lower().replace("\u2019", "'").replace("\u2011", "-").replace("\u2010", "-")
    )


def _find(headers: list[str], *needles: str, required: bool = True) -> str | None:
    """The first header that starts with the needle (case, curly quotes and dashes ignored)."""
    for needle in needles:
        n = _norm(needle)
        for h in headers:
            if _norm(h).startswith(n):
                return h
    if required:
        raise RegistryFormatError(f"column not found: {needles[0]!r}")
    return None


def _find_delete(headers: list[str], contains: str) -> str | None:
    for h in headers:
        n = _norm(h)
        if n.startswith("requests to delete") and contains in n:
            return h
    return None


def parse_registry(csv_text: str) -> list[CaBroker]:
    """Parse the registry CSV into brokers, sorted by name. Raises RegistryFormatError if it is
    not the registry."""
    reader = csv.DictReader(csv_text.splitlines(keepends=True))
    headers = list(reader.fieldnames or [])
    if not headers:
        raise RegistryFormatError("the file is empty")

    c_id = _find(headers, "Broker ID")
    c_name = _find(headers, "Data broker name")
    c_dba = _find(headers, "Doing Business As", required=False)
    c_web = _find(headers, "Data broker primary website")
    c_rights = _find(headers, "Data broker's primary website that contains")
    c_country = _find(headers, "Data broker country", required=False)
    c_fcra = _find(headers, "The data broker or any of its subsidiaries is regulated by the federal Fair Credit")
    c_recv = _find_delete(headers, "total requests received")
    c_whole = _find_delete(headers, "complied in whole")
    c_part = _find_delete(headers, "complied in part")
    c_denied = _find_delete(headers, "denied")
    c_median = _find_delete(headers, "median")
    for label, col in (("deletion requests received", c_recv), ("complied in whole", c_whole),
                       ("complied in part", c_part), ("denied", c_denied), ("median days", c_median)):
        if col is None:
            raise RegistryFormatError(f"column not found: 'Requests to delete - {label}'")

    brokers: list[CaBroker] = []
    seen: set[str] = set()
    for row in reader:
        name = clean_text(row.get(c_name))
        sites = parse_hosts(row.get(c_web))
        if not name or not sites:
            continue  # nothing to show or search by
        broker_id = clean_text(row.get(c_id), 40) or name
        if broker_id in seen:
            continue
        seen.add(broker_id)

        received = parse_count(row.get(c_recv))
        deletion = None
        if received is not None:
            deletion = Deletion(
                received=received,
                whole=parse_count(row.get(c_whole)) or 0,
                part=parse_count(row.get(c_part)) or 0,
                denied=parse_count(row.get(c_denied)) or 0,
                medianDays=parse_days(row.get(c_median)),
            )
        country = clean_text(row.get(c_country), 60).lower() if c_country else ""
        brokers.append(
            CaBroker(
                id=broker_id,
                name=name,
                dba=clean_text(row.get(c_dba)) or None if c_dba else None,
                sites=sites,
                rights=parse_rights_links(row.get(c_rights)),
                fcra=parse_yes(row.get(c_fcra)),
                us=country in ("united states", "usa", "us", ""),
                deletion=deletion,
            )
        )
    brokers.sort(key=lambda b: b.name.lower())
    return brokers


# ---------------------------------------------------------------------------
# Output
# ---------------------------------------------------------------------------

def render(brokers: list[CaBroker], imported_on: str) -> str:
    """The TypeScript module the frontend imports. Compact: one company per line."""
    lines = ",\n".join("  " + json.dumps(asdict(b), ensure_ascii=False, separators=(",", ":")) for b in brokers)
    source = json.dumps(
        {"label": SOURCE_LABEL, "importedOn": imported_on, "companies": len(brokers),
         "sites": len({s for b in brokers for s in b.sites})},
        ensure_ascii=False,
    )
    return (
        "// GENERATED by agents/ca_registry/importer.py from the California Data Broker Registry CSV.\n"
        "// Do not edit by hand. Public information about companies only: no contact emails, phone\n"
        "// numbers or street addresses are kept. Deletion figures are the companies' own reports.\n\n"
        "export interface CaBrokerDeletion {\n  received: number;\n  whole: number;\n  part: number;\n"
        "  denied: number;\n  medianDays: number | null;\n}\n\n"
        "export interface CaBroker {\n  id: string;\n  name: string;\n  dba: string | null;\n  sites: string[];\n"
        "  rights: string[];\n  fcra: boolean;\n  us: boolean;\n  deletion: CaBrokerDeletion | null;\n}\n\n"
        f"export const CA_BROKERS_SOURCE = {source} as const;\n\n"
        f"export const CA_BROKERS: CaBroker[] = [\n{lines},\n];\n"
    )


def rights_pages(brokers: list[CaBroker]) -> list[dict[str, str]]:
    """The one page per company the nightly check visits: its first privacy-request link,
    keyed by registry id. Public company pages only."""
    return [{"id": b.id, "url": b.rights[0]} for b in brokers if b.rights]


def render_pages(brokers: list[CaBroker]) -> str:
    lines = ",\n".join("  " + json.dumps(p, separators=(",", ":")) for p in rights_pages(brokers))
    return f"[\n{lines}\n]\n"


# ---------------------------------------------------------------------------
# Comparing a new file with what is committed
# ---------------------------------------------------------------------------

def read_committed(path: Path | None = None) -> tuple[list[dict], str | None]:
    """The brokers (as dicts) and the import date in the committed TypeScript file, or ([], None)
    if there is none yet."""
    path = OUTPUT if path is None else path
    if not path.exists():
        return [], None
    text = path.read_text(encoding="utf-8")
    try:
        body = text.split("export const CA_BROKERS: CaBroker[] = [", 1)[1].rsplit("];", 1)[0]
        brokers = [json.loads(line.strip().rstrip(",")) for line in body.splitlines() if line.strip()]
        meta = re.search(r'"importedOn":"?(\d{4}-\d{2}-\d{2})', text.replace(" ", ""))
    except (IndexError, ValueError):
        return [], None
    return brokers, meta.group(1) if meta else None


@dataclass
class RegistryDiff:
    added: list[str]
    removed: list[str]
    sites_changed: list[str]
    rights_changed: list[str]
    deletion_changed: int
    unchanged: int

    @property
    def identical(self) -> bool:
        return not (self.added or self.removed or self.sites_changed or self.rights_changed or self.deletion_changed)


def diff_registries(old: list[dict], new: list[CaBroker]) -> RegistryDiff:
    """What differs, by registry id. Company names are used only to describe the change."""
    old_by = {b["id"]: b for b in old}
    new_by = {b.id: asdict(b) for b in new}
    added = sorted(new_by[i]["name"] for i in new_by.keys() - old_by.keys())
    removed = sorted(old_by[i]["name"] for i in old_by.keys() - new_by.keys())
    sites = rights = deletion = same = 0
    sites_names: list[str] = []
    rights_names: list[str] = []
    for i in new_by.keys() & old_by.keys():
        o, n = old_by[i], new_by[i]
        changed = False
        if o["sites"] != n["sites"]:
            sites_names.append(n["name"])
            changed = True
        if o["rights"] != n["rights"]:
            rights_names.append(n["name"])
            changed = True
        if o["deletion"] != n["deletion"]:
            deletion += 1
            changed = True
        if o["name"] != n["name"] or o["dba"] != n["dba"] or o["fcra"] != n["fcra"] or o["us"] != n["us"]:
            changed = True
        same += 0 if changed else 1
    return RegistryDiff(added, removed, sorted(sites_names), sorted(rights_names), deletion, same)


def format_diff(d: RegistryDiff, committed_on: str | None, today: date | None = None) -> str:
    today = today or date.today()
    lines = []
    if committed_on:
        try:
            age = (today - date.fromisoformat(committed_on)).days
            lines.append(f"data in the repo was imported {committed_on} ({age} day{'' if age == 1 else 's'} ago)")
        except ValueError:
            lines.append(f"data in the repo was imported {committed_on}")
    else:
        lines.append("no data in the repo yet")
    if d.identical:
        lines.append(f"no changes: all {d.unchanged} companies are identical")
        return "\n".join(lines)
    lines.append(f"{d.unchanged} companies unchanged")
    def part(label: str, names: list[str]) -> None:
        if names:
            shown = ", ".join(names[:8]) + (f" and {len(names) - 8} more" if len(names) > 8 else "")
            lines.append(f"{len(names)} {label}: {shown}")
    part("added", d.added)
    part("removed", d.removed)
    part("with different sites", d.sites_changed)
    part("with different privacy-request pages", d.rights_changed)
    if d.deletion_changed:
        lines.append(f"{d.deletion_changed} with updated deletion figures")
    return "\n".join(lines)


def import_file(
    csv_path: Path,
    out_path: Path | None = None,
    imported_on: str | None = None,
    pages_path: Path | None = None,
) -> list[CaBroker]:
    # Resolved here, not as default arguments: defaults are fixed when the module loads, which
    # would send a caller that changed OUTPUT (a test, a different checkout) to the old place.
    out_path = OUTPUT if out_path is None else out_path
    pages_path = PAGES_OUTPUT if pages_path is None else pages_path
    brokers = parse_registry(csv_path.read_bytes().decode("utf-8-sig", errors="replace"))
    out_path.write_text(render(brokers, imported_on or date.today().isoformat()), encoding="utf-8")
    pages_path.write_text(render_pages(brokers), encoding="utf-8")
    return brokers


def main(argv: list[str]) -> int:
    args = argv[1:]
    check = "--check" in args
    paths = [a for a in args if a != "--check"]
    if len(paths) != 1:
        print(__doc__)
        return 2
    try:
        new = parse_registry(Path(paths[0]).read_bytes().decode("utf-8-sig", errors="replace"))
    except (OSError, RegistryFormatError) as exc:
        print(f"import failed: {exc}")
        return 1
    old, committed_on = read_committed()
    diff = diff_registries(old, new)
    print(format_diff(diff, committed_on))
    if check:
        return 0 if diff.identical else 1
    import_file(Path(paths[0]))
    sites = {s for b in new for s in b.sites}
    print(f"wrote {OUTPUT.name} and {PAGES_OUTPUT.name}: {len(new)} companies, {len(sites)} sites, "
          f"{len(rights_pages(new))} privacy-request pages")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
