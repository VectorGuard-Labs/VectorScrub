"""The dashboard's manual opt-out guide is generated from the registry the agent uses."""
from pathlib import Path

from discovery.agent import DISCOVERY_SOURCES
from removal.agent import OPT_OUT_REGISTRY
from removal.frontend_export import OUTPUT, brokers, render, scan_sources


def test_the_committed_frontend_file_matches_the_registry():
    assert OUTPUT.exists(), "run: python -m removal.frontend_export"
    assert OUTPUT.read_text(encoding="utf-8") == render(), (
        "frontend/src/lib/brokers.generated.ts is out of date; run `python -m removal.frontend_export` from agents/"
    )


def test_every_source_discovery_can_report_has_a_manual_opt_out_entry():
    exported = {b["name"] for b in brokers()}
    assert {s["name"] for s in DISCOVERY_SOURCES} <= exported


def test_only_https_links_and_no_personal_data_are_exported():
    for b in brokers():
        if b["optOutUrl"]:
            assert b["optOutUrl"].startswith("https://"), b["name"]
        assert b["method"] in ("web_form", "email")
        assert "{" not in (b["optOutUrl"] or ""), "a template placeholder would mean a customer value is meant to be filled in"


def test_the_unverified_california_drop_entry_is_not_offered_as_an_automated_removal():
    assert "ca-drop" not in OPT_OUT_REGISTRY
    assert all(p.procedure_type != "drop_api" for p in OPT_OUT_REGISTRY.values())


def test_scan_sources_match_what_discovery_actually_does():
    exported = {s["name"]: s for s in scan_sources()}
    assert {n for n, s in exported.items() if s["tier"] == "free"} == {
        "spokeo.com", "whitepages.com", "peoplefinder.com", "truepeoplesearch.com"}
    assert len(exported) == 34
    assert {n for n, s in exported.items() if s["manual"]} == {
        "peoplefinder.com", "intelius.com", "peoplelooker.com", "ownerly.com", "instantcheckmate.com",
        "zoominfo.com", "contactout.com", "rocketreach.co"}


def test_the_peopleconnect_brands_share_one_suppression_center_and_no_known_dead_link_returns():
    by_name = {b["name"]: b for b in brokers()}
    shared = "https://suppression.peopleconnect.us/login"
    for n in ("intelius.com", "truthfinder.com", "instantcheckmate.com", "ussearch.com"):
        assert by_name[n]["optOutUrl"] == shared, n
    dead = {  # returned 404 when checked on 2026-11-11
        "https://www.peoplefinder.com/optout.php", "https://www.ownerly.com/opt-out",
        "https://www.advancedbackgroundchecks.com/removal",
        "https://www.peoplelooker.com/f/optout/search",  # moved; found by the nightly check, 2026-11-11
    }
    assert not dead & {b["optOutUrl"] for b in brokers()}
