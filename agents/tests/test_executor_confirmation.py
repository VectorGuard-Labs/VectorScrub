"""
How the web-form executor reads the page a broker shows after a submission. Pure text rules,
so these run without a browser (the end-to-end version is in test_executor_browser.py).

The contract: "submitted" only on positive evidence. These cases are the ones that used to be
accepted by mistake: ordinary words after "case", "reference" and "confirmation" became a
"confirmation reference", and a reference on an error page bypassed the failure check.
"""
import pytest

from removal.executor import find_confirmation_reference, judge_result_page


@pytest.mark.parametrize("text", [
    "In case of error, contact support.",
    "Please check your confirmation email.",
    "For reference, see our privacy policy.",
    "Case closed.",
    "Your request ID: updated",           # labelled, but not an identifier (no digit)
    "Reference: ab1",                     # too short to be a reference
    "Request in progress. Ticket: please wait",
    "Confirmation: required",
    "",
])
def test_ordinary_prose_is_not_a_confirmation_reference(text):
    assert find_confirmation_reference(text) is None


@pytest.mark.parametrize("text, expected", [
    ("Reference: ABC-12345", "ABC-12345"),
    ("Your confirmation number is 98765432.", "98765432"),
    ("Confirmation #: A1B2C3D4", "A1B2C3D4"),
    ("Request ID: XYZ789", "XYZ789"),
    ("Ticket: T-20481 has been opened", "T-20481"),
    ("Case #4521", "4521"),
    ("reference no. 5521-AA", "5521-AA"),
])
def test_a_labelled_identifier_is_a_confirmation_reference(text, expected):
    assert find_confirmation_reference(text) == expected


def test_a_page_of_prose_with_no_evidence_is_not_accepted():
    v = judge_result_page("Opt-out. Nothing to see here. In case of questions see the FAQ.")
    assert not v.accepted and v.reason == "no_evidence"


def test_an_error_page_that_mentions_a_case_is_not_accepted():
    v = judge_result_page("Sorry. In case of error please try again later.")
    assert not v.accepted and v.reference is None


def test_error_wording_beats_a_reference_on_the_same_page():
    # error pages routinely echo a request or case id
    v = judge_result_page("An error occurred. Request ID: REQ-99812. Please try again.")
    assert not v.accepted and v.reason == "failure_wording"
    assert v.reference is None, "a reference from a failed submission must not be recorded as a confirmation"


def test_polite_wording_does_not_outweigh_an_error():
    assert not judge_result_page("Thank you for contacting us. However, an error occurred.").accepted


def test_a_bare_thank_you_is_not_evidence():
    assert not judge_result_page("Thank you for visiting.").accepted


def test_a_reference_alone_is_evidence():
    v = judge_result_page("Your request is being processed. Reference: ABC-12345")
    assert v.accepted and v.reference == "ABC-12345" and v.reason == "reference"


def test_specific_acceptance_wording_is_evidence_without_a_reference():
    for text in [
        "Your request has been submitted.",
        "We have received your opt-out request.",
        "Please check your email to verify your request.",
        "Thank you for your request.",
        "Submitted successfully.",
    ]:
        v = judge_result_page(text)
        assert v.accepted and v.reference is None, text


def test_captcha_or_human_check_after_submitting_is_not_acceptance():
    assert not judge_result_page("Please verify you are human. Your request has been submitted.").accepted


def test_the_page_title_is_no_longer_evidence():
    # the old code returned "page_title:..." as a reference when a title said "success"/"confirm"
    assert judge_result_page("Welcome").reference is None


def test_the_good_fixture_page_from_the_browser_test_is_accepted():
    v = judge_result_page("Thank you\nYour request has been submitted. Reference: ABC-12345")
    assert v.accepted and v.reference == "ABC-12345"


def test_the_error_fixture_page_from_the_browser_test_is_rejected():
    assert not judge_result_page("Thank you for contacting us. However, an error occurred. Please try again.").accepted
