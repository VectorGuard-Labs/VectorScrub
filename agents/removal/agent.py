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
# Brokers with a verified opt-out procedure. California's DROP is deliberately NOT here: it
# is a free service residents use themselves (the dashboard links to it), and no consumer-side
# submission API has been verified.
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
    "truthfinder.com": RemovalProcedure(
        source_name="truthfinder.com",
        procedure_type="web_form",
        opt_out_url="https://www.truthfinder.com/opt-out/",
        instructions="Search for your record, submit opt-out request with record URL. "
                     "Identity verification required (government ID). Processing takes 5-7 business days.",
        requires_id_verification=True,
        estimated_days=7,
    ),
    "instantcheckmate.com": RemovalProcedure(
        source_name="instantcheckmate.com",
        procedure_type="web_form",
        opt_out_url="https://www.instantcheckmate.com/opt-out/",
        instructions="Submit opt-out form with full name and state. "
                     "Verification email sent. Removal typically completes within 48 hours.",
        requires_id_verification=False,
        estimated_days=2,
    ),
    "radaris.com": RemovalProcedure(
        source_name="radaris.com",
        procedure_type="web_form",
        opt_out_url="https://radaris.com/page/how-to-remove",
        instructions="Search for your listing, copy the URL, submit opt-out form. "
                     "Verify via email. Removal completes within 24-72 hours.",
        requires_id_verification=False,
        estimated_days=3,
    ),
    "truepeoplesearch.com": RemovalProcedure(
        source_name="truepeoplesearch.com",
        procedure_type="web_form",
        opt_out_url="https://www.truepeoplesearch.com/removal",
        instructions="Search for your record, click 'Remove', enter email for verification link. "
                     "Complete removal via verification email. Removal typically within 72 hours.",
        requires_id_verification=False,
        estimated_days=3,
    ),
    "fastpeoplesearch.com": RemovalProcedure(
        source_name="fastpeoplesearch.com",
        procedure_type="web_form",
        opt_out_url="https://www.fastpeoplesearch.com/removal",
        instructions="Search for your listing, select it, submit removal request. "
                     "Verify via email. Removal completes within 24 hours.",
        requires_id_verification=False,
        estimated_days=1,
    ),
    "familytreenow.com": RemovalProcedure(
        source_name="familytreenow.com",
        procedure_type="web_form",
        opt_out_url="https://www.familytreenow.com/optout",
        instructions="Search for your record, copy the URL, submit opt-out form with email. "
                     "Verify via email confirmation. Removal within 48 hours.",
        requires_id_verification=False,
        estimated_days=2,
    ),
    "nuwber.com": RemovalProcedure(
        source_name="nuwber.com",
        procedure_type="web_form",
        opt_out_url="https://nuwber.com/removal/link",
        instructions="Search for your record, request removal, verify phone number or email. "
                     "Complete CAPTCHA verification. Removal within 24-72 hours.",
        requires_id_verification=False,
        estimated_days=3,
    ),
    "ussearch.com": RemovalProcedure(
        source_name="ussearch.com",
        procedure_type="web_form",
        opt_out_url="https://www.ussearch.com/opt-out",
        instructions="Part of PeopleConnect network (same as PeopleFinder). "
                     "May be covered by PeopleFinder removal. If not, search for your record "
                     "and submit direct opt-out form. Verify via email.",
        requires_id_verification=False,
        estimated_days=7,
    ),
    "peoplelooker.com": RemovalProcedure(
        source_name="peoplelooker.com",
        procedure_type="web_form",
        opt_out_url="https://www.peoplelooker.com/opt-out",
        instructions="Owned by BeenVerified. May be covered by BeenVerified removal. "
                     "If not, search for your listing, submit opt-out request with email verification. "
                     "Removal within 48-72 hours.",
        requires_id_verification=False,
        estimated_days=3,
    ),
    "ownerly.com": RemovalProcedure(
        source_name="ownerly.com",
        procedure_type="web_form",
        opt_out_url="https://www.ownerly.com/opt-out",
        instructions="Owned by BeenVerified. May be covered by BeenVerified removal. "
                     "If not, submit opt-out form with property address and email. "
                     "Focus on property records. Verify via email.",
        requires_id_verification=False,
        estimated_days=5,
    ),
    # Premium-only expanded sources (see discovery/agent.py)
    "thatsthem.com": RemovalProcedure(
        source_name="thatsthem.com",
        procedure_type="web_form",
        opt_out_url="https://thatsthem.com/optout",
        instructions="Submit the listing details on the opt-out form. The form uses a CAPTCHA, "
                     "so automated submission stops and the request is reported as needing manual completion.",
        requires_id_verification=False,
        estimated_days=3,
    ),
    "smartbackgroundchecks.com": RemovalProcedure(
        source_name="smartbackgroundchecks.com",
        procedure_type="web_form",
        opt_out_url="https://www.smartbackgroundchecks.com/optout",
        instructions="Find the record and request removal; a confirmation email must be followed.",
        requires_id_verification=False,
        estimated_days=7,
    ),
    "advancedbackgroundchecks.com": RemovalProcedure(
        source_name="advancedbackgroundchecks.com",
        procedure_type="web_form",
        opt_out_url="https://www.advancedbackgroundchecks.com/removal",
        instructions="Two steps: submit name and email, follow the verification link, then complete the removal form.",
        requires_id_verification=False,
        estimated_days=7,
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
