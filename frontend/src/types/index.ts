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
  scan_job_id: string;
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
