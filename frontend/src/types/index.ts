export type SubscriptionTier = "free" | "personal" | "premium";

export type PiiCategory =
  | "full_name"
  | "home_address"
  | "phone_number"
  | "email_address"
  | "date_of_birth"
  | "family_association"
  | "employment_history"
  | "social_media_handle"
  | "vehicle_record"
  | "court_record"
  | "other";

export type ExposureSeverity = "low" | "medium" | "high" | "critical";

export type ScanJobStatus =
  | "queued"
  | "running"
  | "completed"
  | "failed"
  | "cancelled";

export type RemovalStatus =
  | "authorized"
  | "submitted"
  | "acknowledged"
  | "verified_removed"
  | "rejected"
  | "unverifiable"
  | "reappeared"
  | "failed";

export interface CustomerProfile {
  id: string;
  email: string;
  subscription_tier: SubscriptionTier;
  privacy_score: number | null;
  is_verified: boolean;
  created_at: string;
  last_scan_at: string | null;
}

export interface Exposure {
  id: string;
  customer_id: string;
  /** null for a listing the customer added themselves. */
  scan_job_id: string | null;
  /** "scan": found by our scan. "customer": added by the customer (their word, not our detection). */
  origin: "scan" | "customer";
  source_name: string;
  source_url: string;
  pii_category: PiiCategory;
  severity: ExposureSeverity;
  pii_summary: string;
  confidence: number;
  is_active: boolean;
  discovered_at: string;
  last_verified_at: string;
  removed_at: string | null;
}

export interface ExposureSummary {
  customer_id: string;
  total_exposures: number;
  active_exposures: number;
  removed_exposures: number;
  critical_count: number;
  high_count: number;
  medium_count: number;
  low_count: number;
  privacy_score: number;
  sources_affected: string[];
  last_scan_at: string | null;
}

export type SourceOutcome =
  | "checked"
  | "blocked"
  | "not_found"
  | "unavailable"
  | "unreadable"
  | "not_supported";

/** What happened when a scan tried to read one source. `source` is a site name, never a URL. */
export interface SourceCoverage {
  source: string;
  outcome: SourceOutcome;
}

export interface ScanJob {
  id: string;
  customer_id: string;
  status: ScanJobStatus;
  sources_queued: number;
  sources_completed: number;
  sources_failed: number;
  exposures_found: number;
  error_message: string | null;
  /** Per-source outcomes; null for scans that ran before coverage was recorded. */
  coverage: SourceCoverage[] | null;
  queued_at: string;
  started_at: string | null;
  completed_at: string | null;
}

export interface AuthResponse {
  // The session is an HttpOnly cookie; it is deliberately not part of the response body.
  customer: CustomerProfile;
}

export interface RemovalRequest {
  id: string;
  customer_id: string;
  exposure_id: string;
  source_name: string;
  source_url: string;
  status: RemovalStatus;
  procedure_used: string;
  confirmation_ref: string | null;
  evidence_hash: string | null;
  customer_authorized_at: string;
  submitted_at: string | null;
  verified_at: string | null;
  next_check_at: string | null;
  created_at: string;
  updated_at: string;
  /** Why a failed request failed, in plain words; null if it has not failed. */
  failure_reason: string | null;
  /** Most recent re-check of a submitted request, in our own words. */
  last_check_result: "removed" | "still_present" | "inconclusive" | null;
  last_check_at: string | null;
  /** Present when the customer reported doing the opt-out by hand. A claim, not evidence. */
  manual_reported_at: string | null;
  manual_email_confirmation: "confirmed" | "not_required" | null;
  manual_submitted_on: string | null;
  assistance_requested_at: string | null;
  assistance_status: "requested" | "in_progress" | "completed" | "declined" | null;
}

export interface RemovalAuditEntry {
  id: string;
  removal_request_id: string;
  customer_id: string;
  action: string;
  actor: string;
  details: Record<string, unknown>;
  recorded_at: string;
}

// Billing
export type BillingIntervalName = "month" | "year";

export interface BillingPlan {
  tier: "personal" | "premium";
  interval: BillingIntervalName;
  /** Stripe Payment Link with this customer's signed reference already attached. */
  checkout_url: string;
}

export interface BillingSubscription {
  tier: "personal" | "premium";
  interval: BillingIntervalName;
  status: string;
  /** True once the customer has cancelled; the plan stays until current_period_end. */
  cancel_at_period_end: boolean;
  current_period_end: string | null;
}

export interface BillingInfo {
  /** False when the server has no Stripe configuration. */
  enabled: boolean;
  plans: BillingPlan[];
  subscription: BillingSubscription | null;
  portal_url: string | null;
}

// Opt-out page health (nightly check of each broker's public opt-out page)
export type OptOutPageStatus = "reachable" | "captcha" | "blocked" | "not_found" | "unavailable";

export type OptOutHealthKind = "optout" | "ca_rights";

export interface OptOutHealth {
  kind: OptOutHealthKind;
  /** A site name for "optout"; the registry's Broker ID for "ca_rights". */
  source: string;
  status: OptOutPageStatus;
  checked_at: string;
  /** The last time the page opened (even with a CAPTCHA); kept across later failures. */
  last_reachable_at: string | null;
}

// Deletion requests sent to companies on the customer's behalf
export interface DeletionChannel {
  id: string;
  company: string;
  /** Where the request would be sent: the address the company itself publishes. */
  address: string;
  /** The company's own page that publishes it, a short quote from it, and when we last checked. */
  source_url: string;
  evidence: string;
  verified_on: string;
  applies_to: string[];
  requests: string[];
  legal_basis: string;
  agent_requirements: string;
  notes: string;
  /** The company's page asks for the address to find a record, so it cannot be sent without it. */
  requires_address: boolean;
  eligible: boolean;
  ineligible_reason: string | null;
}

export interface DeletionInfo {
  authorization: { version: string; text: string };
  plan_ok: boolean;
  cooldown_days: number;
  channels: DeletionChannel[];
}

export type DeletionStatus = "queued" | "sending" | "sent" | "failed" | "replied" | "removed" | "denied";

export interface DeletionRequest {
  id: string;
  channel_id: string;
  company: string;
  status: DeletionStatus;
  queued_at: string;
  sent_at: string | null;
  outcome_at: string | null;
  follow_up_due: boolean;
  error: string | null;
}
