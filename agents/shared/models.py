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
    tier: str  # free | personal | premium


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


class SourceOutcome(str, Enum):
    """What happened when discovery tried to read one source. Mirrors the backend's
    `CoverageOutcome`; the values are what the dashboard shows the customer."""
    CHECKED = "checked"          # the page was read (whether it lists the customer is separate)
    BLOCKED = "blocked"          # the site refused automated access (401/403, CAPTCHA page)
    NOT_FOUND = "not_found"      # the search address no longer works (404/410...)
    UNAVAILABLE = "unavailable"  # down, rate limited, circuit open, or unreachable
    UNREADABLE = "unreadable"    # loaded, but showed no readable results (e.g. built by JavaScript)
    NOT_SUPPORTED = "not_supported"  # only searchable by an interactive form: needs a manual search


class OptOutStatus(str, Enum):
    """What happened when the nightly check fetched a broker's public opt-out page. Mirrors the
    backend's `OptOutPageStatus`. "Reachable" means the page loaded for a plain HTTP request; it
    does NOT mean an automated opt-out succeeds there."""
    REACHABLE = "reachable"
    CAPTCHA = "captcha"          # opens, but its form carries a CAPTCHA a person must complete
    BLOCKED = "blocked"          # 401/403/451, or a "verify you are human" page
    NOT_FOUND = "not_found"      # 404/410...: the registry link needs fixing
    UNAVAILABLE = "unavailable"  # down, rate limited, redirected off-site, or unreachable


class OptOutCheck(BaseModel):
    """One broker's opt-out page, checked. Public information only: a site name and a status."""
    source: str
    status: OptOutStatus
    http_status: Optional[int] = None


class DiscoveryReport(BaseModel):
    """Candidates found, and what happened at every source. A scan that read 3 of 19
    sources and found nothing on the rest is not the same as one that read all 19."""
    candidates: list[DiscoveryResult] = Field(default_factory=list)
    outcomes: dict[str, SourceOutcome] = Field(default_factory=dict)

    @property
    def checked(self) -> int:
        return sum(1 for o in self.outcomes.values() if o is SourceOutcome.CHECKED)

    @property
    def total(self) -> int:
        return len(self.outcomes)

    def coverage(self) -> list[dict[str, str]]:
        """The wire form sent to the backend: public site names and outcomes, nothing else."""
        return [{"source": name, "outcome": outcome.value} for name, outcome in self.outcomes.items()]


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
