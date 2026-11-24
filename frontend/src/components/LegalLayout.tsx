import Link from "next/link";
import { contactEmail, LEGAL_UPDATED, TERMS_VERSION } from "@/lib/legal";

export function Contact() {
  const email = contactEmail();
  return email ? (
    <a href={`mailto:${email}`} className="text-brand-blue-400 underline">
      {email}
    </a>
  ) : (
    <span className="text-amber-300">[contact address not configured yet: ask us from your account page]</span>
  );
}

export function Section({ title, id, children }: { title: string; id?: string; children: React.ReactNode }) {
  return (
    <section id={id} className="mb-8 scroll-mt-8">
      <h2 className="text-xl font-semibold text-white mb-3">{title}</h2>
      <div className="space-y-3 text-gray-300 text-sm leading-relaxed">{children}</div>
    </section>
  );
}

export default function LegalLayout({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <main className="flex-1 px-6 py-12 max-w-3xl mx-auto">
      <Link href="/" className="inline-flex items-center gap-2 mb-8 text-gray-400 hover:text-white text-sm">
        VectorScrub
      </Link>
      <h1 className="text-3xl font-bold text-white mb-2">{title}</h1>
      <p className="text-gray-500 text-xs mb-6">
        Version {TERMS_VERSION} · Updated {LEGAL_UPDATED}
      </p>
      <div className="border border-amber-700/60 bg-amber-950/30 text-amber-200 text-sm rounded-lg p-4 mb-10">
        <strong>Early pilot.</strong> VectorScrub is an early product. A lawyer has reviewed this
        document and our changes follow that advice, but it has not yet been approved as final by
        privacy counsel. It says plainly what the service does and does not do. If something here is
        unclear, ask us before you sign up.
      </div>
      {children}
    </main>
  );
}
