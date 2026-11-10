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

function getToken(): string | null {
  if (typeof window === "undefined") return null;
  return localStorage.getItem("vs_token");
}

function authHeaders(): Record<string, string> {
  const token = getToken();
  return token
    ? { Authorization: `Bearer ${token}`, "Content-Type": "application/json" }
    : { "Content-Type": "application/json" };
}

async function handleResponse<T>(res: Response): Promise<T> {
  if (!res.ok) {
    const text = await res.text().catch(() => res.statusText);
    throw new Error(text || `HTTP ${res.status}`);
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
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ email, password }),
  });
  return handleResponse<AuthResponse>(res);
}

export async function login(
  email: string,
  password: string
): Promise<AuthResponse> {
  const res = await fetch(`${BASE}/auth/login`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ email, password }),
  });
  return handleResponse<AuthResponse>(res);
}

// Profile
export async function getProfile(): Promise<CustomerProfile> {
  const res = await csrfFetch(`${BASE}/me`, { headers: authHeaders() });
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
    headers: authHeaders(),
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
    headers: authHeaders(),
  });
  return handleResponse(res);
}

export async function listScans(): Promise<{ jobs: ScanJob[] }> {
  const res = await csrfFetch(`${BASE}/scans`, { headers: authHeaders() });
  return handleResponse(res);
}

export async function getScan(jobId: string): Promise<{ job: ScanJob }> {
  const res = await csrfFetch(`${BASE}/scans/${jobId}`, {
    headers: authHeaders(),
  });
  return handleResponse(res);
}

// Exposures
export async function listExposures(): Promise<{ exposures: Exposure[] }> {
  const res = await csrfFetch(`${BASE}/exposures`, { headers: authHeaders() });
  return handleResponse(res);
}

export async function getExposureSummary(): Promise<{
  summary: ExposureSummary;
}> {
  const res = await csrfFetch(`${BASE}/exposures/summary`, {
    headers: authHeaders(),
  });
  return handleResponse(res);
}

// Removal Requests
export async function listRemovalRequests(): Promise<{
  requests: RemovalRequest[];
}> {
  const res = await csrfFetch(`${BASE}/removal-requests`, {
    headers: authHeaders(),
  });
  return handleResponse(res);
}

export async function authorizeRemoval(exposureId: string): Promise<{
  request: RemovalRequest;
  message: string;
}> {
  const res = await csrfFetch(`${BASE}/removal-requests/${exposureId}/authorize`, {
    method: "POST",
    headers: authHeaders(),
  });
  return handleResponse(res);
}

export async function getRemovalRequest(requestId: string): Promise<{
  request: RemovalRequest;
  audit_log: RemovalAuditEntry[];
}> {
  const res = await csrfFetch(`${BASE}/removal-requests/${requestId}`, {
    headers: authHeaders(),
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
    headers: authHeaders(),
  });
  return handleResponse(res);
}
