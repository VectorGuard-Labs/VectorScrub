/**
 * VectorScrub API client.
 * All requests go through Next.js rewrites → Rust backend.
 */
import type {
  AuthResponse,
  BillingInfo,
  DeletionInfo,
  DeletionRequest,
  CustomerProfile,
  Exposure,
  ExposureSummary,
  OptOutHealth,
  OptOutHealthKind,
  RemovalAuditEntry,
  RemovalRequest,
  ScanJob,
} from "@/types";

import { csrfFetch } from "@/lib/csrf";

const BASE = "/api/v1";

/**
 * The session is an HttpOnly cookie set by the server at login. This code never sees it, and
 * nothing is stored in localStorage: a script injected into the page cannot read the session,
 * only (at worst) act within it while the page is open. Same-origin fetches send the cookie
 * automatically; state-changing requests also carry the CSRF header (see csrf.ts).
 */
function jsonHeaders(): Record<string, string> {
  return { "Content-Type": "application/json" };
}

/** An error response from the API, with the HTTP status so callers can tell 401 from the rest. */
export class ApiError extends Error {
  constructor(
    public readonly status: number,
    message: string
  ) {
    super(message);
    this.name = "ApiError";
  }
}

/**
 * The human-readable message of an error response. The API answers every error with
 * `{"message": "..."}`; anything else (a proxy's HTML page, an empty body) falls back to the
 * raw text or the HTTP status, so the user never sees raw JSON.
 */
export async function errorMessage(res: Response): Promise<string> {
  const text = (await res.text().catch(() => "")).trim();
  try {
    const body: unknown = JSON.parse(text);
    if (body && typeof body === "object" && typeof (body as { message?: unknown }).message === "string") {
      return (body as { message: string }).message;
    }
  } catch {
    // not JSON
  }
  return text || res.statusText || `HTTP ${res.status}`;
}

async function handleResponse<T>(
  res: Response,
  { authenticated = true }: { authenticated?: boolean } = {}
): Promise<T> {
  if (!res.ok) {
    // The session is missing or has expired: send the user to sign in again instead of
    // leaving every page to show its own "failed to load". (Login and register are not
    // "authenticated" calls; a 401 there just means wrong credentials.)
    if (authenticated && res.status === 401 && typeof window !== "undefined") {
      window.location.assign("/login");
    }
    throw new ApiError(res.status, await errorMessage(res));
  }
  return res.json() as Promise<T>;
}

// Auth
export async function register(
  email: string,
  password: string,
  acceptedTermsVersion: string,
  pilotConsents: { interview: boolean; story: boolean } = { interview: false, story: false }
): Promise<AuthResponse> {
  const res = await fetch(`${BASE}/auth/register`, {
    method: "POST",
    headers: jsonHeaders(),
    credentials: "same-origin",
    body: JSON.stringify({
      email,
      password,
      accepted_terms_version: acceptedTermsVersion,
      pilot_interview_ok: pilotConsents.interview,
      pilot_story_ok: pilotConsents.story,
    }),
  });
  return handleResponse<AuthResponse>(res, { authenticated: false });
}

export async function login(
  email: string,
  password: string
): Promise<AuthResponse> {
  const res = await fetch(`${BASE}/auth/login`, {
    method: "POST",
    headers: jsonHeaders(),
    credentials: "same-origin",
    body: JSON.stringify({ email, password }),
  });
  return handleResponse<AuthResponse>(res, { authenticated: false });
}

/** Ends the session: the server expires the cookie. Never throws; signing out always "works". */
export async function logout(): Promise<void> {
  try {
    await fetch(`${BASE}/auth/logout`, {
      method: "POST",
      credentials: "same-origin",
    });
  } catch {
    // Offline or the server is down: nothing more can be done from here.
  }
}

/**
 * Permanently erases the signed-in account (needs the password again). A wrong password is a
 * 401 that must be shown on the form, not treated as an expired session, so this does not use
 * handleResponse's redirect.
 */
export async function deleteAccount(password: string): Promise<void> {
  const res = await csrfFetch(`${BASE}/me`, {
    method: "DELETE",
    headers: jsonHeaders(),
    body: JSON.stringify({ password }),
  });
  if (!res.ok) throw new ApiError(res.status, await errorMessage(res));
}

/** Track a listing the customer found themselves on a site we cannot scan. */
export async function addOwnListing(sourceUrl: string): Promise<{ exposure: Exposure; message: string }> {
  const res = await csrfFetch(`${BASE}/me/listings`, {
    method: "POST",
    headers: jsonHeaders(),
    body: JSON.stringify({ source_url: sourceUrl }),
  });
  return handleResponse(res);
}

// Profile
export async function getProfile(): Promise<CustomerProfile> {
  const res = await csrfFetch(`${BASE}/me`, { headers: jsonHeaders() });
  return handleResponse<CustomerProfile>(res);
}

// Public: whether each broker's opt-out page loaded at its last nightly check. No session needed.
export async function getOptOutHealth(kind: OptOutHealthKind = "optout"): Promise<OptOutHealth[]> {
  const res = await fetch(`${BASE}/public/optout-health?kind=${kind}`, { credentials: "same-origin" });
  const body = await handleResponse<{ checks: OptOutHealth[] }>(res, { authenticated: false });
  return body.checks;
}

// The US state of the saved address as two letters, or null when there is none or it cannot be
// told. Never the address itself. Used to offer California's DROP service to Californians only.
export async function getResidenceState(): Promise<string | null> {
  const res = await csrfFetch(`${BASE}/me/residence`, { headers: jsonHeaders() });
  const body = await handleResponse<{ state: string | null }>(res);
  return body.state;
}

// Deletion requests sent to companies on the customer's behalf. Every call answers 404 while the
// feature is switched off on the server.
export async function getDeletionInfo(): Promise<DeletionInfo> {
  const res = await csrfFetch(`${BASE}/deletion-requests/info`, { headers: jsonHeaders() });
  return handleResponse<DeletionInfo>(res);
}

export async function listDeletionRequests(): Promise<DeletionRequest[]> {
  const res = await csrfFetch(`${BASE}/deletion-requests`, { headers: jsonHeaders() });
  const body = await handleResponse<{ requests: DeletionRequest[] }>(res);
  return body.requests;
}

export async function createDeletionCampaign(payload: {
  channel_ids: string[];
  include_address: boolean;
  consent: boolean;
  authorization_version: string;
}): Promise<{ message: string }> {
  const res = await csrfFetch(`${BASE}/deletion-campaigns`, {
    method: "POST",
    headers: jsonHeaders(),
    body: JSON.stringify(payload),
  });
  return handleResponse(res);
}

export async function reportDeletionOutcome(id: string, outcome: "replied" | "removed" | "denied"): Promise<void> {
  const res = await csrfFetch(`${BASE}/deletion-requests/${id}/outcome`, {
    method: "POST",
    headers: jsonHeaders(),
    body: JSON.stringify({ outcome }),
  });
  if (!res.ok) throw new ApiError(res.status, await errorMessage(res));
}

// Billing
export async function getBilling(): Promise<BillingInfo> {
  const res = await csrfFetch(`${BASE}/me/billing`, { headers: jsonHeaders() });
  return handleResponse<BillingInfo>(res);
}

// Identity
export interface IdentityPayload {
  full_name: string;
  current_address: string;
  previous_addresses: string[];
  phone_numbers: string[];
  email_addresses: string[];
  date_of_birth?: string;
}

export async function submitIdentity(payload: IdentityPayload): Promise<void> {
  const res = await csrfFetch(`${BASE}/me/identity`, {
    method: "POST",
    headers: jsonHeaders(),
    body: JSON.stringify(payload),
  });
  await handleResponse<unknown>(res);
}

// Scans
export async function initiateSccan(): Promise<{
  job_id: string;
  status: string;
  message: string;
}> {
  const res = await csrfFetch(`${BASE}/scans`, {
    method: "POST",
    headers: jsonHeaders(),
  });
  return handleResponse(res);
}

export async function listScans(): Promise<{ jobs: ScanJob[] }> {
  const res = await csrfFetch(`${BASE}/scans`, { headers: jsonHeaders() });
  return handleResponse(res);
}

export async function getScan(jobId: string): Promise<{ job: ScanJob }> {
  const res = await csrfFetch(`${BASE}/scans/${jobId}`, {
    headers: jsonHeaders(),
  });
  return handleResponse(res);
}

// Exposures
export async function listExposures(): Promise<{ exposures: Exposure[] }> {
  const res = await csrfFetch(`${BASE}/exposures`, { headers: jsonHeaders() });
  return handleResponse(res);
}

export async function getExposureSummary(): Promise<{
  summary: ExposureSummary;
}> {
  const res = await csrfFetch(`${BASE}/exposures/summary`, {
    headers: jsonHeaders(),
  });
  return handleResponse(res);
}

// Removal Requests
export async function listRemovalRequests(): Promise<{
  requests: RemovalRequest[];
}> {
  const res = await csrfFetch(`${BASE}/removal-requests`, {
    headers: jsonHeaders(),
  });
  return handleResponse(res);
}

export async function authorizeRemoval(exposureId: string): Promise<{
  request: RemovalRequest;
  message: string;
}> {
  const res = await csrfFetch(`${BASE}/removal-requests/${exposureId}/authorize`, {
    method: "POST",
    headers: jsonHeaders(),
  });
  return handleResponse(res);
}

export interface ManualReport {
  /** Did the site ask you to confirm by email? Only report once you have clicked the link. */
  email_confirmation: "confirmed" | "not_required";
  /** YYYY-MM-DD, the day you submitted it. Defaults to today. */
  submitted_on?: string;
  /** Optional: the exact profile link you opted out, on the same site. We re-check that page. */
  listing_url?: string;
}

export async function markRemovalDoneByHand(
  requestId: string,
  report: ManualReport
): Promise<{ request: RemovalRequest; message: string }> {
  const res = await csrfFetch(`${BASE}/removal-requests/${requestId}/done-by-hand`, {
    method: "POST",
    headers: jsonHeaders(),
    body: JSON.stringify(report),
  });
  return handleResponse(res);
}

export async function updateListingLink(
  requestId: string,
  listingUrl: string
): Promise<{ request: RemovalRequest; message: string }> {
  const res = await csrfFetch(`${BASE}/removal-requests/${requestId}/listing-link`, {
    method: "POST",
    headers: jsonHeaders(),
    body: JSON.stringify({ listing_url: listingUrl }),
  });
  return handleResponse(res);
}

export async function requestRemovalAssistance(
  requestId: string
): Promise<{ request: RemovalRequest; message: string }> {
  const res = await csrfFetch(`${BASE}/removal-requests/${requestId}/request-assistance`, {
    method: "POST",
    headers: jsonHeaders(),
  });
  return handleResponse(res);
}

export async function getRemovalRequest(requestId: string): Promise<{
  request: RemovalRequest;
  audit_log: RemovalAuditEntry[];
}> {
  const res = await csrfFetch(`${BASE}/removal-requests/${requestId}`, {
    headers: jsonHeaders(),
  });
  return handleResponse(res);
}

// Privacy Score History
export interface PrivacyScoreHistoryEntry {
  id: string;
  customer_id: string;
  score: number;
  critical_count: number;
  high_count: number;
  medium_count: number;
  low_count: number;
  total_exposures: number;
  recorded_at: string;
}

export interface PrivacyScoreTrend {
  current_score: number;
  previous_score: number | null;
  score_delta: number | null;
  history: PrivacyScoreHistoryEntry[];
}

export async function getPrivacyScoreHistory(days: number = 30): Promise<PrivacyScoreTrend> {
  const res = await csrfFetch(`${BASE}/privacy-score/history?days=${days}`, {
    headers: jsonHeaders(),
  });
  return handleResponse(res);
}
