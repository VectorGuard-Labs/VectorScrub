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
        instructions="Spokeo needs the link to one profile, not a search page. Search spokeo.com for your name, open the result that is you (right city and age), and copy that page's address: it looks like spokeo.com/First-Last/City/ST/p12345678. Paste it into the opt-out form with your email, then click the link in Spokeo's confirmation email. If several profiles are you, opt out of each one.",
        requires_id_verification=False,
        estimated_days=7,
        handler_class="SpokeoHandler",
    ),
    "whitepages.com": RemovalProcedure(
        source_name="whitepages.com",
        procedure_type="web_form",
        opt_out_url="https://www.whitepages.com/suppression-requests",
        instructions="Enter listing URL and submit suppression request.",
        requires_id_verification=False,
        estimated_days=3,
        handler_class="WhitePagesHandler",
    ),
    "peoplefinder.com": RemovalProcedure(
        source_name="peoplefinder.com",
        procedure_type="web_form",
        opt_out_url="https://www.intelius.com/privacy-center/",
        instructions="PeopleFinder.com is run alongside Intelius and links to Intelius's privacy center for removal requests. Start there; the PeopleConnect Suppression Center may also apply.",
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
        handler_class="BeenVerifiedHandler",
    ),
    "intelius.com": RemovalProcedure(
        source_name="intelius.com",
        procedure_type="web_form",
        opt_out_url="https://suppression.peopleconnect.us/login",
        instructions="One request in PeopleConnect's shared Suppression Center covers Intelius, TruthFinder, Instant Checkmate and US Search. Verify your email, find your record by name and date of birth, select it and choose to suppress it.",
        requires_id_verification=False,
        estimated_days=7,
        handler_class="InteliusHandler",
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
        opt_out_url="https://suppression.peopleconnect.us/login",
        instructions="One request in PeopleConnect's shared Suppression Center covers Intelius, TruthFinder, Instant Checkmate and US Search. Verify your email, find your record by name and date of birth, select it and choose to suppress it.",
        requires_id_verification=False,
        estimated_days=7,
        handler_class="InteliusHandler",
    ),
    "instantcheckmate.com": RemovalProcedure(
        source_name="instantcheckmate.com",
        procedure_type="web_form",
        opt_out_url="https://suppression.peopleconnect.us/login",
        instructions="One request in PeopleConnect's shared Suppression Center covers Intelius, TruthFinder, Instant Checkmate and US Search. Verify your email, find your record by name and date of birth, select it and choose to suppress it.",
        requires_id_verification=False,
        estimated_days=2,
        handler_class="InteliusHandler",
    ),
    "radaris.com": RemovalProcedure(
        source_name="radaris.com",
        procedure_type="web_form",
        opt_out_url="https://radaris.com/page/how-to-remove",
        instructions="Search for your listing, copy the URL, submit opt-out form. "
                     "Verify via email. Removal completes within 24-72 hours.",
        requires_id_verification=False,
        estimated_days=3,
        handler_class="RadarisHandler",
    ),
    "truepeoplesearch.com": RemovalProcedure(
        source_name="truepeoplesearch.com",
        procedure_type="web_form",
        opt_out_url="https://www.truepeoplesearch.com/removal",
        instructions="Search for your record, click 'Remove', enter email for verification link. "
                     "Complete removal via verification email. Removal typically within 72 hours.",
        requires_id_verification=False,
        estimated_days=3,
        handler_class="TruePeopleSearchHandler",
    ),
    "fastpeoplesearch.com": RemovalProcedure(
        source_name="fastpeoplesearch.com",
        procedure_type="web_form",
        opt_out_url="https://www.fastpeoplesearch.com/removal",
        instructions="Search for your listing, select it, submit removal request. "
                     "Verify via email. Removal completes within 24 hours.",
        requires_id_verification=False,
        estimated_days=1,
        handler_class="FastPeopleSearchHandler",
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
        handler_class="NuwberHandler",
        # Phase 3: CAPTCHA handling
        supports_human_captcha=True,  # Frequent CAPTCHA, human intervention supported
        prefers_email_for_captcha=False,  # Has web form, try it first
    ),
    "ussearch.com": RemovalProcedure(
        source_name="ussearch.com",
        procedure_type="web_form",
        opt_out_url="https://suppression.peopleconnect.us/login",
        instructions="One request in PeopleConnect's shared Suppression Center covers Intelius, TruthFinder, Instant Checkmate and US Search. Verify your email, find your record by name and date of birth, select it and choose to suppress it.",
        requires_id_verification=False,
        estimated_days=7,
        handler_class="InteliusHandler",
    ),
    "peoplelooker.com": RemovalProcedure(
        source_name="peoplelooker.com",
        procedure_type="web_form",
        opt_out_url="https://www.peoplelooker.com/optout",
        instructions="Search your name, select your record and confirm the verification email PeopleLooker sends. The site sends this address on to its current opt-out search page.",
        requires_id_verification=False,
        estimated_days=3,
    ),
    "ownerly.com": RemovalProcedure(
        source_name="ownerly.com",
        procedure_type="web_form",
        opt_out_url="https://www.ownerly.com/optout",
        instructions="Search by name and state, select your record, enter your email and confirm the verification email. Ownerly's opt-outs are managed by BeenVerified, so a BeenVerified removal may also cover it.",
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
        # Phase 3: CAPTCHA handling
        supports_human_captcha=True,  # Always has CAPTCHA, human intervention needed
        prefers_email_for_captcha=False,  # Web form is the only option
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
        opt_out_url="https://www.advancedbackgroundchecks.com/opt-out",
        instructions="Two steps: submit your name and email, follow the verification link within 24 hours, then complete the removal form.",
        requires_id_verification=False,
        estimated_days=7,
    ),
    # Phase 1 Expansion - Low-Hanging Fruit
    "peekyou.com": RemovalProcedure(
        source_name="peekyou.com",
        procedure_type="web_form",
        opt_out_url="https://www.peekyou.com/about/contact/optout/",
        instructions="Enter your full name and email address. PeekYou will send a verification email. Click the link in the email to confirm your opt-out request. Removal typically completes within 48 hours.",
        requires_id_verification=False,
        estimated_days=2,
        handler_class="PeekYouHandler",
    ),
    "addresses.com": RemovalProcedure(
        source_name="addresses.com",
        procedure_type="email",
        opt_out_email="support@addresses.com",
        instructions="Email with subject 'Privacy Opt-Out Request' including your full name, current address, and any profile URLs you want removed.",
        requires_id_verification=False,
        estimated_days=10,
    ),
    "usphonebook.com": RemovalProcedure(
        source_name="usphonebook.com",
        procedure_type="web_form",
        opt_out_url="https://www.usphonebook.com/opt-out",
        instructions="Search for your listing, select it, and submit the opt-out form with your email for verification. Removal completes within 72 hours.",
        requires_id_verification=False,
        estimated_days=3,
    ),
    "anywho.com": RemovalProcedure(
        source_name="anywho.com",
        procedure_type="web_form",
        opt_out_url="https://www.anywho.com/whitepages/suppression-requests",
        instructions="AnyWho is operated by Whitepages. Use the Whitepages suppression request form. Enter your listing URL and submit.",
        requires_id_verification=False,
        estimated_days=3,
    ),
    "411.com": RemovalProcedure(
        source_name="411.com",
        procedure_type="email",
        opt_out_email="privacy@411.com",
        instructions="Email with subject 'Opt-Out Request' including your full name, phone number, address, and profile URL.",
        requires_id_verification=False,
        estimated_days=7,
    ),
    "checkpeople.com": RemovalProcedure(
        source_name="checkpeople.com",
        procedure_type="web_form",
        opt_out_url="https://www.checkpeople.com/opt-out",
        instructions="Enter your name and email, then verify via email link. Removal typically within 48 hours.",
        requires_id_verification=False,
        estimated_days=2,
    ),
    "peoplewiz.com": RemovalProcedure(
        source_name="peoplewiz.com",
        procedure_type="web_form",
        opt_out_url="https://www.peoplewiz.com/privacy",
        instructions="Fill out the privacy request form with your name, email, and profile URL. Verify via email. Removal within 72 hours.",
        requires_id_verification=False,
        estimated_days=3,
    ),
    "infotracer.com": RemovalProcedure(
        source_name="infotracer.com",
        procedure_type="web_form",
        opt_out_url="https://infotracer.com/optout",
        instructions="Enter personal details to find your record, then submit opt-out request. Email verification required. Removal within 5-7 days.",
        requires_id_verification=False,
        estimated_days=7,
    ),
    "publicrecordsnow.com": RemovalProcedure(
        source_name="publicrecordsnow.com",
        procedure_type="email",
        opt_out_email="support@publicrecordsnow.com",
        instructions="Email with subject 'Privacy Request - Opt Out' including your full name, address, and profile URL. Allow 7-10 business days.",
        requires_id_verification=False,
        estimated_days=10,
    ),
    "myaddressbook.com": RemovalProcedure(
        source_name="myaddressbook.com",
        procedure_type="email",
        opt_out_email="privacy@myaddressbook.com",
        instructions="Email with subject 'Data Removal Request' including your name and address. Allow 14 days for processing.",
        requires_id_verification=False,
        estimated_days=14,
    ),
    "neighbor.report": RemovalProcedure(
        source_name="neighbor.report",
        procedure_type="web_form",
        opt_out_url="https://neighbor.report/opt-out",
        instructions="Enter your name and address to search for your record. Select it and submit opt-out. Email verification required. Removal within 48-72 hours.",
        requires_id_verification=False,
        estimated_days=3,
    ),
    "pipl.com": RemovalProcedure(
        source_name="pipl.com",
        procedure_type="web_form",
        opt_out_url="https://pipl.com/privacy/remove/",
        instructions="Enter your profile URL and email address. Pipl will send verification email. Click link to confirm removal. Processing takes 48 hours.",
        requires_id_verification=False,
        estimated_days=2,
    ),
    "zoominfo.com": RemovalProcedure(
        source_name="zoominfo.com",
        procedure_type="web_form",
        opt_out_url="https://www.zoominfo.com/update/remove",
        instructions="Search for your profile, enter your email and verify. ZoomInfo is B2B-focused but includes some consumer records. Removal within 24 hours for verified requests.",
        requires_id_verification=False,
        estimated_days=1,
    ),
    "contactout.com": RemovalProcedure(
        source_name="contactout.com",
        procedure_type="email",
        opt_out_email="privacy@contactout.com",
        instructions="Email with subject 'GDPR Data Removal Request' including your name, email, and any associated profile URLs. B2B service, allow 30 days per GDPR compliance.",
        requires_id_verification=False,
        estimated_days=30,
    ),
    "rocketreach.co": RemovalProcedure(
        source_name="rocketreach.co",
        procedure_type="email",
        opt_out_email="privacy@rocketreach.co",
        instructions="Email with subject 'Data Removal Request - CCPA/GDPR' including your full name, work email, and profile URL. B2B-focused service. Allow 30 days.",
        requires_id_verification=False,
        estimated_days=30,
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
