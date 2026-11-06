"""
Shared data models for agent-to-agent and agent-to-backend communication.
"""
from __future__ import annotations
from enum import Enum
from typing import Optional
from pydantic import BaseModel, Field
import uuid


class PiiCategory(str, Enum):
    FULL_NAME = "full_name"
    HOME_ADDRESS = "home_address"
    PHONE_NUMBER = "phone_number"
    EMAIL_ADDRESS = "email_address"
    DATE_OF_BIRTH = "date_of_birth"
    FAMILY_ASSOCIATION = "family_association"
    EMPLOYMENT_HISTORY = "employment_history"
    SOCIAL_MEDIA_HANDLE = "social_media_handle"
    VEHICLE_RECORD = "vehicle_record"
    COURT_RECORD = "court_record"
    OTHER = "other"


class ExposureSeverity(str, Enum):
    LOW = "low"
    MEDIUM = "medium"
    HIGH = "high"
    CRITICAL = "critical"


class ScanJobStatus(str, Enum):
    QUEUED = "queued"
    RUNNING = "running"
    COMPLETED = "completed"
    FAILED = "failed"
    CANCELLED = "cancelled"


class ScanJobPayload(BaseModel):
    """Payload consumed from the Redis scan queue."""
    job_id: uuid.UUID
    customer_id: uuid.UUID
    identity_token: str
    tier: str  # free | personal | premium | family


class IdentityContext(BaseModel):
    """
    Minimal PII context passed to agents.
    Only the fields needed for a specific check are populated.
    Never logged or stored — zeroized after use.
    """
    customer_id: uuid.UUID
    first_name: str = ""
    last_name: str = ""
    full_name: str = ""
    email: Optional[str] = None
    current_address: str = ""
    previous_addresses: list[str] = Field(default_factory=list)
    phone_numbers: list[str] = Field(default_factory=list)
    email_addresses: list[str] = Field(default_factory=list)
    date_of_birth: Optional[str] = None


class DiscoveryResult(BaseModel):
    """A single candidate URL where customer PII may appear."""
    source_name: str
    source_url: str
    raw_text_snippet: str
    confidence: float = Field(ge=0.0, le=1.0)


class DetectionResult(BaseModel):
    """Confirmed PII exposure after detection agent analysis."""
    source_name: str
    source_url: str
    pii_category: PiiCategory
    severity: ExposureSeverity
    pii_summary: str  # masked (e.g. "123 Main St***") — never raw
    confidence: float = Field(ge=0.0, le=1.0)


class RemovalProcedure(BaseModel):
    """How to opt out of a specific data source."""
    source_name: str
    procedure_type: str  # "web_form" | "email" | "drop_api" | "manual"
    opt_out_url: Optional[str] = None
    opt_out_email: Optional[str] = None
    instructions: str = ""
    requires_id_verification: bool = False
    estimated_days: int = 14
