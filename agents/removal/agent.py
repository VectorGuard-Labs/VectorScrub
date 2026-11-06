"""
Removal Agent — Phase 2
Identifies the correct opt-out procedure for each confirmed exposure
and prepares authorized removal requests.

Security principle: Removal requests are NEVER submitted without
explicit customer authorization recorded in the database.
The agent prepares and queues; a human-authorized trigger submits.
"""
from __future__ import annotations

import structlog

from shared.models import DetectionResult, RemovalProcedure

log = structlog.get_logger(__name__)

# Opt-out procedure registry.
# In production this grows to 600+ entries covering the DROP ecosystem
# and individual site procedures. Stored in DB for hot-reloading.
OPT_OUT_REGISTRY: dict[str, RemovalProcedure] = {
    "spokeo.com": RemovalProcedure(
        source_name="spokeo.com",
        procedure_type="web_form",
        opt_out_url="https://www.spokeo.com/optout",
        instructions="Submit name and URL via the opt-out form. Verification email sent.",
        requires_id_verification=False,
        estimated_days=7,
    ),
    "whitepages.com": RemovalProcedure(
        source_name="whitepages.com",
        procedure_type="web_form",
        opt_out_url="https://www.whitepages.com/suppression-requests",
        instructions="Enter listing URL and submit suppression request.",
        requires_id_verification=False,
        estimated_days=3,
    ),
    "peoplefinder.com": RemovalProcedure(
        source_name="peoplefinder.com",
        procedure_type="web_form",
        opt_out_url="https://www.peoplefinder.com/optout.php",
        instructions="Search for your record, select it, and submit opt-out.",
        requires_id_verification=False,
        estimated_days=14,
    ),
    "beenverified.com": RemovalProcedure(
        source_name="beenverified.com",
        procedure_type="web_form",
        opt_out_url="https://www.beenverified.com/app/optout/search",
        instructions="Enter personal details, find your record, and submit opt-out.",
        requires_id_verification=False,
        estimated_days=7,
    ),
    "intelius.com": RemovalProcedure(
        source_name="intelius.com",
        procedure_type="web_form",
        opt_out_url="https://www.intelius.com/opt-out",
        instructions="Submit opt-out form. Response within 72 hours.",
        requires_id_verification=False,
        estimated_days=7,
    ),
    "mylife.com": RemovalProcedure(
        source_name="mylife.com",
        procedure_type="email",
        opt_out_email="privacy@mylife.com",
        instructions="Email with subject 'Opt-Out Request' including full name and profile URL.",
        requires_id_verification=True,
        estimated_days=30,
    ),
    # California DROP platform (batch submission for 600+ brokers)
    "ca-drop": RemovalProcedure(
        source_name="California DROP Platform",
        procedure_type="drop_api",
        opt_out_url="https://privacy.ca.gov/data-deletion/",
        instructions="Submit via California's official DELETE Request and Opt-Out Platform. "
                     "Eligible California residents only. Covers 600+ registered data brokers.",
        requires_id_verification=True,
        estimated_days=45,
    ),
}


class RemovalAgent:
    """
    Looks up opt-out procedures for confirmed exposures and prepares
    removal request records. Does NOT submit requests — that requires
    explicit customer authorization from the frontend.
    """

    def get_procedure(self, source_name: str) -> RemovalProcedure | None:
        """Look up the opt-out procedure for a data source."""
        # Normalize: strip www., lowercase
        normalized = source_name.lower().removeprefix("www.")
        return OPT_OUT_REGISTRY.get(normalized)

    def prepare_removal_requests(
        self, exposures: list[DetectionResult]
    ) -> list[tuple[DetectionResult, RemovalProcedure | None]]:
        """
        Pair each exposure with its removal procedure (if known).
        Returns list of (exposure, procedure_or_None) tuples.
        """
        pairs = []
        for exposure in exposures:
            procedure = self.get_procedure(exposure.source_name)
            if not procedure:
                log.warning(
                    "no_procedure_found",
                    source=exposure.source_name,
                    note="Manual review required",
                )
            pairs.append((exposure, procedure))
        return pairs

    def sources_without_procedures(self, exposures: list[DetectionResult]) -> list[str]:
        """Return source names that have no registered opt-out procedure."""
        return [
            e.source_name
            for e in exposures
            if not self.get_procedure(e.source_name)
        ]
