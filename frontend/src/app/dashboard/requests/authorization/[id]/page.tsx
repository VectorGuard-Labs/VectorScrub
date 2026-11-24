"use client";

import { useEffect, useState } from "react";
import Link from "next/link";
import { useParams } from "next/navigation";
import { Download } from "lucide-react";
import { getDeletionAuthorizationRecord } from "@/lib/api";
import { formatDate } from "@/lib/utils";
import type { DeletionAuthorizationRecord, DeletionStatus } from "@/types";

const STATUS_WORDS: Record<DeletionStatus, string> = {
  queued: "waiting to be sent",
  sending: "being sent",
  sent: "sent",
  failed: "not sent",
  replied: "sent; the company replied",
  removed: "sent; you reported the company deleted your data",
  denied: "sent; you reported the company declined",
  withdrawn: "withdrawn, never sent",
};

/**
 * The signed authorization of one campaign, laid out to print. "Download PDF" opens the browser's
 * print dialog, where "Save as PDF" writes the file: the page builds no PDF itself and loads nothing
 * from anywhere else. The document text is shown exactly as signed, so its SHA-256 can be checked.
 */
export default function SignedAuthorizationPage() {
  const params = useParams<{ id: string }>();
  const [record, setRecord] = useState<DeletionAuthorizationRecord | null>(null);
  const [error, setError] = useState("");

  useEffect(() => {
    let live = true;
    getDeletionAuthorizationRecord(params.id)
      .then((r) => live && setRecord(r))
      .catch(() => live && setError("Could not load this authorization. It may not exist, or the feature is switched off."));
    return () => {
      live = false;
    };
  }, [params.id]);

  return (
    <main className="flex-1 max-w-3xl mx-auto w-full px-6 py-10 print:px-0 print:py-0 print:text-black">
      <div className="flex flex-wrap items-center justify-between gap-3 mb-6 print:hidden">
        <Link href="/dashboard/requests" className="text-gray-400 hover:text-white text-sm">
          ← Deletion requests
        </Link>
        {record?.signed_document && (
          <button
            onClick={() => window.print()}
            className="inline-flex items-center gap-2 bg-brand-blue-600 hover:bg-brand-blue-500 text-white px-4 py-2 rounded-lg text-sm font-medium"
          >
            <Download className="w-4 h-4" aria-hidden="true" />
            Download PDF
          </button>
        )}
      </div>

      {error && <p role="alert" className="text-red-300 text-sm">{error}</p>}
      {!record && !error && <p className="text-gray-400 text-sm animate-pulse">Loading…</p>}

      {record && (
        <>
          <p className="text-gray-400 text-sm mb-4 print:hidden">
            {record.signed_document
              ? 'This is the authorization you signed. To keep it as a PDF, press "Download PDF" and choose "Save as PDF" in the print window. Give it to a company that asks for proof of your signed permission.'
              : "This request was made before signing was added, so there is no signed document. The text you accepted is below."}
          </p>

          <pre
            className="bg-gray-900 border border-gray-800 rounded-lg p-5 text-sm text-gray-200 whitespace-pre-wrap break-words font-mono print:bg-white print:text-black print:border-0 print:p-0"
            data-testid="signed-document"
          >
            {record.signed_document ?? record.authorization_text}
          </pre>

          <dl className="mt-6 text-xs text-gray-400 space-y-1 print:text-black">
            {record.signed_document_sha256 && (
              <div>
                <dt className="inline font-medium">Document fingerprint (SHA-256 of the text above): </dt>
                <dd className="inline font-mono break-all">{record.signed_document_sha256}</dd>
              </div>
            )}
            <div>
              <dt className="inline font-medium">Authorization version: </dt>
              <dd className="inline">{record.authorization_version}</dd>
            </div>
            {record.withdrawn_at && (
              <div>
                <dt className="inline font-medium">Withdrawn: </dt>
                <dd className="inline">{formatDate(record.withdrawn_at)}. Nothing that was still waiting was sent after this.</dd>
              </div>
            )}
          </dl>

          <section className="mt-6 print:mt-4" aria-labelledby="status-heading">
            <h2 id="status-heading" className="text-white text-sm font-semibold mb-2 print:text-black">What happened to each request</h2>
            <ul className="text-xs text-gray-400 space-y-1 print:text-black">
              {record.companies.map((c) => (
                <li key={c.request_id}>
                  {c.company}
                  {c.to_address ? ` (${c.to_address})` : ""}: {STATUS_WORDS[c.status]}
                  {c.sent_at ? `, sent ${formatDate(c.sent_at)}` : ""}
                  {c.withdrawn_at ? `, withdrawn ${formatDate(c.withdrawn_at)}` : ""}
                </li>
              ))}
            </ul>
          </section>
        </>
      )}
    </main>
  );
}
