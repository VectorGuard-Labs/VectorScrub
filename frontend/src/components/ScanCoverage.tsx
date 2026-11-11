import Link from "next/link";
import { AlertTriangle } from "lucide-react";
import type { ScanJob, SourceOutcome } from "@/types";

const GROUPS: { outcome: SourceOutcome; label: string; hint: string }[] = [
  { outcome: "checked", label: "Checked", hint: "We read the page." },
  { outcome: "blocked", label: "Blocked automated access", hint: "The site refused our request." },
  { outcome: "not_found", label: "Search address no longer works", hint: "The site returned “not found”." },
  { outcome: "unavailable", label: "Unavailable", hint: "Down, rate limited, or unreachable." },
  { outcome: "unreadable", label: "No readable results", hint: "The page loaded but showed nothing we could read." },
  { outcome: "not_supported", label: "Needs a manual search", hint: "These sites only show results through an interactive search, so they cannot be checked automatically." },
];

/** "Checked 3 of 19 sources." for a scan that recorded coverage, otherwise "". */
export function coverageSummary(job: ScanJob): string {
  const total = job.coverage?.length ?? 0;
  if (!job.coverage || total === 0) return "";
  const checked = job.coverage.filter((c) => c.outcome === "checked").length;
  return `Checked ${checked} of ${total} sources.`;
}

/**
 * Which sources the last scan could and could not read. A scan that reads 3 of 19 sites and
 * finds nothing on the others is not the same as one that read all 19, and the dashboard must
 * not suggest it is.
 */
export default function ScanCoverage({ job }: { job: ScanJob }) {
  const coverage = job.coverage;
  if (!coverage || coverage.length === 0) return null;

  const checked = coverage.filter((c) => c.outcome === "checked").length;
  const unchecked = coverage.length - checked;

  return (
    <section
      className="bg-gray-900 border border-gray-800 rounded-xl p-5 mb-8"
      aria-labelledby="coverage-heading"
    >
      <h2 id="coverage-heading" className="text-white font-semibold text-sm mb-1">
        Source coverage (last scan)
      </h2>
      <p className="text-gray-400 text-sm">
        We could read <span className="text-white">{checked}</span> of{" "}
        <span className="text-white">{coverage.length}</span> sources.
      </p>

      {unchecked > 0 && (
        <p
          role="note"
          className="flex items-start gap-2 text-orange-300 text-sm mt-3 bg-orange-950/40 border border-orange-900 rounded-lg px-3 py-2"
        >
          <AlertTriangle className="w-4 h-4 mt-0.5 flex-shrink-0" aria-hidden="true" />
          <span>
            {unchecked} {unchecked === 1 ? "source" : "sources"} could not be checked. Finding nothing there
            does not mean you are not listed. You can search those sites yourself:{" "}
            <Link href="/dashboard/optouts" className="underline hover:text-orange-200">
              see the manual opt-out guide
            </Link>
            .
          </span>
        </p>
      )}

      <details className="mt-3">
        <summary className="text-teal-400 hover:text-teal-300 text-sm cursor-pointer select-none">
          Show every source
        </summary>
        <div className="mt-3 space-y-3">
          {GROUPS.map(({ outcome, label, hint }) => {
            const names = coverage.filter((c) => c.outcome === outcome).map((c) => c.source);
            if (names.length === 0) return null;
            return (
              <div key={outcome}>
                <div className="text-gray-300 text-sm font-medium">
                  {label} <span className="text-gray-500 font-normal">({names.length})</span>
                </div>
                <div className="text-gray-500 text-xs mb-1">{hint}</div>
                <div className="text-gray-400 text-sm">{names.join(", ")}</div>
              </div>
            );
          })}
        </div>
      </details>
    </section>
  );
}
