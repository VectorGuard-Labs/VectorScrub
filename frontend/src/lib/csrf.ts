/**
 * CSRF protection, client side (double-submit cookie).
 *
 * The backend issues a random token at GET /api/v1/csrf-token, stores the same
 * value in the `csrf_token` cookie, and rejects any POST/PUT/DELETE whose
 * `X-CSRF-Token` header does not equal that cookie. This module reads the token
 * and attaches the header; use `csrfFetch` instead of `fetch` for every request
 * to the API other than login/register (which the server exempts).
 */
const BASE = "/api/v1";
const COOKIE_NAME = "csrf_token";
const HEADER_NAME = "X-CSRF-Token";
const SAFE_METHODS = new Set(["GET", "HEAD", "OPTIONS"]);

function readCookie(name: string): string | null {
  if (typeof document === "undefined") return null;
  for (const pair of document.cookie.split(";")) {
    // Split on the FIRST "=" only; a cookie value may itself contain "=".
    const idx = pair.indexOf("=");
    if (idx === -1) continue;
    if (pair.slice(0, idx).trim() === name) return pair.slice(idx + 1).trim();
  }
  return null;
}

async function issueToken(): Promise<string> {
  const res = await fetch(`${BASE}/csrf-token`, { credentials: "same-origin" });
  if (!res.ok) throw new Error("Could not obtain a CSRF token");
  const body = (await res.json()) as { csrf_token?: string };
  if (!body.csrf_token) throw new Error("CSRF token missing from response");
  return body.csrf_token;
}

/**
 * The token the server will accept right now: the current cookie value, or a
 * freshly issued one (which also sets the cookie) if there is none.
 */
export async function getCsrfToken(forceRefresh = false): Promise<string> {
  if (!forceRefresh) {
    const existing = readCookie(COOKIE_NAME);
    if (existing) return existing;
  }
  return issueToken();
}

/**
 * fetch() that adds the CSRF header to state-changing requests. If the server
 * rejects the token (cookie expired, cleared, or rotated) it gets a fresh one
 * and retries once.
 */
export async function csrfFetch(
  input: string,
  init: RequestInit = {}
): Promise<Response> {
  const method = (init.method ?? "GET").toUpperCase();
  if (SAFE_METHODS.has(method)) return fetch(input, init);

  const send = async (token: string): Promise<Response> => {
    const headers = new Headers(init.headers);
    headers.set(HEADER_NAME, token);
    return fetch(input, { ...init, headers, credentials: "same-origin" });
  };

  let res = await send(await getCsrfToken());
  if (res.status === 403) {
    const body = await res.clone().text().catch(() => "");
    if (body.includes("CSRF")) {
      res = await send(await getCsrfToken(true));
    }
  }
  return res;
}
