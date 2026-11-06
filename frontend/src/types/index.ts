export type SubscriptionTier = "free" | "personal" | "premium" | "family";

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
  | "reappeared";

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

export interface ScanJob {
  id: string;
  customer_id: string;
  status: ScanJobStatus;
  sources_queued: number;
  sources_completed: number;
  sources_failed: number;
  exposures_found: number;
  error_message: string | null;
  queued_at: string;
  started_at: string | null;
  completed_at: string | null;
}

export interface AuthResponse {
  token: string;
  customer: CustomerProfile;
}
