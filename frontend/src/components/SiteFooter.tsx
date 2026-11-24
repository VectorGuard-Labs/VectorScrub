import Link from "next/link";
import { postalAddress } from "@/lib/legal";

/**
 * Shown at the bottom of every page, signed in or not, so the Terms and the Privacy Policy are one
 * click away wherever the customer is. Rendered once, by the root layout. Plain links: it loads nothing
 * from anywhere else. Hidden when printing (the privacy report is printed).
 */
export default function SiteFooter() {
  const address = postalAddress();
  return (
    <footer className="px-6 py-6 border-t border-gray-800 text-center print:hidden" data-testid="site-footer">
      <nav aria-label="Legal" className="text-gray-500 text-xs">
        <Link href="/terms" className="hover:text-gray-300 underline-offset-2 hover:underline">Terms of Service</Link>
        <span aria-hidden="true">&nbsp;·&nbsp;</span>
        <Link href="/privacy" className="hover:text-gray-300 underline-offset-2 hover:underline">Privacy Policy</Link>
      </nav>
      <p className="text-gray-600 text-xs mt-2">© 2026 VectorGuard Labs. All rights reserved.</p>
      {address && <p className="text-gray-600 text-xs mt-1">{address}</p>}
    </footer>
  );
}
