"""
The California Data Broker Registry importer.

The CSV is outside data, so these tests pin what survives the import (company, sites, rights
pages, self-reported deletion figures) and what must never (emails, phones, addresses, unsafe
links), plus that a differently-shaped export fails loudly instead of importing garbage.
"""
import csv
import io
import json
import re
from pathlib import Path

import pytest

from ca_registry.importer import (
    OUTPUT, RegistryFormatError, parse_count, parse_days, parse_hosts, parse_registry,
    parse_rights_links, render,
)

# ---- cells -----------------------------------------------------------------------------------

@pytest.mark.parametrize("cell, expected", [
    ("spokeo.com; www.thatsthem.com; www.anywho.com", ["spokeo.com", "thatsthem.com", "anywho.com"]),
    ("https://www.example.com/path?q=1", ["example.com"]),
    ("HTTP://EXAMPLE.COM, example.com", ["example.com"]),             # case folded, duplicates removed
    ("beenverified.com; https", ["beenverified.com"]),               # a stray scheme word is dropped
    ("example.com:8080/x user@host.test", ["example.com", "host.test"]),
    ("not a host; localhost; 12345; ", []),
    ("", []),
    ("a_b.com javascript:alert(1)", []),                              # underscores and script schemes are not hostnames
])
def test_site_cells_yield_only_hostnames(cell, expected):
    assert parse_hosts(cell) == expected


def test_a_cell_cannot_produce_an_unbounded_site_list():
    assert len(parse_hosts(";".join(f"s{i}.com" for i in range(500)))) == 60


@pytest.mark.parametrize("cell, expected", [
    ("https://example.com/privacy", ["https://example.com/privacy"]),
    ("example.com/privacy", ["https://example.com/privacy"]),         # no scheme: assume https
    ("www.example.com", ["https://www.example.com"]),
    ("See https://a.com/optout or https://b.com/dsar.", ["https://a.com/optout", "https://b.com/dsar"]),
    ("https://a.com/x https://a.com/x", ["https://a.com/x"]),         # duplicates removed
    ("https://a.com/Opt-Out?ID=7", ["https://a.com/Opt-Out?ID=7"]),  # path and query case is kept
    ("https://A.COM/x", ["https://a.com/x"]),                         # only the host is lowercased
    ("http://insecure.example/optout", ["http://insecure.example/optout"]),
    ("https://a.com/1 https://b.com/2 https://c.com/3 https://d.com/4", ["https://a.com/1", "https://b.com/2", "https://c.com/3"]),
])
def test_rights_cells_yield_clean_links(cell, expected):
    assert parse_rights_links(cell) == expected


@pytest.mark.parametrize("cell", [
    "javascript:alert(1)", "JaVaScRiPt:alert(1)", "data:text/html,<script>alert(1)</script>",
    "mailto:privacy@example.com", "ftp://example.com/x", "file:///etc/passwd",
    "https://user:pass@example.com/x",                  # credentials in a link
    "https://example.com:8443/x",                       # unusual port
    "https://localhost/x", "https://10.0.0.1/x", "https:///nohost", "click here", "N/A", "",
    "https://" + "a" * 600 + ".com",                    # absurdly long
])
def test_unsafe_or_meaningless_links_never_survive(cell):
    assert parse_rights_links(cell) == []


def test_numbers():
    assert [parse_count(x) for x in ("1,234", "5", "5.0", "", "x", "-3", "1e5", None, "99999999999999")] == [1234, 5, 5, None, None, None, None, None, None]
    assert [parse_days(x) for x in ("5.5", "12", "", "abc", "-1", "99999", None)] == [5.5, 12.0, None, None, None, None, None]


# ---- whole files -----------------------------------------------------------------------------

HEADERS = [
    "Broker ID", "Data broker name:", "Doing Business As (DBA), if applicable:", "Data broker primary website:",
    "Data broker primary contact email address:", "Data broker primary phone number: [optional]",
    "Data broker primary street address:", "Data broker city:", "Data broker state:", "Data broker zip code:",
    "Data broker country:",
    "Data broker's primary website that contains details on how consumers can exercise their CA Consumer Privacy rights:",
    "The data broker or any of its subsidiaries is regulated by the federal Fair Credit Reporting Act",
    "Requests to delete - Total requests received",
    "Requests to delete - Total requests received - Complied in whole",
    "Requests to delete - Total requests received - Complied in part",
    "Requests to delete - Total requests received - Denied",
    "Requests to delete - The number of days to respond substantively to a request to delete in 2024 - Mean",
    "Requests to delete - The number of days to respond substantively to a request to delete in 2024 - Median",
]


def make_row(**kw):
    row = {
        "Broker ID": "1", "Data broker name:": "Acme Data, LLC", "Doing Business As (DBA), if applicable:": "",
        "Data broker primary website:": "acme.com", "Data broker primary contact email address:": "jane.doe@acme.com",
        "Data broker primary phone number: [optional]": "555-0100", "Data broker primary street address:": "1 Secret St",
        "Data broker city:": "Springfield", "Data broker state:": "CA", "Data broker zip code:": "90000",
        "Data broker country:": "UNITED STATES",
        "Data broker's primary website that contains details on how consumers can exercise their CA Consumer Privacy rights:": "acme.com/privacy",
        "The data broker or any of its subsidiaries is regulated by the federal Fair Credit Reporting Act": "No",
        "Requests to delete - Total requests received": "100",
        "Requests to delete - Total requests received - Complied in whole": "90",
        "Requests to delete - Total requests received - Complied in part": "4",
        "Requests to delete - Total requests received - Denied": "6",
        "Requests to delete - The number of days to respond substantively to a request to delete in 2024 - Mean": "8",
        "Requests to delete - The number of days to respond substantively to a request to delete in 2024 - Median": "7.5",
    }
    row.update(kw)
    return row


def to_csv(rows, headers=HEADERS):
    buf = io.StringIO()
    w = csv.DictWriter(buf, fieldnames=headers, extrasaction="ignore")
    w.writeheader()
    w.writerows(rows)
    return buf.getvalue()


def test_a_row_becomes_a_company_with_its_sites_links_and_figures():
    [b] = parse_registry(to_csv([make_row(**{"Data broker primary website:": "acme.com; www.acme-people.com"})]))
    assert (b.id, b.name, b.dba, b.sites, b.rights, b.fcra, b.us) == (
        "1", "Acme Data, LLC", None, ["acme.com", "acme-people.com"], ["https://acme.com/privacy"], False, True)
    assert (b.deletion.received, b.deletion.whole, b.deletion.part, b.deletion.denied, b.deletion.medianDays) == (100, 90, 4, 6, 7.5)


def test_no_contact_detail_survives_into_the_output():
    brokers = parse_registry(to_csv([make_row()]))
    text = render(brokers, "2026-11-11")
    for secret in ("jane.doe", "acme.com>", "555-0100", "1 Secret St", "Springfield", "90000"):
        assert secret not in text, secret
    assert "@" not in text
    assert set(json.loads(json.dumps(brokers[0], default=lambda o: o.__dict__))) == {
        "id", "name", "dba", "sites", "rights", "fcra", "us", "deletion"}, "exactly these fields, nothing else"


def test_multiline_cells_reordered_columns_and_curly_quotes_are_handled():
    headers = list(reversed(HEADERS))                                   # a different column order
    curly = [h.replace("'", "\u2019") for h in headers]                 # typographic apostrophes
    rows = [make_row(**{"Data broker name:": "Multi\nLine  Name,\tInc.",
                        "Data broker primary website:": "a.com\nb.com"})]
    buf = io.StringIO()
    w = csv.writer(buf)
    w.writerow(curly)
    w.writerow([rows[0][h] for h in headers])
    [b] = parse_registry(buf.getvalue())
    assert b.name == "Multi Line Name, Inc." and b.sites == ["a.com", "b.com"]
    assert b.rights == ["https://acme.com/privacy"]


def test_rows_with_nothing_to_show_and_duplicate_ids_are_skipped_and_the_result_is_sorted():
    out = parse_registry(to_csv([
        make_row(**{"Broker ID": "3", "Data broker name:": "zeta inc", "Data broker primary website:": "zeta.com"}),
        make_row(**{"Broker ID": "4", "Data broker name:": "", "Data broker primary website:": "x.com"}),           # no name
        make_row(**{"Broker ID": "5", "Data broker name:": "NoSite", "Data broker primary website:": "n/a"}),     # no site
        make_row(**{"Broker ID": "3", "Data broker name:": "dupe", "Data broker primary website:": "dupe.com"}),  # repeated id
        make_row(**{"Broker ID": "6", "Data broker name:": "Alpha Co", "Data broker primary website:": "alpha.com"}),
    ]))
    assert [b.name for b in out] == ["Alpha Co", "zeta inc"]


def test_missing_figures_become_none_and_not_zero():
    [b] = parse_registry(to_csv([make_row(**{"Requests to delete - Total requests received": ""})]))
    assert b.deletion is None, "unreported is not the same as zero requests"
    [b] = parse_registry(to_csv([make_row(**{"Requests to delete - The number of days to respond substantively to a request to delete in 2024 - Median": "n/a"})]))
    assert b.deletion.medianDays is None


def test_foreign_and_fcra_flags():
    [b] = parse_registry(to_csv([make_row(**{"Data broker country:": "UNITED KINGDOM",
        "The data broker or any of its subsidiaries is regulated by the federal Fair Credit Reporting Act": "Yes"})]))
    assert (b.us, b.fcra) == (False, True)


def test_a_file_that_is_not_the_registry_fails_loudly_naming_the_column():
    with pytest.raises(RegistryFormatError, match="Broker ID"):
        parse_registry("name,site\nx,y\n")
    no_delete = [h for h in HEADERS if "denied" not in h.lower()]
    with pytest.raises(RegistryFormatError, match="denied"):
        parse_registry(to_csv([make_row()], headers=no_delete))
    with pytest.raises(RegistryFormatError, match="empty"):
        parse_registry("")


# ---- the committed output --------------------------------------------------------------------

def committed():
    text = OUTPUT.read_text(encoding="utf-8")
    body = text.split("export const CA_BROKERS: CaBroker[] = [", 1)[1].rsplit("];", 1)[0]
    return text, [json.loads(line.strip().rstrip(",")) for line in body.splitlines() if line.strip()]


def test_the_committed_directory_is_clean_company_data():
    text, brokers = committed()
    assert len(brokers) > 500, "the real registry was imported"
    assert "@" not in text.split("export const CA_BROKERS_SOURCE")[1], "no email address anywhere in the data"
    assert not re.search(r"mailto:|javascript:|data:text", text, re.I)
    ids = [b["id"] for b in brokers]
    assert len(ids) == len(set(ids))
    names = [b["name"].lower() for b in brokers]
    assert names == sorted(names), "sorted by name"
    for b in brokers:
        assert set(b) == {"id", "name", "dba", "sites", "rights", "fcra", "us", "deletion"}
        assert b["name"] and b["sites"], b["id"]
        # re-running the cell parsers over what was written changes nothing
        assert parse_hosts(" ".join(b["sites"])) == b["sites"], b["id"]
        assert parse_rights_links(" ".join(b["rights"])) == b["rights"], b["id"]
        assert all(u.startswith(("https://", "http://")) for u in b["rights"])
        d = b["deletion"]
        if d:
            # (the companies' own figures: their parts need not add up to the total, so that is not checked)
            assert all(isinstance(d[k], int) and d[k] >= 0 for k in ("received", "whole", "part", "denied"))


def test_the_source_line_matches_the_data():
    text, brokers = committed()
    meta = json.loads(re.search(r"CA_BROKERS_SOURCE = (\{.*?\}) as const", text).group(1))
    assert meta["companies"] == len(brokers)
    assert meta["sites"] == len({s for b in brokers for s in b["sites"]})
    assert "California" in meta["label"] and re.fullmatch(r"\d{4}-\d{2}-\d{2}", meta["importedOn"])


def test_the_sites_we_list_are_found_under_their_parent_companies():
    _, brokers = committed()
    owner = {s: b["name"] for b in brokers for s in b["sites"]}
    assert owner["thatsthem.com"] == owner["spokeo.com"], "Spokeo, Inc. runs ThatsThem"
    assert owner["peoplefinder.com"] == owner["ussearch.com"] == owner["intelius.com"]
    assert owner["advancedbackgroundchecks.com"] == owner["smartbackgroundchecks.com"] == owner["fastpeoplesearch.com"]
    assert owner["peoplelooker.com"] == owner["ownerly.com"] == owner["beenverified.com"]
    assert "radaris.com" not in owner, "Radaris is not in the California registry"
