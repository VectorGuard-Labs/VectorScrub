"use client";

import { useEffect, useState } from "react";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { LogOut, ExternalLink, Info } from "lucide-react";
import {
  ApiError,
  createDeletionCampaign,
  getDeletionInfo,
  listDeletionRequests,
  logout,
  reportDeletionOutcome,
  withdrawDeletionCampaign,
  withdrawDeletionRequest,
} from "@/lib/api";
import { formatDate } from "@/lib/utils";
import type { DeletionInfo, DeletionKind, DeletionRequest, DeletionStatus } from "@/types";
import { safeHref } from "@/lib/caBrokerSearch";

const STATUS: Record<DeletionStatus, { text: string; style: string }> = {
  queued: { text: "Waiting to be sent", style: "text-yellow-300 border-yellow-800 bg-yellow-950" },
  sending: { text: "Sending", style: "text-yellow-300 border-yellow-800 bg-yellow-950" },
  sent: { text: "Sent. Waiting for the company", style: "text-blue-300 border-blue-800 bg-blue-950" },
  failed: { text: "Not sent", style: "text-red-300 border-red-800 bg-red-950" },
  replied: { text: "The company replied", style: "text-blue-300 border-blue-800 bg-blue-950" },
  removed: { text: "Company says it deleted your data", style: "text-green-400 border-green-800 bg-green-950" },
  denied: { text: "The company declined", style: "text-orange-300 border-orange-800 bg-orange-950" },
  withdrawn: { text: "Withdrawn. Never sent", style: "text-gray-300 border-gray-700 bg-gray-900" },
};

/** What each kind of request asks a company to do, in the customer's words. */
const KIND_LABEL: Record<DeletionKind, { choice: string; asks: string }> = {
  delete: { choice: "Ask them to delete the personal information they hold about me", asks: "delete your data" },
  opt_out_sale: { choice: "Ask them to stop selling or sharing my personal information", asks: "stop selling or sharing it" },
};

function kindsText(kinds: DeletionKind[]): string {
  return kinds.map((k) => KIND_LABEL[k].asks).join(" and ");
}

export default function DeletionRequestsPage() {
  const router = useRouter();
  const [info, setInfo] = useState<DeletionInfo | null>(null);
  const [unavailable, setUnavailable] = useState(false);
  const [requests, setRequests] = useState<DeletionRequest[]>([]);
  const [loading, setLoading] = useState(true);
  const [picked, setPicked] = useState<Set<string>>(new Set());
  // Deleting and opting out of sale or sharing are separate choices. Nothing is ticked for the customer.
  const [kinds, setKinds] = useState<Set<DeletionKind>>(new Set());
  const [signature, setSignature] = useState("");
  const [esignConsent, setEsignConsent] = useState(false);
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
    let live = true;
    Promise.all([getDeletionInfo(), listDeletionRequests()])
      .then(([i, r]) => {
        if (!live) return;
        setInfo(i);
        setRequests(r);
      })
      .catch((e) => {
        if (!live) return;
        if (e instanceof ApiError && e.status === 404) setUnavailable(true);
        else setError("Could not load this page. Please try again.");
      })
      .finally(() => {
        if (live) setLoading(false);
      });
    return () => {
      live = false;
    };
  }, []);

  function toggle(id: string) {
    setPicked((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }

  function toggleKind(k: DeletionKind) {
    setKinds((prev) => {
      const next = new Set(prev);
      if (next.has(k)) next.delete(k);
      else next.add(k);
      return next;
    });
  }

  async function withdraw(id: string) {
    setError("");
    setMessage("");
    try {
      await withdrawDeletionRequest(id);
      setMessage("Withdrawn. That request will not be sent.");
      await load();
    } catch (e) {
      setError(e instanceof Error ? e.message : "Something went wrong.");
      await load();
    }
  }

  async function withdrawAllWaiting() {
    setError("");
    setMessage("");
    const campaigns = [...new Set(requests.filter((r) => r.status === "queued").map((r) => r.campaign_id))];
    try {
      let n = 0;
      for (const id of campaigns) n += await withdrawDeletionCampaign(id);
      setMessage(n === 1 ? "Withdrawn. 1 request will not be sent." : `Withdrawn. ${n} requests will not be sent.`);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Something went wrong.");
    }
    await load();
  }

  async function submit() {
    if (!info) return;
    setBusy(true);
    setError("");
    setMessage("");
    try {
      const r = await createDeletionCampaign({
        channel_ids: [...picked],
        request_kinds: info.request_kinds.filter((k) => kinds.has(k)),
        include_address: sharingAddress,
        consent,
        authorization_version: info.authorization.version,
        signature,
        esign_consent: esignConsent,
      });
      setMessage(r.message);
      setPicked(new Set());
      setKinds(new Set());
      setConsent(false);
      setSignature("");
      setEsignConsent(false);
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
  // What a company would be asked, given the kinds chosen: the kinds its page offers. None means there is nothing to send it.
  const askedOf = (c: { requests: string[] }) => (info?.request_kinds ?? []).filter((k) => kinds.has(k) && c.requests.includes(k));
  const offersNothingChosen = (c: { requests: string[] }) => kinds.size > 0 && askedOf(c).length === 0;
  const pickedNothingOffered = !!info?.channels.some((c) => picked.has(c.id) && offersNothingChosen(c));
  const open = !!info?.plan_ok && !!info?.in_pilot;
  const signed = esignConsent && signature.trim().length > 0;
  const canSubmit = open && picked.size > 0 && kinds.size > 0 && !pickedNothingOffered && consent && signed && !busy;
  const waiting = requests.filter((r) => r.status === "queued");

  return (
    <div className="flex-1">
      <nav className="border-b border-gray-800 px-6 py-4 flex items-center justify-between">
        <div className="flex items-center gap-2">
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
            <Link href="/dashboard/optouts" className="text-brand-blue-400 hover:text-brand-blue-300 underline">
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

            {info.in_pilot && (
              <div className="bg-gray-900 border border-gray-800 rounded-lg px-4 py-3 mb-6 text-sm text-gray-300" data-testid="drop-note">
                <p className="text-white font-medium mb-1">Use California&apos;s DROP too</p>
                <p>
                  California runs a free service, DROP, that sends one deletion request to every registered data broker.
                  Since August 1, 2026, registered data brokers must check it at least every 45 days and delete matching
                  records. The companies below are listed in the state&apos;s data broker registry. Requests from this page
                  do not replace DROP, and we do not submit DROP requests for you. Use both:{" "}
                  <a href="https://consumer.drop.privacy.ca.gov/" target="_blank" rel="noopener noreferrer" className="text-brand-blue-400 hover:text-brand-blue-300 inline-flex items-center gap-1">
                    open DROP <ExternalLink className="w-3 h-3" aria-hidden="true" />
                  </a>
                  .
                </p>
              </div>
            )}

            {!info.in_pilot && (
              <p className="bg-gray-900 border border-gray-800 rounded-lg px-4 py-3 mb-6 text-sm text-gray-300" data-testid="outside-pilot">
                This is a small pilot, open for now only to residents of {info.pilot_states.join(", ")}. We use the state of
                the address in your saved details, and it is not open in yours yet, or we could not tell your state. Your
                requests are not affected, and the{" "}
                <Link href="/dashboard/optouts" className="text-brand-blue-400 hover:text-brand-blue-300 underline">manual opt-out guide</Link>{" "}
                shows how to ask each company yourself.
              </p>
            )}

            {!info.plan_ok && (
              <p className="bg-gray-900 border border-gray-800 rounded-lg px-4 py-3 mb-6 text-sm text-gray-300">
                Sending requests is part of the paid plan.{" "}
                <Link href="/dashboard/billing" className="text-brand-blue-400 hover:text-brand-blue-300 underline">See plans</Link>
              </p>
            )}

            {(error || message) && (
              <div
                role={error ? "alert" : "status"}
                className={`rounded-lg px-4 py-3 mb-6 text-sm border ${
                  error ? "bg-red-950 border-red-800 text-red-300" : "bg-brand-blue-950 border-brand-blue-800 text-brand-blue-300"
                }`}
              >
                {error || message}
              </div>
            )}

            <section aria-labelledby="kinds-heading" className="mb-8">
              <h2 id="kinds-heading" className="text-white font-semibold mb-2">1. What to ask for</h2>
              <p className="text-gray-400 text-sm mb-3">
                These are separate choices and nothing is ticked for you. A company is asked only for what you tick here, and
                only for what its own page offers.
              </p>
              <div className="space-y-2">
                {info.request_kinds.map((k) => (
                  <label key={k} className={`flex items-start gap-3 text-sm text-gray-200 ${open ? "cursor-pointer" : "opacity-70"}`}>
                    <input type="checkbox" className="mt-1" checked={kinds.has(k)} disabled={!open} onChange={() => toggleKind(k)} />
                    <span>{KIND_LABEL[k].choice}</span>
                  </label>
                ))}
              </div>
            </section>

            <section aria-labelledby="choose-heading" className="mb-10">
              <h2 id="choose-heading" className="text-white font-semibold mb-3">2. Choose companies</h2>
              <ul className="space-y-3">
                {info.channels.map((c) => (
                  <li key={c.id} className="bg-gray-900 border border-gray-800 rounded-xl p-5">
                    <label className={`flex items-start gap-3 ${c.eligible && open && !offersNothingChosen(c) ? "cursor-pointer" : "opacity-70"}`}>
                      <input
                        type="checkbox"
                        className="mt-1"
                        checked={picked.has(c.id)}
                        disabled={!c.eligible || !open || offersNothingChosen(c)}
                        onChange={() => toggle(c.id)}
                        aria-describedby={`why-${c.id}`}
                      />
                      <span>
                        <span className="text-white font-medium text-sm">{c.company}</span>
                        <span id={`why-${c.id}`} className="block text-gray-400 text-sm mt-1">
                          {kinds.size === 0
                            ? <>Its page offers a request to {kindsText(c.requests as DeletionKind[])}, under {c.legal_basis}.</>
                            : offersNothingChosen(c)
                              ? <>Its page does not offer what you ticked above, so there is nothing to send it.</>
                              : <>Would ask the company to {kindsText(askedOf(c) as DeletionKind[])}, under {c.legal_basis}.</>}
                        </span>
                      </span>
                    </label>
                    <div className="ml-7 mt-2 text-xs text-gray-500 space-y-1">
                      <p>
                        Would be emailed to <span className="text-gray-300">{c.address}</span>, the address{" "}
                        <a href={safeHref(c.source_url) ?? undefined} target="_blank" rel="noopener noreferrer" className="text-brand-blue-400 hover:text-brand-blue-300 inline-flex items-center gap-1">
                          the company publishes <ExternalLink className="w-3 h-3" aria-hidden="true" />
                        </a>
                        . We last checked on {formatDate(c.verified_on)}.
                      </p>
                      {c.registry_id && <p>Listed in California&apos;s data broker registry (no. {c.registry_id}).</p>}
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
              <h2 id="share-heading" className="text-white font-semibold mb-2">3. What we share</h2>
              <p className="text-gray-400 text-sm mb-3">
                Always your name and your email address (so the company can verify you and reply to you). We never
                send your phone numbers or date of birth.
              </p>
              <label className="flex items-start gap-3 text-sm text-gray-300 cursor-pointer">
                <input
                  type="checkbox"
                  className="mt-1"
                  checked={sharingAddress}
                  disabled={needsAddress || !open}
                  onChange={(e) => setIncludeAddress(e.target.checked)}
                />
                <span>
                  Also include my address, which can help a company find my records.
                  {needsAddress && " A company you chose needs it, so it is included."}
                </span>
              </label>
            </section>

            <section aria-labelledby="auth-heading" className="mb-6">
              <h2 id="auth-heading" className="text-white font-semibold mb-2">4. Your authorization</h2>
              <div
                tabIndex={0}
                role="region"
                aria-label="Authorization text"
                className="bg-gray-900 border border-gray-700 rounded-lg p-4 text-sm text-gray-300 whitespace-pre-wrap max-h-64 overflow-y-auto focus:ring-2 focus:ring-brand-blue-600 focus:outline-none"
              >
                {info.authorization.text}
              </div>
              <p className="text-gray-600 text-xs mt-1">Version {info.authorization.version}. We keep the exact text you accept.</p>
              <label className="flex items-start gap-3 text-sm text-gray-200 mt-3 cursor-pointer">
                <input type="checkbox" className="mt-1" checked={consent} disabled={!open} onChange={(e) => setConsent(e.target.checked)} />
                <span>
                  I have read this and I authorize VectorScrub to send the requests I ticked above, to the companies I
                  selected, for me.
                </span>
              </label>
            </section>

            <section aria-labelledby="sign-heading" className="mb-6">
              <h2 id="sign-heading" className="text-white font-semibold mb-2">5. Sign</h2>
              <p className="text-gray-400 text-sm mb-3">
                Some companies ask for proof of your signed permission. You sign by typing your full name, exactly as it
                appears in your saved details (it is the name the companies receive). We keep the signed copy, encrypted, and
                you can download it as a PDF from this page.
              </p>
              <label className="flex items-start gap-3 text-sm text-gray-200 mb-3 cursor-pointer">
                <input type="checkbox" className="mt-1" checked={esignConsent} disabled={!open} onChange={(e) => setEsignConsent(e.target.checked)} />
                <span>I agree to sign this authorization electronically, and that my typed name is my signature.</span>
              </label>
              <label htmlFor="signature" className="block text-gray-300 text-sm mb-1">Your full name</label>
              <input
                id="signature"
                type="text"
                autoComplete="name"
                maxLength={200}
                value={signature}
                disabled={!open}
                onChange={(e) => setSignature(e.target.value)}
                className="w-full max-w-md bg-gray-900 border border-gray-700 rounded-lg px-3 py-2 text-white text-sm focus:ring-2 focus:ring-brand-blue-600 focus:outline-none"
              />
            </section>

            <button
              onClick={() => void submit()}
              disabled={!canSubmit}
              className="bg-brand-red-600 hover:bg-brand-red-500 disabled:opacity-40 disabled:cursor-not-allowed text-white px-5 py-2.5 rounded-lg text-sm font-medium transition-colors"
            >
              {busy ? "Sending…" : picked.size === 0 ? "Sign and send requests" : `Sign and send ${picked.size} request${picked.size === 1 ? "" : "s"}`}
            </button>

            <section aria-labelledby="yours-heading" className="mt-12">
              <h2 id="yours-heading" className="text-white font-semibold mb-3">Your requests</h2>
              <div className="flex items-start gap-2 text-gray-500 text-xs mb-4">
                <Info className="w-4 h-4 flex-shrink-0 mt-0.5" aria-hidden="true" />
                <span>
                  Companies reply to your email address, not to us, so we cannot see their answer. Tell us what
                  you hear and we will keep the record. Under California law a company generally has 45 days to
                  respond, and can take up to 45 more if it tells you.
                </span>
              </div>
              {waiting.length > 0 && (
                <div className="flex flex-wrap items-center gap-3 mb-4 text-sm text-gray-300">
                  <span>
                    {waiting.length === 1 ? "1 request is waiting to be sent." : `${waiting.length} requests are waiting to be sent.`} You can
                    still withdraw {waiting.length === 1 ? "it" : "them"}. A request that has been sent cannot be recalled.
                  </span>
                  <button
                    onClick={() => void withdrawAllWaiting()}
                    className="px-3 py-1 rounded border border-gray-600 text-gray-200 hover:border-gray-400 text-xs"
                  >
                    Withdraw all waiting
                  </button>
                </div>
              )}
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
                      {r.requests.length > 0 && (
                        <p className="text-gray-500 text-xs mt-1">Asked it to {kindsText(r.requests)}.</p>
                      )}
                      {r.status === "failed" && r.error && <p className="text-orange-300 text-xs mt-2">{r.error}</p>}
                      {r.status === "sent" && !r.follow_up_due && (
                        <p className="text-gray-500 text-xs mt-2" data-testid={`waiting-${r.id}`}>
                          Within the usual response time. Check your email for a reply, or a request to confirm it was you.
                        </p>
                      )}
                      {r.follow_up_due && (
                        <p className="text-orange-300 text-xs mt-2" data-testid={`no-answer-${r.id}`}>
                          {r.extended_period_over
                            ? "It has been 90 days or more and no answer is reported. Even a company that took an extension has used it. "
                            : "It has been 45 days or more and no answer is reported. That is not proof the company is late: it may have replied to your email, or taken the extra time the law allows. "}
                          Check your email, then tell us what you heard. You can also contact the company directly using the{" "}
                          <Link href="/dashboard/optouts" className="underline">manual opt-out guide</Link>.
                        </p>
                      )}
                      <div className="flex flex-wrap items-center gap-3 mt-3 text-xs">
                        {r.status === "queued" && (
                          <button onClick={() => void withdraw(r.id)} className="px-3 py-1 rounded border border-gray-600 text-gray-200 hover:border-gray-400">
                            Withdraw
                          </button>
                        )}
                        <Link
                          href={`/dashboard/requests/authorization/${r.campaign_id}`}
                          className="text-brand-blue-400 hover:text-brand-blue-300 underline"
                        >
                          Signed authorization (PDF)
                        </Link>
                      </div>
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
