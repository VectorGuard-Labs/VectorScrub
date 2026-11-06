/**
 * VectorScrub API client.
 * All requests go through Next.js rewrites → Rust backend.
 */
import type {
  AuthResponse,
  CustomerProfile,
  Exposure,
  ExposureSummary,
  ScanJob,
} from "@/types";

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
  const res = await fetch(`${BASE}/me`, { headers: authHeaders() });
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
  const res = await fetch(`${BASE}/me/identity`, {
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
  const res = await fetch(`${BASE}/scans`, {
    method: "POST",
    headers: authHeaders(),
  });
  return handleResponse(res);
}

export async function listScans(): Promise<{ jobs: ScanJob[] }> {
  const res = await fetch(`${BASE}/scans`, { headers: authHeaders() });
  return handleResponse(res);
}

export async function getScan(jobId: string): Promise<{ job: ScanJob }> {
  const res = await fetch(`${BASE}/scans/${jobId}`, {
    headers: authHeaders(),
  });
  return handleResponse(res);
}

// Exposures
export async function listExposures(): Promise<{ exposures: Exposure[] }> {
  const res = await fetch(`${BASE}/exposures`, { headers: authHeaders() });
  return handleResponse(res);
}

export async function getExposureSummary(): Promise<{
  summary: ExposureSummary;
}> {
  const res = await fetch(`${BASE}/exposures/summary`, {
    headers: authHeaders(),
  });
  return handleResponse(res);
}
