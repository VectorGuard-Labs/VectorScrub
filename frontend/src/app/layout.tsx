import type { Metadata } from "next";
import "./globals.css";
import ErrorBoundary from "@/components/ErrorBoundary";
import SiteFooter from "@/components/SiteFooter";

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
      <body className="min-h-screen flex flex-col bg-gray-950 text-gray-100 antialiased">
        {/* The page grows to fill the screen, so the footer sits at the bottom of short pages too. */}
        <div className="flex-1 flex flex-col">
          <ErrorBoundary>{children}</ErrorBoundary>
        </div>
        <SiteFooter />
      </body>
    </html>
  );
}
