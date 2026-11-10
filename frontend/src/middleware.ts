import { NextResponse, type NextRequest } from "next/server";
import { buildCsp, newNonce } from "@/lib/csp";

/**
 * Sets a Content-Security-Policy with a fresh nonce on every page response.
 *
 * Next.js reads the nonce from the CSP header on the *request* and stamps it onto the inline
 * scripts it renders, so the same policy is put on both the forwarded request and the
 * response. Pages must therefore be rendered per request (see `dynamic` in app/layout.tsx).
 *
 * The backend sets its own security headers on /api/v1 responses, so the API proxy and
 * static assets are skipped here.
 */
export function middleware(request: NextRequest) {
  const csp = buildCsp(newNonce(), process.env.NODE_ENV === "development");
  // The nonce is the one embedded in `csp`; Next extracts it from this request header.
  const requestHeaders = new Headers(request.headers);
  requestHeaders.set("Content-Security-Policy", csp);

  const response = NextResponse.next({ request: { headers: requestHeaders } });
  response.headers.set("Content-Security-Policy", csp);
  return response;
}

export const config = {
  matcher: [
    {
      source: "/((?!api/|_next/static|_next/image|favicon.ico|robots.txt).*)",
      // Prefetches are not rendered pages and carry no scripts.
      missing: [
        { type: "header", key: "next-router-prefetch" },
        { type: "header", key: "purpose", value: "prefetch" },
      ],
    },
  ],
};
