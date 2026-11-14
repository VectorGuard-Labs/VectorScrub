"""Site-specific broker removal handlers."""
from .base import BrokerHandler
from .generic import GenericHandler
from .spokeo import SpokeoHandler
from .whitepages import WhitePagesHandler
from .truepeoplesearch import TruePeopleSearchHandler
from .beenverified import BeenVerifiedHandler
from .radaris import RadarisHandler
from .fastpeoplesearch import FastPeopleSearchHandler
from .intelius import InteliusHandler
from .nuwber import NuwberHandler
from .peekyou import PeekYouHandler

# Handler registry - maps broker domain to handler instances
SITE_HANDLERS = {
    "spokeo.com": SpokeoHandler(),
    "whitepages.com": WhitePagesHandler(),
    "beenverified.com": BeenVerifiedHandler(),
    "radaris.com": RadarisHandler(),
    "truepeoplesearch.com": TruePeopleSearchHandler(),
    "fastpeoplesearch.com": FastPeopleSearchHandler(),
    # PeopleConnect suppression center (shared handler)
    "intelius.com": InteliusHandler(),
    "truthfinder.com": InteliusHandler(),
    "instantcheckmate.com": InteliusHandler(),
    "ussearch.com": InteliusHandler(),
    "nuwber.com": NuwberHandler(),
    "peekyou.com": PeekYouHandler(),
}


def get_handler(broker_name: str, handler_class_name: str | None = None) -> BrokerHandler:
    """
    Get the appropriate handler instance for a broker.

    Args:
        broker_name: Normalized broker name (e.g., "spokeo.com")
        handler_class_name: Optional explicit handler class name (unused, for future)

    Returns:
        Site-specific handler instance if available, otherwise GenericHandler instance
    """
    # Normalize broker name
    normalized = broker_name.lower().removeprefix("www.")

    # Check registry first
    if normalized in SITE_HANDLERS:
        return SITE_HANDLERS[normalized]

    # Fall back to generic
    return GenericHandler()


__all__ = [
    "BrokerHandler",
    "GenericHandler",
    "SpokeoHandler",
    "WhitePagesHandler",
    "TruePeopleSearchHandler",
    "BeenVerifiedHandler",
    "RadarisHandler",
    "FastPeopleSearchHandler",
    "InteliusHandler",
    "NuwberHandler",
    "PeekYouHandler",
    "SITE_HANDLERS",
    "get_handler",
]
