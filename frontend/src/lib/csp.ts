/**
 * Content-Security-Policy for the web app.
 *
 * Scripts: only the app's own files and the inline bootstraps Next.js itself emits, which are
 * allowed through a per-request nonce. There is no 'unsafe-inline' for scripts, so markup
 * injected into a page cannot run script. 'strict-dynamic' lets the nonced bootstrap load
 * the app's chunks.
 *
 * Styles keep 'unsafe-inline': React and the charting library set inline style attributes,
 * which a nonce cannot cover. Style injection is a much weaker primitive than script
 * injection, and the policy still blocks every other kind of resource.
 */
export function buildCsp(nonce: string, isDev: boolean): string {
  const directives: Record<string, string[]> = {
    "default-src": ["'self'"],
    // Next's dev server and React Refresh need eval; production does not.
    "script-src": ["'self'", `'nonce-${nonce}'`, "'strict-dynamic'", ...(isDev ? ["'unsafe-eval'"] : [])],
    "style-src": ["'self'", "'unsafe-inline'"],
    "img-src": ["'self'", "data:", "blob:"],
    "font-src": ["'self'"],
    // The browser only talks to this origin; /api/v1 is proxied to the backend server-side.
    "connect-src": ["'self'"],
    "object-src": ["'none'"],
    "base-uri": ["'self'"],
    "form-action": ["'self'"],
    "frame-ancestors": ["'none'"],
  };
  const policy = Object.entries(directives).map(([name, values]) => `${name} ${values.join(" ")}`);
  if (!isDev) policy.push("upgrade-insecure-requests");
  return policy.join("; ");
}

/** A fresh, unguessable nonce (base64) for one response. */
export function newNonce(): string {
  const bytes = new Uint8Array(16);
  crypto.getRandomValues(bytes);
  let binary = "";
  for (const b of bytes) binary += String.fromCharCode(b);
  return btoa(binary);
}
