"use client";

import { useEffect, useState } from "react";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { Shield, LogOut, ExternalLink, Info } from "lucide-react";
import {
  ApiError,
  createDeletionCampaign,
  getDeletionInfo,
  listDeletionRequests,
  logout,
  reportDeletionOutcome,
} from "@/lib/api";
import { formatDate } from "@/lib/utils";
import type { DeletionInfo, DeletionRequest, DeletionStatus } from "@/types";

const STATUS: Record<DeletionStatus, { text: string; style: string }> = {
  queued: { text: "Waiting to be sent", style: "text-yellow-300 border-yellow-800 bg-yellow-950" },
  sending: { text: "Sending", style: "text-yellow-300 border-yellow-800 bg-yellow-950" },
  sent: { text: "Sent. Waiting for the company", style: "text-blue-300 border-blue-800 bg-blue-950" },
  failed: { text: "Not sent", style: "text-red-300 border-red-800 bg-red-950" },
  replied: { text: "The company replied", style: "text-blue-300 border-blue-800 bg-blue-950" },
  removed: { text: "Company says it deleted your data", style: "text-green-400 border-green-800 bg-green-950" },
  denied: { text: "The company declined", style: "text-orange-300 border-orange-800 bg-orange-950" },
};

export default function DeletionRequestsPage() {
  const router = useRouter();
  const [info, setInfo] = useState<DeletionInfo | null>(null);
  const [unavailable, setUnavailable] = useState(false);
  const [requests, setRequests] = useState<DeletionRequest[]>([]);
  const [loading, setLoading] = useState(true);
  const [picked, setPicked] = useState<Set<string>>(new Set());
  const [includeAddress, setIncludeAddress] = useState(false);
  const [consent, setConsent] = useState(false);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState("");
  const [error, setError] = useState("");

  async function load() {
    try {
      const [i, r] = await Promise.all([getDeletionInfo(), listDeletionRequests()]);
      setInfo(i);
      setRequests(r);
    } catch (e) {
      if (e instanceof ApiError && e.status === 404) setUnavailable(true);
      else setError("Could not load this page. Please try again.");
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    void load();
  }, []);

  function toggle(id: string) {
    setPicked((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }

  async function submit() {
    if (!info) return;
    setBusy(true);
    setError("");
    setMessage("");
    try {
      const r = await createDeletionCampaign({
        channel_ids: [...picked],
        include_address: sharingAddress,
        consent,
        authorization_version: info.authorization.version,
      });
      setMessage(r.message);
      setPicked(new Set());
      setConsent(false);
      await load();
    } catch (e) {
      setError(e instanceof Error ? e.message : "Something went wrong.");
    } finally {
      setBusy(false);
    }
  }

  async function report(id: string, outcome: "replied" | "removed" | "denied") {
    setError("");
    try {
      await reportDeletionOutcome(id, outcome);
      await load();
    } catch (e) {
      setError(e instanceof Error ? e.message : "Something went wrong.");
    }
  }

  function handleSignOut() {
    void logout().finally(() => router.push("/"));
  }

  // A company whose page asks for the address cannot be sent to without it.
  const needsAddress = !!info?.channels.some((c) => picked.has(c.id) && c.requires_address);
  const sharingAddress = includeAddress || needsAddress;
  const canSubmit = !!info?.plan_ok && picked.size > 0 && consent && !busy;

  return (
    <div className="min-h-screen">
      <nav className="border-b border-gray-800 px-6 py-4 flex items-center justify-between">
        <div className="flex items-center gap-2">
          <Shield className="text-teal-400 w-5 h-5" aria-hidden="true" />
          <span className="text-white font-bold">VectorScrub</span>
        </div>
        <div className="flex items-center gap-4">
          <button onClick={() => router.push("/dashboard")} className="text-gray-400 hover:text-white text-sm font-medium transition-colors">
            Dashboard
          </button>
          <button onClick={handleSignOut} className="flex items-center gap-1.5 text-gray-400 hover:text-white text-sm transition-colors" aria-label="Sign out">
            <LogOut className="w-4 h-4" />
            Sign out
          </button>
        </div>
      </nav>

      <main className="max-w-3xl mx-auto px-6 py-10">
        <h1 className="text-2xl font-bold text-white mb-2">Deletion requests</h1>

        {loading ? (
          <p className="text-gray-400 text-sm animate-pulse">Loading…</p>
        ) : unavailable ? (
          <p className="text-gray-400 text-sm" data-testid="requests-unavailable">
            Sending deletion requests for you is not available yet. In the meantime, the{" "}
            <Link href="/dashboard/optouts" className="text-teal-400 hover:text-teal-300 underline">
              manual opt-out guide
            </Link>{" "}
            shows how to ask each company yourself.
          </p>
        ) : !info ? (
          <p role="alert" className="text-red-300 text-sm">{error}</p>
        ) : (
          <>
            <p className="text-gray-400 text-sm mb-6">
              We can email some companies a request to delete your data, as your authorized agent. We only use an
              address the company itself publishes, and only after you approve each company here. A request is not
              a guarantee: companies can decline, and many only act for residents of certain states.
            </p>

            {!info.plan_ok && (
              <p className="bg-gray-900 border border-gray-800 rounded-lg px-4 py-3 mb-6 text-sm text-gray-300">
                Sending requests is part of the paid plan.{" "}
                <Link href="/dashboard/billing" className="text-teal-400 hover:text-teal-300 underline">See plans</Link>
              </p>
            )}

            {(error || message) && (
              <div
                role={error ? "alert" : "status"}
                className={`rounded-lg px-4 py-3 mb-6 text-sm border ${
                  error ? "bg-red-950 border-red-800 text-red-300" : "bg-teal-950 border-teal-800 text-teal-300"
                }`}
              >
                {error || message}
              </div>
            )}

            <section aria-labelledby="choose-heading" className="mb-10">
              <h2 id="choose-heading" className="text-white font-semibold mb-3">Choose companies</h2>
              <ul className="space-y-3">
                {info.channels.map((c) => (
                  <li key={c.id} className="bg-gray-900 border border-gray-800 rounded-xl p-5">
                    <label className={`flex items-start gap-3 ${c.eligible && info.plan_ok ? "cursor-pointer" : "opacity-70"}`}>
                      <input
                        type="checkbox"
                        className="mt-1"
                        checked={picked.has(c.id)}
                        disabled={!c.eligible || !info.plan_ok}
                        onChange={() => toggle(c.id)}
                        aria-describedby={`why-${c.id}`}
                      />
                      <span>
                        <span className="text-white font-medium text-sm">{c.company}</span>
                        <span id={`why-${c.id}`} className="block text-gray-400 text-sm mt-1">
                          Asks the company to {c.requests.includes("delete") ? "delete your data" : ""}
                          {c.requests.length > 1 ? " and " : ""}
                          {c.requests.includes("opt_out_sale") ? "stop selling or sharing it" : ""}, under {c.legal_basis}.
                        </span>
                      </span>
                    </label>
                    <div className="ml-7 mt-2 text-xs text-gray-500 space-y-1">
                      <p>
                        Would be emailed to <span className="text-gray-300">{c.address}</span>, the address{" "}
                        <a href={c.source_url} target="_blank" rel="noopener noreferrer" className="text-teal-400 hover:text-teal-300 inline-flex items-center gap-1">
                          the company publishes <ExternalLink className="w-3 h-3" aria-hidden="true" />
                        </a>
                        . We last checked on {formatDate(c.verified_on)}.
                      </p>
                      <p>{c.agent_requirements}</p>
                      {c.notes && <p>{c.notes}</p>}
                      {c.requires_address && (
                        <p className="text-gray-400">This company needs your address to find your records, so choosing it shares your address.</p>
                      )}
                      {!c.eligible && c.ineligible_reason && (
                        <p className="text-orange-300" data-testid={`ineligible-${c.id}`}>{c.ineligible_reason}</p>
                      )}
                    </div>
                  </li>
                ))}
              </ul>
            </section>

            <section aria-labelledby="share-heading" className="mb-6">
              <h2 id="share-heading" className="text-white font-semibold mb-2">What we share</h2>
              <p className="text-gray-400 text-sm mb-3">
                Always your name and your email address (so the company can verify you and reply to you). We never
                send your phone numbers or date of birth.
              </p>
              <label className="flex items-start gap-3 text-sm text-gray-300 cursor-pointer">
                <input
                  type="checkbox"
                  className="mt-1"
                  checked={sharingAddress}
                  disabled={needsAddress}
                  onChange={(e) => setIncludeAddress(e.target.checked)}
                />
                <span>
                  Also include my address, which can help a company find my records.
                  {needsAddress && " A company you chose needs it, so it is included."}
                </span>
              </label>
            </section>

            <section aria-labelledby="auth-heading" className="mb-6">
              <h2 id="auth-heading" className="text-white font-semibold mb-2">Your authorization</h2>
              <div
                tabIndex={0}
                role="region"
                aria-label="Authorization text"
                className="bg-gray-900 border border-gray-700 rounded-lg p-4 text-sm text-gray-300 whitespace-pre-wrap max-h-64 overflow-y-auto focus:ring-2 focus:ring-teal-600 focus:outline-none"
              >
                {info.authorization.text}
              </div>
              <p className="text-gray-600 text-xs mt-1">Version {info.authorization.version}. We keep the exact text you accept.</p>
              <label className="flex items-start gap-3 text-sm text-gray-200 mt-3 cursor-pointer">
                <input type="checkbox" className="mt-1" checked={consent} onChange={(e) => setConsent(e.target.checked)} />
                <span>I have read this and I authorize VectorScrub to send these requests for me.</span>
              </label>
            </section>

            <button
              onClick={() => void submit()}
              disabled={!canSubmit}
              className="bg-teal-600 hover:bg-teal-500 disabled:opacity-40 disabled:cursor-not-allowed text-white px-5 py-2.5 rounded-lg text-sm font-medium transition-colors"
            >
              {busy ? "Sending…" : picked.size === 0 ? "Send requests" : `Send ${picked.size} request${picked.size === 1 ? "" : "s"}`}
            </button>

            <section aria-labelledby="yours-heading" className="mt-12">
              <h2 id="yours-heading" className="text-white font-semibold mb-3">Your requests</h2>
              <div className="flex items-start gap-2 text-gray-500 text-xs mb-4">
                <Info className="w-4 h-4 flex-shrink-0 mt-0.5" aria-hidden="true" />
                <span>
                  Companies reply to your email address, not to us, so we cannot see their answer. Tell us what
                  you hear and we will keep the record. A company is expected to answer within 45 days.
                </span>
              </div>
              {requests.length === 0 ? (
                <p className="text-gray-500 text-sm">No requests yet.</p>
              ) : (
                <ul className="space-y-3">
                  {requests.map((r) => (
                    <li key={r.id} className="bg-gray-900 border border-gray-800 rounded-xl p-5">
                      <div className="flex flex-wrap items-center justify-between gap-2 mb-1">
                        <span className="text-white text-sm font-medium">{r.company}</span>
                        <span className={`text-xs px-3 py-1 rounded-full border ${STATUS[r.status].style}`}>{STATUS[r.status].text}</span>
                      </div>
                      <p className="text-gray-500 text-xs">
                        Requested {formatDate(r.queued_at)}
                        {r.sent_at ? ` · sent ${formatDate(r.sent_at)}` : ""}
                        {r.outcome_at ? ` · updated ${formatDate(r.outcome_at)}` : ""}
                      </p>
                      {r.status === "failed" && r.error && <p className="text-orange-300 text-xs mt-2">{r.error}</p>}
                      {r.follow_up_due && (
                        <p className="text-orange-300 text-xs mt-2" data-testid={`overdue-${r.id}`}>
                          It has been more than 45 days with no answer reported. You can contact the company directly
                          using the{" "}
                          <Link href="/dashboard/optouts" className="underline">manual opt-out guide</Link>.
                        </p>
                      )}
                      {(r.status === "sent" || r.status === "replied") && (
                        <div className="flex flex-wrap items-center gap-2 mt-3 text-xs">
                          <span className="text-gray-400">What did the company say?</span>
                          {r.status === "sent" && (
                            <button onClick={() => void report(r.id, "replied")} className="px-3 py-1 rounded border border-gray-700 text-gray-200 hover:border-gray-500">
                              It replied
                            </button>
                          )}
                          <button onClick={() => void report(r.id, "removed")} className="px-3 py-1 rounded border border-green-800 text-green-300 hover:border-green-600">
                            It deleted my data
                          </button>
                          <button onClick={() => void report(r.id, "denied")} className="px-3 py-1 rounded border border-orange-800 text-orange-300 hover:border-orange-600">
                            It declined
                          </button>
                        </div>
                      )}
                    </li>
                  ))}
                </ul>
              )}
            </section>
          </>
        )}
      </main>
    </div>
  );
}
