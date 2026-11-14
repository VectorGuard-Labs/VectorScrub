"""Tests for site-specific broker form handlers."""
import pytest

from removal.handlers import (
    get_handler,
    GenericHandler,
    SpokeoHandler,
    WhitePagesHandler,
    BeenVerifiedHandler,
    RadarisHandler,
    TruePeopleSearchHandler,
    FastPeopleSearchHandler,
    InteliusHandler,
    NuwberHandler,
    PeekYouHandler,
)


def test_get_handler_returns_site_specific_handler():
    """get_handler should return site-specific handler when available."""
    handler = get_handler("spokeo.com")
    assert isinstance(handler, SpokeoHandler)
    assert handler.__class__.__name__ == "SpokeoHandler"


def test_get_handler_returns_generic_for_unknown_broker():
    """get_handler should return GenericHandler for brokers without specific handlers."""
    handler = get_handler("unknown-broker.com")
    assert isinstance(handler, GenericHandler)
    assert handler.__class__.__name__ == "GenericHandler"


def test_get_handler_normalizes_broker_names():
    """get_handler should normalize broker names (remove www. prefix)."""
    handler1 = get_handler("spokeo.com")
    handler2 = get_handler("www.spokeo.com")
    assert isinstance(handler1, SpokeoHandler)
    assert isinstance(handler2, SpokeoHandler)


def test_handler_registry_has_all_expected_handlers():
    """Verify handler registry contains all implemented handlers."""
    from removal.handlers import SITE_HANDLERS

    expected_brokers = {
        "spokeo.com",
        "whitepages.com",
        "beenverified.com",
        "radaris.com",
        "truepeoplesearch.com",
        "fastpeoplesearch.com",
        "intelius.com",
        "truthfinder.com",
        "instantcheckmate.com",
        "ussearch.com",
        "nuwber.com",
        "peekyou.com",
    }

    assert set(SITE_HANDLERS.keys()) == expected_brokers


def test_peopleconnect_brokers_share_intelius_handler():
    """PeopleConnect brands should all use InteliusHandler."""
    peopleconnect_brokers = [
        "intelius.com",
        "truthfinder.com",
        "instantcheckmate.com",
        "ussearch.com",
    ]

    for broker in peopleconnect_brokers:
        handler = get_handler(broker)
        assert isinstance(handler, InteliusHandler), f"{broker} should use InteliusHandler"


def test_all_handlers_have_required_methods():
    """All handlers should implement the BrokerHandler interface."""
    from removal.handlers import SITE_HANDLERS

    for broker, handler in SITE_HANDLERS.items():
        # Check required methods exist
        assert hasattr(handler, "fill_form"), f"{broker} handler missing fill_form"
        assert hasattr(handler, "submit"), f"{broker} handler missing submit"
        assert hasattr(handler, "has_captcha"), f"{broker} handler missing has_captcha"
        assert hasattr(handler, "extract_confirmation"), f"{broker} handler missing extract_confirmation"


def test_handler_names_are_correct():
    """Handler instances should have correct class names."""
    assert get_handler("spokeo.com").__class__.__name__ == "SpokeoHandler"
    assert get_handler("whitepages.com").__class__.__name__ == "WhitePagesHandler"
    assert get_handler("beenverified.com").__class__.__name__ == "BeenVerifiedHandler"
    assert get_handler("unknown.com").__class__.__name__ == "GenericHandler"


def test_generic_handler_is_default():
    """GenericHandler should be used for any broker without a specific handler."""
    # Test multiple unknown brokers
    unknown_brokers = [
        "example.com",
        "test-broker.com",
        "another-unknown.com",
    ]

    for broker in unknown_brokers:
        handler = get_handler(broker)
        assert isinstance(handler, GenericHandler), f"{broker} should use GenericHandler"
