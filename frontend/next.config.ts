import path from "path";
import type { NextConfig } from "next";

const nextConfig: NextConfig = {
  // Prevent Next.js from walking up to a parent lockfile as the workspace root
  outputFileTracingRoot: path.join(__dirname, "../../"),
  async rewrites() {
    return [
      {
        source: "/api/v1/:path*",
        destination: `${process.env.NEXT_PUBLIC_API_URL ?? "http://localhost:8080"}/api/v1/:path*`,
      },
    ];
  },
};

export default nextConfig;
