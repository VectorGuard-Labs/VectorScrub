"""
Shared configuration for all VectorScrub AI agents.
All settings loaded from environment variables via pydantic-settings.
"""
from functools import lru_cache
from pydantic_settings import BaseSettings, SettingsConfigDict


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
    max_concurrent_sources: int = 5
    request_timeout_seconds: int = 30
    max_retries: int = 3

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


@lru_cache
def get_config() -> AgentConfig:
    return AgentConfig()
