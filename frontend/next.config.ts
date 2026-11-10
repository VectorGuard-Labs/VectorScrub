import path from "path";
import type { NextConfig } from "next";

/**
 * Where the Rust API is, as seen from the Next.js server (not from the browser: the browser
 * only ever talks to this app, which proxies /api/v1/* to the backend).
 *
 * This is read when `next build` runs, because Next.js bakes rewrites into the build. In
 * Docker it is therefore a build argument (see infra/docker/Dockerfile.frontend), not a
 * runtime variable. It is deliberately not NEXT_PUBLIC_*: it is a server-side address and
 * must not be shipped to browsers.
 */
const API_URL = (process.env.API_URL ?? "http://localhost:8080").replace(/\/+$/, "");

/**
 * Headers that are the same for every response, including static assets.
 * The Content-Security-Policy is not here: it carries a per-request nonce and is set in
 * src/middleware.ts.
 */
const securityHeaders = [
  { key: "X-Content-Type-Options", value: "nosniff" },
  { key: "X-Frame-Options", value: "DENY" },
  { key: "Referrer-Policy", value: "strict-origin-when-cross-origin" },
  { key: "Cross-Origin-Opener-Policy", value: "same-origin" },
  {
    key: "Permissions-Policy",
    value:
      "geolocation=(), microphone=(), camera=(), payment=(), usb=(), " +
      "magnetometer=(), gyroscope=(), accelerometer=()",
  },
  // Browsers ignore HSTS on plain http, so this is harmless in local production builds.
  ...(process.env.NODE_ENV === "production"
    ? [{ key: "Strict-Transport-Security", value: "max-age=31536000; includeSubDomains" }]
    : []),
];

const nextConfig: NextConfig = {
  // A self-contained server in .next/standalone, which is what the Docker image runs.
  output: "standalone",
  // Trace from this directory, not a parent: stops Next walking up to an unrelated lockfile
  // and keeps server.js at the top of .next/standalone.
  outputFileTracingRoot: path.join(__dirname),
  poweredByHeader: false,
  async headers() {
    return [{ source: "/:path*", headers: securityHeaders }];
  },
  async rewrites() {
    return [
      {
        source: "/api/v1/:path*",
        destination: `${API_URL}/api/v1/:path*`,
      },
    ];
  },
};

export default nextConfig;
