"use client";

import { useEffect, useState, useRef } from "react";
import { useRouter } from "next/navigation";
import {
  Shield,
  Search,
  AlertTriangle,
  CheckCircle,
  LogOut,
  RefreshCw,
  ExternalLink,
} from "lucide-react";
import {
  getProfile,
  getExposureSummary,
  listExposures,
  initiateSccan,
  authorizeRemoval,
  listScans,
  logout,
} from "@/lib/api";
import {
  privacyScoreLabel,
  severityColor,
  formatDate,
  formatDateTime,
  piiCategoryLabel,
} from "@/lib/utils";
import type { CustomerProfile, ExposureSummary, Exposure, ScanJob } from "@/types";
import PrivacyScoreChart from "@/components/PrivacyScoreChart";
import ScanCoverage, { coverageSummary } from "@/components/ScanCoverage";

export default function DashboardPage() {
  const router = useRouter();
  const [profile, setProfile] = useState<CustomerProfile | null>(null);
  const [summary, setSummary] = useState<ExposureSummary | null>(null);
  const [exposures, setExposures] = useState<Exposure[]>([]);
  const [loading, setLoading] = useState(true);
  const [scanLoading, setScanLoading] = useState(false);
  const [scanMessage, setScanMessage] = useState("");
  // The most recent scan that finished, for the source-coverage panel.
  const [latestFinished, setLatestFinished] = useState<ScanJob | null>(null);
  const [error, setError] = useState("");
  // Display only (spinner). Logic uses the ref below, never this state.
  const [isPolling, setIsPolling] = useState(false);
  // The interval handle doubles as "are we polling". It is a ref, not state, because the
  // interval callback would otherwise keep reading the value from the render that created it.
  const pollingIntervalRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const mountedRef = useRef(true);

  useEffect(() => {
    mountedRef.current = true;

    // No client-side token check: the session is an HttpOnly cookie the page cannot see.
    // If it is missing or expired the first API call answers 401 and the API client sends
    // the user to /login.
    loadData();

    // If a scan is already queued or running (page opened or refreshed mid-scan), resume polling.
    void checkScanStatus();

    // Cleanup polling interval on unmount
    return () => {
      mountedRef.current = false;
      if (pollingIntervalRef.current) {
        clearInterval(pollingIntervalRef.current);
        pollingIntervalRef.current = null;
      }
    };
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  async function loadData() {
    setLoading(true);
    try {
      const [p, s, e] = await Promise.all([
        getProfile(),
        getExposureSummary(),
        listExposures(),
      ]);
      setProfile(p);
      setSummary(s.summary);
      setExposures(e.exposures);
    } catch {
      setError("Failed to load dashboard. Please try signing in again.");
    } finally {
      setLoading(false);
    }
  }

  async function checkScanStatus() {
    try {
      const { jobs } = await listScans();

      // Check if there's a running scan
      const runningScan = jobs.find(
        (job) => job.status === "queued" || job.status === "running"
      );

      if (!mountedRef.current) return;

      const newestFirst = [...jobs].sort(
        (a, b) => new Date(b.queued_at).getTime() - new Date(a.queued_at).getTime()
      );
      setLatestFinished(
        newestFirst.find((j) => j.status === "completed" || j.status === "failed") ?? null
      );

      if (runningScan) {
        // Scan is still running: show it and make sure we keep polling.
        setScanMessage("Scan job queued. Your AI agents are working.");
        ensurePolling();
      } else if (pollingIntervalRef.current) {
        // We were polling and nothing is running any more: the scan just finished.
        // Refresh the data, stop polling, and report the most recent scan's outcome.
        stopPolling();
        await loadData();

        const latest = newestFirst[0];

        if (latest?.status === "completed") {
          const n = latest.exposures_found;
          setScanMessage(
            `Scan completed. Found ${n} ${n === 1 ? "exposure" : "exposures"}. ${coverageSummary(latest)}`.trim()
          );
        } else if (latest?.status === "failed") {
          setScanMessage("Scan failed. Please try again.");
        } else {
          setScanMessage("");
        }
      }
    } catch (err) {
      console.error("Error checking scan status:", err);
    }
  }

  function ensurePolling() {
    // Idempotent: one interval at a time, checked every 3 seconds.
    if (pollingIntervalRef.current) return;
    setIsPolling(true);
    pollingIntervalRef.current = setInterval(() => {
      void checkScanStatus();
    }, 3000);
  }

  function startPolling() {
    ensurePolling();
    // Check immediately rather than waiting for the first tick.
    void checkScanStatus();
  }

  function stopPolling() {
    if (pollingIntervalRef.current) {
      clearInterval(pollingIntervalRef.current);
      pollingIntervalRef.current = null;
    }
    setIsPolling(false);
  }

  async function handleScan() {
    setScanLoading(true);
    setScanMessage("");
    try {
      const result = await initiateSccan();
      setScanMessage(result.message);

      // Start polling for scan status updates
      startPolling();
    } catch (err: unknown) {
      setScanMessage(err instanceof Error ? err.message : "Scan failed");
    } finally {
      setScanLoading(false);
    }
  }

  function handleSignOut() {
    void logout().finally(() => router.push("/"));
  }

  async function handleAuthorizeRemoval(exposureId: string) {
    if (!confirm("Authorize removal of this exposure?")) return;

    try {
      const result = await authorizeRemoval(exposureId);
      setScanMessage(result.message);
      await loadData();
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : "Authorization failed");
    }
  }

  const scoreInfo = summary
    ? privacyScoreLabel(summary.privacy_score)
    : { label: "—", color: "text-gray-500" };

  // How much of the source list the last scan could actually read; the score only reflects
  // those sources.
  const scoreBasis = (() => {
    const cov = latestFinished?.coverage;
    if (!cov || cov.length === 0) return null;
    const checked = cov.filter((c) => c.outcome === "checked").length;
    return { checked, total: cov.length, partial: checked < cov.length };
  })();

  if (loading) {
    return (
      <div className="min-h-screen flex items-center justify-center">
        <div className="text-gray-400 text-sm animate-pulse">
          Loading your privacy dashboard…
        </div>
      </div>
    );
  }

  return (
    <div className="min-h-screen">
      {/* Top nav */}
      <nav className="border-b border-gray-800 px-6 py-4 flex items-center justify-between">
        <div className="flex items-center gap-2">
          <Shield className="text-teal-400 w-5 h-5" aria-hidden="true" />
          <span className="text-white font-bold">VectorScrub</span>
        </div>
        <div className="flex items-center gap-4">
          <button
            onClick={() => router.push("/dashboard/profile")}
            className="text-gray-400 hover:text-white text-sm font-medium transition-colors"
          >
            Identity
          </button>
          <button
            onClick={() => router.push("/dashboard/removals")}
            className="text-gray-400 hover:text-white text-sm font-medium transition-colors"
          >
            Removals
          </button>
          <button
            onClick={() => router.push("/dashboard/history")}
            className="text-gray-400 hover:text-white text-sm font-medium transition-colors"
          >
            History
          </button>
          <button
            onClick={() => router.push("/dashboard/optouts")}
            className="text-gray-400 hover:text-white text-sm font-medium transition-colors"
          >
            Opt-out guide
          </button>
          <button
            onClick={() => router.push("/dashboard/billing")}
            className="text-gray-400 hover:text-white text-sm font-medium transition-colors"
          >
            Billing
          </button>
          <span className="text-gray-500 text-sm hidden sm:block">
            {profile?.email}
          </span>
          <button
            onClick={handleSignOut}
            className="flex items-center gap-1.5 text-gray-400 hover:text-white text-sm transition-colors"
            aria-label="Sign out"
          >
            <LogOut className="w-4 h-4" />
            Sign out
          </button>
        </div>
      </nav>

      <main className="max-w-6xl mx-auto px-6 py-10">
        {error && (
          <div
            role="alert"
            className="bg-red-950 border border-red-800 text-red-300 rounded-lg px-4 py-3 mb-6 text-sm"
          >
            {error}
          </div>
        )}

        {/* Header row */}
        <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-4 mb-10">
          <div>
            <h1 className="text-2xl font-bold text-white mb-1">
              Privacy Dashboard
            </h1>
            <p className="text-gray-400 text-sm">
              {summary?.last_scan_at
                ? `Last scan: ${formatDateTime(summary.last_scan_at)}`
                : "No scans yet — run your first scan below."}
            </p>
          </div>
          <button
            onClick={handleScan}
            disabled={scanLoading}
            className="flex items-center gap-2 bg-teal-600 hover:bg-teal-500 disabled:opacity-50 text-white px-5 py-2.5 rounded-lg text-sm font-medium transition-colors"
          >
            <RefreshCw
              className={`w-4 h-4 ${scanLoading ? "animate-spin" : ""}`}
              aria-hidden="true"
            />
            {scanLoading ? "Scanning…" : "Run Scan"}
          </button>
        </div>

        {scanMessage && (
          <div className="bg-teal-950 border border-teal-800 text-teal-300 rounded-lg px-4 py-3 mb-6 text-sm flex items-center gap-2">
            {isPolling && (
              <RefreshCw className="w-4 h-4 animate-spin flex-shrink-0" aria-hidden="true" />
            )}
            <span>{scanMessage}</span>
          </div>
        )}

        {latestFinished && <ScanCoverage job={latestFinished} />}

        {/* Summary cards */}
        {summary && (
          <div className="grid grid-cols-2 sm:grid-cols-4 gap-4 mb-10">
            <div className="bg-gray-900 border border-gray-800 rounded-xl p-5">
              <div className={`text-3xl font-bold mb-1 ${scoreInfo.color}`}>
                {Math.round(summary.privacy_score)}
              </div>
              <div className="text-gray-400 text-sm">Privacy Score</div>
              <div className={`text-xs mt-1 ${scoreInfo.color}`}>
                {scoreInfo.label}
              </div>
              {scoreBasis && (
                <div
                  className={`text-xs mt-1 ${scoreBasis.partial ? "text-orange-300" : "text-gray-500"}`}
                  title={
                    scoreBasis.partial
                      ? "Some sites could not be checked, so your real exposure may be higher than this score shows."
                      : "Every source was checked."
                  }
                >
                  Based on {scoreBasis.checked} of {scoreBasis.total} sources
                </div>
              )}
            </div>
            <div className="bg-gray-900 border border-gray-800 rounded-xl p-5">
              <div className="text-3xl font-bold text-white mb-1">
                {summary.active_exposures}
              </div>
              <div className="text-gray-400 text-sm">Active Exposures</div>
            </div>
            <div className="bg-gray-900 border border-gray-800 rounded-xl p-5">
              <div className="text-3xl font-bold text-green-400 mb-1">
                {summary.removed_exposures}
              </div>
              <div className="text-gray-400 text-sm">Removed</div>
            </div>
            <div className="bg-gray-900 border border-gray-800 rounded-xl p-5">
              <div className="text-3xl font-bold text-red-400 mb-1">
                {summary.critical_count + summary.high_count}
              </div>
              <div className="text-gray-400 text-sm">High/Critical</div>
            </div>
          </div>
        )}

        {/* Privacy Score Trend Charts */}
        <div className="mb-10">
          <PrivacyScoreChart />
        </div>

        {/* Exposures list */}
        <div>
          <h2 className="text-lg font-semibold text-white mb-4">
            Discovered Exposures
          </h2>
          {exposures.length === 0 ? (
            <div className="bg-gray-900 border border-gray-800 rounded-xl p-12 text-center">
              <Search
                className="w-10 h-10 text-gray-600 mx-auto mb-3"
                aria-hidden="true"
              />
              <p className="text-gray-400 mb-1">No exposures found yet.</p>
              <p className="text-gray-600 text-sm">
                Run a scan to discover where your information is exposed.
              </p>
            </div>
          ) : (
            <div className="space-y-3">
              {exposures.map((exp) => (
                <div
                  key={exp.id}
                  className="bg-gray-900 border border-gray-800 rounded-xl p-5 flex flex-col sm:flex-row sm:items-center gap-4"
                >
                  <div className="flex-1 min-w-0">
                    <div className="flex items-center gap-2 mb-1 flex-wrap">
                      <span className="text-white font-medium text-sm">
                        {exp.source_name}
                      </span>
                      <span
                        className={`text-xs px-2 py-0.5 rounded-full border font-medium ${severityColor(
                          exp.severity
                        )}`}
                      >
                        {exp.severity}
                      </span>
                      <span className="text-xs bg-gray-800 text-gray-400 px-2 py-0.5 rounded-full">
                        {piiCategoryLabel(exp.pii_category)}
                      </span>
                    </div>
                    <p className="text-gray-400 text-sm truncate">
                      {exp.pii_summary}
                    </p>
                    <p className="text-gray-600 text-xs mt-1">
                      Discovered {formatDate(exp.discovered_at)} · Confidence{" "}
                      {Math.round(exp.confidence * 100)}%
                    </p>
                  </div>
                  <div className="flex items-center gap-3 flex-shrink-0">
                    {exp.is_active ? (
                      <>
                        <span className="flex items-center gap-1 text-orange-400 text-xs">
                          <AlertTriangle className="w-3.5 h-3.5" aria-hidden="true" />
                          Active
                        </span>
                        <button
                          onClick={() => handleAuthorizeRemoval(exp.id)}
                          className="text-teal-400 hover:text-teal-300 text-xs font-medium transition-colors"
                        >
                          Remove
                        </button>
                      </>
                    ) : (
                      <span className="flex items-center gap-1 text-green-400 text-xs">
                        <CheckCircle className="w-3.5 h-3.5" aria-hidden="true" />
                        Removed
                      </span>
                    )}
                    <a
                      href={exp.source_url}
                      target="_blank"
                      rel="noopener noreferrer"
                      className="text-gray-500 hover:text-teal-400 transition-colors"
                      aria-label={`View ${exp.source_name} listing`}
                    >
                      <ExternalLink className="w-4 h-4" />
                    </a>
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>
      </main>
    </div>
  );
}
