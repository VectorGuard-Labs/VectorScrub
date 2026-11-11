"""
Shared configuration for all VectorScrub AI agents.
All settings loaded from environment variables via pydantic-settings.
"""
import os
from functools import lru_cache

from dotenv import dotenv_values
from pydantic_settings import BaseSettings, SettingsConfigDict

# Secrets that belong to the Rust backend and must never be visible to the Python agents.
# The master key decrypts every customer's identity; the agents get identity only through the
# backend's scoped release endpoints. The JWT secret signs customer sessions, and the database
# credentials reach the data directly. An agent that can see any of these has more access than
# its design gives it, and a compromised agent (it fetches and parses untrusted web pages)
# would inherit it.
BACKEND_ONLY_SECRETS = ("MASTER_ENCRYPTION_KEY", "JWT_SECRET", "DATABASE_URL", "POSTGRES_PASSWORD", "STRIPE_WEBHOOK_SECRET")


class ForbiddenSecretsError(RuntimeError):
    """The agent's environment contains backend-only secrets."""


class AgentConfig(BaseSettings):
    model_config = SettingsConfigDict(env_file=".env", env_file_encoding="utf-8", extra="ignore")

    # Rust backend
    backend_api_url: str = "http://127.0.0.1:8080"
    agent_api_key: str  # matches backend's expected X-Agent-Key value

    # LLM providers (at least one required)
    openai_api_key: str = ""
    anthropic_api_key: str = ""

    # Default model for classification and reasoning tasks
    default_llm_provider: str = "anthropic"  # "openai" | "anthropic"
    default_llm_model: str = "claude-3-5-sonnet-20241022"

    # Redis
    redis_url: str = "redis://127.0.0.1:6379"
    scan_queue_key: str = "vectorscrub:scan_jobs"

    # Agent behaviour
    scan_heartbeat_seconds: int = 30  # how often a worker tells the backend it is still alive
    max_concurrent_sources: int = 5
    request_timeout_seconds: int = 30
    max_retries: int = 3

    # Monitoring worker (removal verification + reappearance checks)
    monitoring_batch_size: int = 25            # targets fetched from the backend per cycle
    monitoring_idle_seconds: int = 3600        # sleep when nothing is due
    monitoring_request_delay_seconds: float = 2.0  # politeness delay between source fetches

    # Removal worker
    removal_request_delay_seconds: float = 5.0  # spacing between submissions to data sources

    # PII guard: person-name detection (spaCy) used before any text reaches an LLM
    pii_ner_model: str = "en_core_web_sm"
    # True (default): if the NER model cannot be loaded, LLM calls are blocked (fail
    # closed). False: continue on the vocabulary filter alone, which still removes
    # every non-vocabulary word but gives the model less structure ("…" instead of
    # "[NAME]"). Use False only for local development without the model installed.
    pii_ner_required: bool = True

    # PII safety: never log or transmit raw PII beyond what's minimally needed
    pii_log_masking: bool = True

    # SMTP configuration for email-based opt-out requests
    smtp_host: str = "smtp.gmail.com"
    smtp_port: int = 587
    smtp_username: str = ""
    smtp_password: str = ""
    smtp_from_email: str = ""
    smtp_use_tls: bool = True

    # California DROP API configuration
    drop_api_url: str = "https://privacy.ca.gov/api/v1"
    drop_api_key: str = ""
    drop_api_secret: str = ""
    drop_enable_sandbox: bool = True  # Use sandbox environment for testing


def assert_no_backend_secrets(env_file: str | None = None) -> None:
    """
    Refuse to run if the process environment, or the .env file the settings would read,
    contains a backend-only secret. Names only are reported, never values.
    """
    found = [name for name in BACKEND_ONLY_SECRETS if os.environ.get(name)]
    if env_file and os.path.isfile(env_file):
        found += [
            f"{name} (in {env_file})"
            for name, value in dotenv_values(env_file).items()
            if name in BACKEND_ONLY_SECRETS and value
        ]
    if found:
        raise ForbiddenSecretsError(
            "The agents must not have backend-only secrets, but found: "
            + ", ".join(found)
            + ". Give the agent process its own environment (BACKEND_API_URL, AGENT_API_KEY, "
            "REDIS_URL, LLM and SMTP settings only); see .env.example."
        )


@lru_cache
def get_config() -> AgentConfig:
    assert_no_backend_secrets(AgentConfig.model_config.get("env_file"))
    return AgentConfig()
