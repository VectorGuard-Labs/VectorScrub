"""The dashboard's manual opt-out guide is generated from the registry the agent uses."""
from pathlib import Path

from discovery.agent import DISCOVERY_SOURCES
from removal.agent import OPT_OUT_REGISTRY
from removal.frontend_export import OUTPUT, brokers, render


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
