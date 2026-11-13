"use client";

import { useEffect, useState } from "react";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { Shield, LogOut, ArrowLeft } from "lucide-react";
import { listRemovalRequests, logout, markRemovalDoneByHand, requestRemovalAssistance, updateListingLink } from "@/lib/api";
import {
  removalStage,
  formatDate,
  isAutomationBlocked,
  isOptOutPageGone,
} from "@/lib/utils";
import { BROKER_OPT_OUTS } from "@/lib/brokers.generated";
import { safeHref } from "@/lib/caBrokerSearch";
import type { RemovalRequest } from "@/types";

export default function RemovalsPage() {
  const router = useRouter();
  const [requests, setRequests] = useState<RemovalRequest[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    // No client-side token check: the session is an HttpOnly cookie the page cannot see.
    // If it is missing or expired the first API call answers 401 and the API client sends
    // the user to /login.
    loadRequests();
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  async function loadRequests() {
    setLoading(true);
    try {
      const data = await listRemovalRequests();
      setRequests(data.requests);
    } finally {
      setLoading(false);
    }
  }

  // Refresh after an action without blanking the page, so the card can show what just happened.
  async function refreshQuietly() {
    const data = await listRemovalRequests();
    setRequests(data.requests);
  }

  // One card per listing: its newest request. Older attempts (for example automated tries that the
  // site blocked) are history, not something to act on, so they are only counted on the card.
  const byExposure = new Map<string, RemovalRequest[]>();
  for (const r of requests) byExposure.set(r.exposure_id, [...(byExposure.get(r.exposure_id) ?? []), r]);
  const current = [...byExposure.values()]
    .map((rs) => [...rs].sort((a, b) => b.created_at.localeCompare(a.created_at)))
    .sort((a, b) => b[0].created_at.localeCompare(a[0].created_at))
    .map((rs) => ({ ...rs[0], earlier: rs.length - 1 }));

  function handleSignOut() {
    void logout().finally(() => router.push("/"));
  }

  if (loading) {
    return (
      <div className="min-h-screen flex items-center justify-center">
        <div className="text-gray-400 text-sm animate-pulse">
          Loading removal requests…
        </div>
      </div>
    );
  }

  return (
    <div className="min-h-screen">
      {/* Top nav */}
      <nav className="border-b border-gray-800 bg-black/40 backdrop-blur-sm sticky top-0 z-10">
        <div className="max-w-6xl mx-auto px-6 py-4 flex items-center justify-between">
          <div className="flex items-center gap-2">
            <Shield className="w-6 h-6 text-teal-400" aria-hidden="true" />
            <span className="font-bold text-lg text-white">VectorScrub</span>
          </div>
          <button
            onClick={handleSignOut}
            className="flex items-center gap-2 text-gray-400 hover:text-white transition-colors text-sm"
          >
            <LogOut className="w-4 h-4" />
            Sign Out
          </button>
        </div>
      </nav>

      <main className="max-w-6xl mx-auto px-6 py-10">
        <div className="flex items-center gap-4 mb-6">
          <button
            onClick={() => router.push("/dashboard")}
            className="text-gray-400 hover:text-white transition-colors"
          >
            <ArrowLeft className="w-5 h-5" />
          </button>
          <h1 className="text-2xl font-bold text-white">Removal Requests</h1>
        </div>

        {requests.length === 0 ? (
          <div className="bg-gray-900 border border-gray-800 rounded-xl p-12 text-center">
            <p className="text-gray-400 mb-1">No removal requests yet.</p>
            <p className="text-gray-600 text-sm">
              Authorize removals from your dashboard to track their progress here.
            </p>
          </div>
        ) : (
          <div className="space-y-4">
            {current.map((req) => (
              <div
                key={req.id}
                className="bg-gray-900 border border-gray-800 rounded-xl p-5"
              >
                <div className="flex items-center justify-between mb-3">
                  <span className="text-white font-medium">{req.source_name}</span>
                  <span
                    className={`text-xs px-3 py-1 rounded-full border ${removalStage(req).color}`}
                  >
                    {removalStage(req).label}
                  </span>
                </div>

                <p className="text-gray-300 text-sm mb-2" data-testid={`stage-${req.id}`}>
                  {removalStage(req).detail}
                </p>
                {req.earlier > 0 && (
                  <p className="text-gray-500 text-xs mb-2">
                    {req.earlier} earlier {req.earlier === 1 ? "attempt" : "attempts"} for this listing did not go through.
                    They are kept in your history.
                  </p>
                )}

                <div className="text-gray-400 text-sm space-y-1">
                  <div>
                    Authorized: {formatDate(req.customer_authorized_at)}
                  </div>
                  {req.submitted_at && !req.manual_reported_at && <div>Submitted: {formatDate(req.submitted_at)}</div>}
                  {req.confirmation_ref && (
                    <div>Confirmation: {req.confirmation_ref}</div>
                  )}
                  {req.manual_reported_at && (
                    <div>
                      You reported submitting it
                      {req.manual_submitted_on ? ` on ${formatDate(`${req.manual_submitted_on}T12:00:00`)}` : ""}
                      {req.manual_email_confirmation === "confirmed"
                        ? " and confirming the site's email"
                        : req.manual_email_confirmation === "not_required"
                          ? " (no email confirmation needed)"
                          : ""}
                    </div>
                  )}
                  {req.last_check_at && (
                    <div>
                      Last check: {formatDate(req.last_check_at)} (
                      {req.last_check_result === "removed"
                        ? "listing gone"
                        : req.last_check_result === "still_present"
                          ? "listing still there"
                          : "inconclusive: we could not tell"}
                      )
                    </div>
                  )}
                  {req.next_check_at && (req.status === "submitted" || req.status === "acknowledged") && (
                    <div>Next check: {formatDate(req.next_check_at)}</div>
                  )}
                  {req.verified_at && (
                    <div className="text-green-400">
                      Verified removed: {formatDate(req.verified_at)}
                    </div>
                  )}
                </div>

                {req.status === "failed" && req.failure_reason && (
                  <p className="mt-3 text-sm text-gray-300 border-t border-gray-800 pt-3" data-testid={`reason-${req.id}`}>
                    {req.failure_reason}
                  </p>
                )}

                {req.procedure_used === "manual" && (req.status === "submitted" || req.status === "acknowledged") && (
                  <CheckedLink req={req} onDone={refreshQuietly} />
                )}

                {(req.status === "failed" || req.status === "rejected" || req.status === "unverifiable") && (
                  <ManualRoute req={req} onDone={loadRequests} />
                )}
              </div>
            ))}
          </div>
        )}
      </main>
    </div>
  );
}

/**
 * What to do by hand when automation cannot finish a removal: the company's own opt-out page and,
 * because most forms ask for it, the link to the person's own listing. It says plainly when
 * retrying cannot help (a site that refuses our browser will refuse it again). Afterwards the
 * customer reports what they did. That report is recorded as their claim; only our own re-check
 * can mark the listing removed.
 */
function ManualRoute({ req, onDone }: { req: RemovalRequest; onDone: () => Promise<void> | void }) {
  const optOut = BROKER_OPT_OUTS.find((b) => b.name === req.source_name);
  const optOutHref = optOut?.optOutUrl ? safeHref(optOut.optOutUrl) : null;
  const listing = safeHref(req.source_url);
  const retryWillNotHelp = isAutomationBlocked(req.failure_reason) || isOptOutPageGone(req.failure_reason);

  // The customer's own calendar day, not UTC: in the evening in California, UTC is already tomorrow.
  const now = new Date();
  const today = `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, "0")}-${String(now.getDate()).padStart(2, "0")}`;
  const [submitted, setSubmitted] = useState(false);
  const [email, setEmail] = useState<"" | "confirmed" | "not_required" | "pending">("");
  const [date, setDate] = useState(today);
  const [profileUrl, setProfileUrl] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [helpBusy, setHelpBusy] = useState(false);
  const [helpMessage, setHelpMessage] = useState<string | null>(null);

  async function report() {
    if (email === "" || email === "pending") return;
    setBusy(true);
    setError(null);
    try {
      await markRemovalDoneByHand(req.id, {
        email_confirmation: email,
        submitted_on: date,
        ...(profileUrl.trim() ? { listing_url: profileUrl.trim() } : {}),
      });
      await onDone();
    } catch (e) {
      setError(e instanceof Error ? e.message : "Could not save. Try again.");
    } finally {
      setBusy(false);
    }
  }

  async function askForHelp() {
    setHelpBusy(true);
    setError(null);
    try {
      const r = await requestRemovalAssistance(req.id);
      setHelpMessage(r.message);
      await onDone();
    } catch (e) {
      setError(e instanceof Error ? e.message : "Could not send the request. Try again.");
    } finally {
      setHelpBusy(false);
    }
  }

  return (
    <div className="mt-3 text-sm text-orange-300 border-t border-gray-800 pt-3 space-y-3" data-testid={`manual-${req.id}`}>
      <p>
        {retryWillNotHelp
          ? "Trying again will not help here, so this one needs to be done by hand. It takes a few minutes:"
          : "Many sites block automated tools, so this one may need to be done by hand. It takes a few minutes:"}
      </p>
      <ol className="list-decimal ml-5 space-y-1 text-gray-300">
        {listing && (
          <li>
            Open{" "}
            <a href={listing} target="_blank" rel="noopener noreferrer" className="text-teal-400 hover:text-teal-300 underline break-all">
              the page where we found you on {req.source_name}
            </a>
            . If it is a list of search results, click the one that is you (right city and age) and copy{" "}
            <em>that</em> page&apos;s address: most opt-out forms reject a search-results link.
          </li>
        )}
        <li>
          {optOutHref ? (
            <>
              Open{" "}
              <a href={optOutHref} target="_blank" rel="noopener noreferrer" className="text-teal-400 hover:text-teal-300 underline">
                {req.source_name}&apos;s opt-out page
              </a>{" "}
              and submit the request.
            </>
          ) : (
            <>
              Follow the steps in the{" "}
              <Link href="/dashboard/optouts" className="text-teal-400 hover:text-teal-300 underline">
                manual opt-out guide
              </Link>
              .
            </>
          )}
        </li>
        <li>If the site emails you a confirmation link, click it. The request does not count until you do.</li>
      </ol>

      {req.status === "failed" && (
        <div className="border border-gray-800 rounded-lg p-3 space-y-2 text-gray-300" data-testid={`report-${req.id}`}>
          <p className="text-gray-200 font-medium">Did you complete it?</p>
          {!submitted ? (
            <div className="flex gap-2">
              <button
                onClick={() => setSubmitted(true)}
                className="px-3 py-1.5 rounded-lg border border-teal-600 text-teal-300 hover:bg-teal-900/30 text-sm"
              >
                Yes, I submitted it
              </button>
              <span className="text-xs text-gray-500 self-center">Not yet? Come back when you have.</span>
            </div>
          ) : (
            <div className="space-y-2">
              <fieldset className="space-y-1">
                <legend className="text-xs text-gray-400">Did the site ask you to confirm by email?</legend>
                {(
                  [
                    ["confirmed", "Yes, and I clicked the link"],
                    ["not_required", "No email confirmation was needed"],
                    ["pending", "Yes, but I have not clicked it yet"],
                  ] as const
                ).map(([value, text]) => (
                  <label key={value} className="flex items-center gap-2 text-sm">
                    <input type="radio" name={`email-${req.id}`} checked={email === value} onChange={() => setEmail(value)} />
                    {text}
                  </label>
                ))}
              </fieldset>
              {email === "pending" && (
                <p className="text-amber-300 text-xs">
                  Click the link in the site&apos;s email first. Until you do, the request has not been
                  submitted. Then come back and report it.
                </p>
              )}
              <label className="block text-xs text-gray-400">
                Day you submitted it
                <input
                  type="date"
                  value={date}
                  max={today}
                  onChange={(e) => setDate(e.target.value)}
                  className="ml-2 bg-gray-800 border border-gray-700 rounded px-2 py-1 text-white text-sm"
                />
              </label>
              <label className="block text-xs text-gray-400">
                The exact profile link you opted out (optional, recommended)
                <input
                  type="url"
                  inputMode="url"
                  value={profileUrl}
                  onChange={(e) => setProfileUrl(e.target.value)}
                  placeholder={`https://www.${req.source_name}/...`}
                  className="mt-1 block w-full bg-gray-800 border border-gray-700 rounded px-2 py-1 text-white text-sm"
                />
                <span className="text-gray-500">
                  If you used a different link from ours (for example one profile instead of a search page), paste it
                  here. We will re-check that exact page, which gives a clearer result. It must be on {req.source_name}.
                </span>
              </label>
              <button
                onClick={report}
                disabled={busy || email === "" || email === "pending" || !date}
                className="px-3 py-1.5 rounded-lg border border-teal-600 text-teal-300 hover:bg-teal-900/30 disabled:opacity-50 text-sm"
                data-testid={`done-by-hand-${req.id}`}
              >
                {busy ? "Saving..." : "Record what I did"}
              </button>
            </div>
          )}
          <p className="text-gray-500 text-xs">
            This records what you tell us. It does not verify removal. We re-check the listing on a schedule
            (about 3, 7, 14 and 30 days after you submitted it, up to 45), and sites can take weeks. If we
            cannot read the page, we say the result is inconclusive; we never assume it is gone.
          </p>
        </div>
      )}

      {req.status === "failed" && (
        <div className="text-gray-300 text-xs space-y-1">
          {req.assistance_status === "requested" || req.assistance_status === "in_progress" || helpMessage ? (
            <p data-testid={`help-${req.id}`}>
              Help requested{req.assistance_requested_at ? ` on ${formatDate(req.assistance_requested_at)}` : ""}. A person will
              write to your account email. This is a free pilot service with no guaranteed response time.
            </p>
          ) : (
            <>
              <button
                onClick={askForHelp}
                disabled={helpBusy}
                className="px-3 py-1.5 rounded-lg border border-gray-600 text-gray-200 hover:border-gray-400 disabled:opacity-50 text-sm"
                data-testid={`ask-help-${req.id}`}
              >
                {helpBusy ? "Sending..." : "Ask a person to help me with this"}
              </button>
              <p className="text-gray-500">
                Pilot only: someone will write to your account email and walk you through it. We never ask
                for your email password or to get around a site&apos;s verification, and you stay in control of
                every step.
              </p>
            </>
          )}
        </div>
      )}
      {error && <p className="text-red-400 text-xs">{error}</p>}

      <p className="text-gray-500 text-xs">
        The{" "}
        <Link href="/dashboard/optouts" className="underline">
          opt-out guide
        </Link>{" "}
        has the steps for every site.
      </p>
    </div>
  );
}

/**
 * Which page our re-checks visit, and a way to point them at the exact profile the customer opted
 * out (brokers often want one profile, and that page gives the clearest result).
 */
function CheckedLink({ req, onDone }: { req: RemovalRequest; onDone: () => Promise<void> | void }) {
  const current = safeHref(req.source_url);
  const [open, setOpen] = useState(false);
  const [url, setUrl] = useState("");
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState<{ ok: boolean; text: string } | null>(null);

  async function save() {
    setBusy(true);
    setMsg(null);
    try {
      const r = await updateListingLink(req.id, url.trim());
      setMsg({ ok: true, text: `${r.message} Click "this page" to check it opens the right profile.` });
      setOpen(false);
      setUrl("");
      await onDone();
    } catch (e) {
      setMsg({ ok: false, text: e instanceof Error ? e.message : "Could not update the link." });
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="mt-3 text-xs text-gray-400 border-t border-gray-800 pt-3 space-y-2" data-testid={`checked-link-${req.id}`}>
      <p>
        We re-check{" "}
        {current ? (
          <a href={current} target="_blank" rel="noopener noreferrer" className="text-teal-400 underline break-all">
            this page
          </a>
        ) : (
          "the page we found"
        )}
        .{" "}
        {!open && (
          <button onClick={() => setOpen(true)} className="underline text-gray-300">
            Opted out a different link? Update it
          </button>
        )}
      </p>
      {open && (
        <div className="flex flex-col sm:flex-row gap-2">
          <label htmlFor={`link-${req.id}`} className="sr-only">Profile link you opted out</label>
          <input
            id={`link-${req.id}`}
            type="url"
            inputMode="url"
            value={url}
            onChange={(e) => setUrl(e.target.value)}
            placeholder={`https://www.${req.source_name}/...`}
            className="flex-1 bg-gray-800 border border-gray-700 rounded px-2 py-1 text-white text-sm"
          />
          <button
            onClick={save}
            disabled={busy || !url.trim()}
            className="px-3 py-1 rounded border border-teal-600 text-teal-300 disabled:opacity-50 text-sm"
          >
            {busy ? "Saving..." : "Use this link"}
          </button>
        </div>
      )}
      {msg && <p className={msg.ok ? "text-teal-300" : "text-red-400"}>{msg.text}</p>}
    </div>
  );
}
