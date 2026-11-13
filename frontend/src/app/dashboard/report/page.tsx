"use client";

import { useEffect, useState } from "react";
import Link from "next/link";
import { ArrowLeft, Printer } from "lucide-react";
import { listExposures, listRemovalRequests, listScans } from "@/lib/api";
import { formatDate, piiCategoryLabel } from "@/lib/utils";
import {
  OUTCOME_LABEL,
  OUTCOME_MEANING,
  OUTCOME_ORDER,
  latestRequest,
  reportOutcome,
  type ReportOutcome,
} from "@/lib/report";
import type { Exposure, RemovalRequest, ScanJob } from "@/types";

/**
 * A plain, printable account of where each listing stands. It carries no listing links, names or
 * identity details, so it is safe to print or share. It reports only what we know: submitted is
 * not removed, and a page we could not read is "inconclusive", not "gone".
 */
export default function ReportPage() {
  const [exposures, setExposures] = useState<Exposure[]>([]);
  const [requests, setRequests] = useState<RemovalRequest[]>([]);
  const [scan, setScan] = useState<ScanJob | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");

  useEffect(() => {
    (async () => {
      try {
        const [e, r, s] = await Promise.all([listExposures(), listRemovalRequests(), listScans()]);
        setExposures(e.exposures);
        setRequests(r.requests);
        setScan(s.jobs.find((j) => j.status === "completed" && j.coverage) ?? null);
      } catch (err) {
        setError(err instanceof Error ? err.message : "Could not load your report.");
      } finally {
        setLoading(false);
      }
    })();
  }, []);

  const rows = exposures.map((e) => {
    const req = latestRequest(e.id, requests);
    return { e, req, outcome: reportOutcome(e, req) };
  });
  const counts = OUTCOME_ORDER.map((o) => ({ o, n: rows.filter((r) => r.outcome === o).length })).filter((c) => c.n > 0);
  const unchecked = scan?.coverage?.filter((c) => c.outcome !== "checked") ?? [];
  const checked = (scan?.coverage?.length ?? 0) - unchecked.length;

  return (
    <div className="min-h-screen bg-gray-950 text-gray-100 print:bg-white print:text-black">
      <main className="max-w-4xl mx-auto px-6 py-10">
        <div className="flex items-center justify-between mb-8 print:hidden">
          <Link href="/dashboard" className="inline-flex items-center gap-2 text-gray-400 hover:text-white text-sm">
            <ArrowLeft className="w-4 h-4" aria-hidden="true" /> Back to dashboard
          </Link>
          <button
            onClick={() => window.print()}
            className="inline-flex items-center gap-2 border border-gray-700 hover:border-gray-500 text-gray-200 text-sm px-3 py-1.5 rounded-lg"
          >
            <Printer className="w-4 h-4" aria-hidden="true" /> Print or save as PDF
          </button>
        </div>

        <h1 className="text-3xl font-bold text-white print:text-black mb-1">Your privacy report</h1>
        <p className="text-gray-400 print:text-gray-700 text-sm mb-8">
          VectorScrub · prepared {formatDate(new Date().toISOString())}
        </p>

        {loading && <p className="text-gray-400">Loading…</p>}
        {error && <p role="alert" className="text-red-400">{error}</p>}

        {!loading && !error && (
          <>
            <section className="mb-8">
              <h2 className="text-lg font-semibold mb-3">Summary</h2>
              {rows.length === 0 ? (
                <p className="text-gray-300 print:text-gray-800 text-sm">
                  We have not found any listings for you yet. Run a scan from your dashboard.
                </p>
              ) : (
                <ul className="text-sm space-y-1" data-testid="report-summary">
                  <li>
                    <strong>{rows.length}</strong> {rows.length === 1 ? "listing" : "listings"} found.
                  </li>
                  {counts.map(({ o, n }) => (
                    <li key={o}>
                      <strong>{n}</strong> · {OUTCOME_LABEL[o]}
                    </li>
                  ))}
                </ul>
              )}
            </section>

            {rows.length > 0 && (
              <section className="mb-8">
                <h2 className="text-lg font-semibold mb-3">Each listing</h2>
                <table className="w-full text-sm border-collapse" data-testid="report-table">
                  <thead>
                    <tr className="text-left border-b border-gray-700 print:border-gray-400">
                      <th className="py-2 pr-3">Site</th>
                      <th className="py-2 pr-3">Found</th>
                      <th className="py-2 pr-3">Where it stands</th>
                      <th className="py-2">Key dates</th>
                    </tr>
                  </thead>
                  <tbody>
                    {rows.map(({ e, req, outcome }) => (
                      <tr key={e.id} className="border-b border-gray-800 print:border-gray-300 align-top">
                        <td className="py-2 pr-3">
                          {e.source_name}
                          <div className="text-xs text-gray-500">{piiCategoryLabel(e.pii_category)}</div>
                        </td>
                        <td className="py-2 pr-3">{formatDate(e.discovered_at)}</td>
                        <td className="py-2 pr-3">
                          <strong>{OUTCOME_LABEL[outcome as ReportOutcome]}</strong>
                          <div className="text-xs text-gray-400 print:text-gray-700">{OUTCOME_MEANING[outcome]}</div>
                          {req?.procedure_used === "manual" && req.manual_reported_at && (
                            <div className="text-xs text-gray-400 print:text-gray-700">
                              You reported completing the opt-out yourself. This is your report; it is not verified.
                            </div>
                          )}
                        </td>
                        <td className="py-2 text-xs text-gray-300 print:text-gray-800">
                          {req?.customer_authorized_at && <div>Authorized {formatDate(req.customer_authorized_at)}</div>}
                          {req?.submitted_at && <div>Submitted {formatDate(req.submitted_at)}</div>}
                          {req?.last_check_at && <div>Last check {formatDate(req.last_check_at)}</div>}
                          {req?.verified_at && <div>Verified removed {formatDate(req.verified_at)}</div>}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </section>
            )}

            <section className="mb-8">
              <h2 className="text-lg font-semibold mb-3">What we could not check</h2>
              {scan?.coverage ? (
                <p className="text-sm text-gray-300 print:text-gray-800">
                  Our last scan read {checked} of {scan.coverage.length} sources.
                  {unchecked.length > 0 && (
                    <>
                      {" "}
                      These could not be read, so finding nothing there does not mean you are not listed:{" "}
                      {unchecked.map((c) => c.source).join(", ")}.
                    </>
                  )}
                </p>
              ) : (
                <p className="text-sm text-gray-300 print:text-gray-800">No completed scan with coverage details yet.</p>
              )}
            </section>

            <section className="text-xs text-gray-400 print:text-gray-700 border-t border-gray-800 print:border-gray-400 pt-4">
              <p className="mb-2">
                This report shows what we know. A request being submitted does not mean a listing was
                removed; only our own re-check can say that, and when we cannot read a page we report it as
                inconclusive. Sites can take weeks, and listings can return. This report is not legal advice.
              </p>
              <p>
                It contains no listing links or identity details. Questions or corrections: see our{" "}
                <Link href="/terms" className="underline">Terms</Link> for how to reach us.
              </p>
            </section>
          </>
        )}
      </main>
    </div>
  );
}
