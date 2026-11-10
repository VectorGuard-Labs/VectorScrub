import type { Metadata } from "next";
import "./globals.css";
import ErrorBoundary from "@/components/ErrorBoundary";

export const metadata: Metadata = {
  title: "VectorScrub — Your personal information. Under your control.",
  description:
    "AI-powered privacy protection that discovers, identifies, and helps remove your personal information from the digital world.",
};

// Render every page per request. The Content-Security-Policy carries a fresh nonce for each
// response (see src/middleware.ts), and a page that was prerendered at build time would have
// inline scripts the nonce cannot be added to, which the policy would then block.
export const dynamic = "force-dynamic";

export default function RootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <html lang="en">
      <body className="min-h-screen bg-gray-950 text-gray-100 antialiased">
        <ErrorBoundary>{children}</ErrorBoundary>
      </body>
    </html>
  );
}
