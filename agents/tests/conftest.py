"""
Test setup.

* Puts the agents package on sys.path.
* Never reads a real .env: pydantic-settings would otherwise load agents/.env (real API
  keys) when pytest runs from this directory.
"""
import os
import sys
import uuid
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

os.environ.update(
    AGENT_API_KEY="test-agent-key-" + "x" * 30,
    ANTHROPIC_API_KEY="test-not-a-real-key",
    DEFAULT_LLM_PROVIDER="anthropic",
    BACKEND_API_URL="http://backend.test",
    MONITORING_REQUEST_DELAY_SECONDS="0",
    REMOVAL_REQUEST_DELAY_SECONDS="0",
    SCAN_HEARTBEAT_SECONDS="1",
)

from shared import config as _config  # noqa: E402

_config.AgentConfig.model_config["env_file"] = None  # never load a real .env in tests
_config.get_config.cache_clear()

from shared.models import IdentityContext  # noqa: E402


@pytest.fixture
def identity() -> IdentityContext:
    """A customer whose every field appears somewhere in the test pages."""
    return IdentityContext(
        customer_id=uuid.uuid4(),
        first_name="Jane",
        last_name="Smith",
        full_name="Jane Marie Smith",
        email="jane.smith@example.com",
        current_address="123 Main St, Austin, TX 78701",
        previous_addresses=["45 Oak Ave, Dallas, TX 75201"],
        phone_numbers=["(512) 555-0147", "+1 214-555-0199"],
        email_addresses=["jane.smith@example.com"],
        date_of_birth="1984-03-09",
    )


@pytest.fixture(autouse=True)
def _fresh_circuit_breakers():
    """Circuit breakers are process-global; do not let one test's failures open another's."""
    from shared import circuit_breaker

    circuit_breaker._circuit_breakers.clear()
    yield
    circuit_breaker._circuit_breakers.clear()


class FakeChatModel:
    """Stands in for the LLM. Records every message it is sent."""

    def __init__(self, *, is_match=True, confidence=0.9, error: Exception | None = None):
        self.sent: list[list] = []
        self._is_match, self._confidence, self._error = is_match, confidence, error

    def with_structured_output(self, schema):
        outer = self

        class _Structured:
            async def ainvoke(self, messages):
                outer.sent.append(messages)
                if outer._error:
                    raise outer._error
                return schema(
                    is_match=outer._is_match,
                    pii_categories=["full_name"],
                    severity="high",
                    confidence=outer._confidence,
                )

        return _Structured()

    @property
    def payloads(self) -> str:
        return "\n".join(m.content for call in self.sent for m in call)


@pytest.fixture
def fake_llm():
    return FakeChatModel()
