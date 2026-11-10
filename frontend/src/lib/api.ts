/**
 * VectorScrub API client.
 * All requests go through Next.js rewrites → Rust backend.
 */
import type {
  AuthResponse,
  CustomerProfile,
  Exposure,
  ExposureSummary,
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
  password: string
): Promise<AuthResponse> {
  const res = await fetch(`${BASE}/auth/register`, {
    method: "POST",
    headers: jsonHeaders(),
    credentials: "same-origin",
    body: JSON.stringify({ email, password }),
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

// Profile
export async function getProfile(): Promise<CustomerProfile> {
  const res = await csrfFetch(`${BASE}/me`, { headers: jsonHeaders() });
  return handleResponse<CustomerProfile>(res);
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
