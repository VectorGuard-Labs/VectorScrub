/**
 * Version of the Terms of Service and Privacy Policy that registration records. Must equal
 * TERMS_VERSION in backend/src/models/customer.rs: the API refuses a registration that does not
 * send the current value. Change both when either document changes in a way customers must
 * agree to again.
 */
export const TERMS_VERSION = "2026-11-pilot-4";

export const LEGAL_UPDATED = "November 19, 2026";

/** Where customers write for support and data requests (set CONTACT_EMAIL on the web server). */
export function contactEmail(): string | null {
  const v = process.env.CONTACT_EMAIL?.trim();
  return v && /^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(v) ? v : null;
}

/** Minimum age to hold an account. Registration asks for a declaration; the API refuses without it. */
export const MINIMUM_AGE = 18;

/**
 * The company's postal address (set POSTAL_ADDRESS on the web server, on one line, parts
 * separated by commas). Shown in the page footers and in the copyright notice. Any commercial
 * email we ever send must carry it too (see docs/COMPLIANCE_CHECKLIST.md). `null` until set:
 * nothing is invented in its place.
 */
export function postalAddress(): string | null {
  const v = process.env.POSTAL_ADDRESS?.trim();
  return v && v.length >= 10 && v.length <= 300 ? v : null;
}

export type DmcaAgent = { name: string; address: string; email: string; registration: string };

/**
 * The agent designated to receive copyright infringement notices under 17 U.S.C. 512(c), exactly
 * as registered with the U.S. Copyright Office. Shown only when ALL four values are set
 * (DMCA_AGENT_NAME, DMCA_AGENT_ADDRESS, DMCA_AGENT_EMAIL, DMCA_AGENT_REGISTRATION, the
 * "DMCA-1-xxxxxxx" number the Office issues). The page never claims a designated agent that has
 * not been registered, because the safe harbor depends on the registration being real and current.
 */
export function dmcaAgent(): DmcaAgent | null {
  const name = process.env.DMCA_AGENT_NAME?.trim();
  const address = process.env.DMCA_AGENT_ADDRESS?.trim();
  const email = process.env.DMCA_AGENT_EMAIL?.trim();
  const registration = process.env.DMCA_AGENT_REGISTRATION?.trim();
  if (!name || !address || !registration || !email) return null;
  if (!/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email)) return null;
  if (!/^DMCA-\d-\d{3,}$/i.test(registration)) return null;
  return { name, address, email, registration };
}
