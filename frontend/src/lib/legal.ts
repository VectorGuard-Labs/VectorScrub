/**
 * Version of the Terms of Service and Privacy Policy that registration records. Must equal
 * TERMS_VERSION in backend/src/models/customer.rs: the API refuses a registration that does not
 * send the current value. Change both when either document changes in a way customers must
 * agree to again.
 */
export const TERMS_VERSION = "2026-11-pilot-2";

export const LEGAL_UPDATED = "November 10, 2026";

/** Where customers write for support and data requests (set CONTACT_EMAIL on the web server). */
export function contactEmail(): string | null {
  const v = process.env.CONTACT_EMAIL?.trim();
  return v && /^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(v) ? v : null;
}
