"use client";

import { useEffect, useMemo, useState } from "react";
import { useRouter } from "next/navigation";
import { Shield, LogOut, ExternalLink, Mail, Info } from "lucide-react";
import { addOwnListing, getOptOutHealth, getResidenceState, listExposures, listRemovalRequests, listScans, logout } from "@/lib/api";
import { formatDate, isAutomationBlocked } from "@/lib/utils";
import { BROKER_OPT_OUTS, type BrokerOptOut } from "@/lib/brokers.generated";
import CaRegistrySearch, { type RegistryData } from "@/components/CaRegistrySearch";
import SiteNetwork from "@/components/SiteNetwork";
import { ownersBySite } from "@/lib/caBrokerSearch";
import { PAGE_STATUS, byKey, isStale } from "@/lib/optoutHealth";
import type { Exposure, OptOutHealth, RemovalRequest, SourceOutcome } from "@/types";
import { ROUTE_STATUS, routeStatus, tracker, type StepState } from "@/lib/optoutGuide";
import { getPublicCoverageClient, type PublicCoverage } from "@/lib/coverage";
import { latestRequest } from "@/lib/report";

// California's own free deletion service for residents. It is run by the state, not by us.
const DROP_URL = "https://consumer.drop.privacy.ca.gov/";

const STEP_STYLE: Record<StepState, string> = {
  done: "text-green-400",
  pending: "text-amber-300",
  not_applicable: "text-gray-500",
  unknown: "text-gray-400",
};
const STEP_MARK: Record<StepState, string> = { done: "\u2713", pending: "\u2026", not_applicable: "\u2013", unknown: "?" };

export default function OptOutsPage() {
  const router = useRouter();
  const [outcomes, setOutcomes] = useState<Record<string, SourceOutcome>>({});
  const [listed, setListed] = useState<Set<string>>(new Set());
  const [exposures, setExposures] = useState<Exposure[]>([]);
  const [requests, setRequests] = useState<RemovalRequest[]>([]);
  const [publicCoverage, setPublicCoverage] = useState<PublicCoverage | null>(null);
  const [newUrl, setNewUrl] = useState("");
  const [adding, setAdding] = useState(false);
  const [addMessage, setAddMessage] = useState<{ ok: boolean; text: string } | null>(null);
  const [health, setHealth] = useState<Record<string, OptOutHealth>>({});
  const [registryHealth, setRegistryHealth] = useState<Record<string, OptOutHealth>>({});
  // null = unknown (no address saved, or its state cannot be told)
  const [residence, setResidence] = useState<string | null>(null);
  const [registry, setRegistry] = useState<RegistryData | null>(null);
  const [registryFailed, setRegistryFailed] = useState(false);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        // The page check is a nice-to-have: the guide works without it.
        getPublicCoverageClient().then((c) => {
          if (!cancelled) setPublicCoverage(c);
        });
        const [scans, exposures, checks, registryChecks, reqs] = await Promise.all([
          listScans(),
          listExposures(),
          getOptOutHealth("optout").catch(() => [] as OptOutHealth[]),
          getOptOutHealth("ca_rights").catch(() => [] as OptOutHealth[]),
          listRemovalRequests().catch(() => ({ requests: [] as RemovalRequest[] })),
        ]);
        setExposures(exposures.exposures);
        setRequests(reqs.requests);
        if (cancelled) return;
        setHealth(byKey(checks));
        setRegistryHealth(byKey(registryChecks));
        getResidenceState()
          .then((st) => {
            if (!cancelled) setResidence(st);
          })
          .catch(() => {
            // unknown: the page then asks "Live in California?" instead of guessing
          });
        const latest = [...scans.jobs]
          .filter((j) => j.status === "completed" || j.status === "failed")
          .sort((a, b) => new Date(b.queued_at).getTime() - new Date(a.queued_at).getTime())[0];
        const map: Record<string, SourceOutcome> = {};
        for (const c of latest?.coverage ?? []) map[c.source] = c.outcome;
        setOutcomes(map);
        setListed(new Set(exposures.exposures.filter((e) => e.is_active).map((e) => e.source_name)));
      } catch {
        // The guide is still useful without scan data.
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  // The California registry is a large file, so it is fetched after the page is up.
  useEffect(() => {
    let cancelled = false;
    import("@/lib/ca-brokers.generated")
      .then((m) => {
        if (!cancelled) setRegistry({ brokers: m.CA_BROKERS, source: m.CA_BROKERS_SOURCE });
      })
      .catch(() => {
        if (!cancelled) setRegistryFailed(true);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const owners = useMemo(() => (registry ? ownersBySite(registry.brokers) : null), [registry]);

  async function reloadTracking() {
    const [e, r] = await Promise.all([listExposures(), listRemovalRequests()]);
    setExposures(e.exposures);
    setRequests(r.requests);
    setListed(new Set(e.exposures.filter((x) => x.is_active).map((x) => x.source_name)));
  }

  async function handleAddListing(ev: React.FormEvent) {
    ev.preventDefault();
    setAdding(true);
    setAddMessage(null);
    try {
      const r = await addOwnListing(newUrl.trim());
      setAddMessage({ ok: true, text: r.message });
      setNewUrl("");
      await reloadTracking();
    } catch (e) {
      setAddMessage({ ok: false, text: e instanceof Error ? e.message : "Could not add the listing." });
    } finally {
      setAdding(false);
    }
  }

  const guidedSites = useMemo(() => new Set(BROKER_OPT_OUTS.map((b) => b.name)), []);

  /** Prefill the "track a listing" form for a site, and take the customer there. */
  function trackOn(site: string) {
    setNewUrl(`https://${site}/`);
    setAddMessage(null);
    const form = document.getElementById("add-listing");
    form?.scrollIntoView({ behavior: "smooth", block: "center" });
    window.setTimeout(() => document.getElementById("listing-url")?.focus(), 300);
  }

  // Four separate figures, never added together: catalogued is not guided, guided is not
  // automated, and none of them is a verified removal.
  const guided = BROKER_OPT_OUTS.filter((b) => b.optOutUrl || b.optOutEmail).length;
  const readableNow = publicCoverage && publicCoverage.scans > 0 ? publicCoverage.sources.filter((s) => s.readable).length : null;
  const verifiedSites = publicCoverage?.verified_sources.length ?? null;

  function handleSignOut() {
    void logout().finally(() => router.push("/"));
  }

  // Sites where you are listed first, then the ones we could not check, then the rest.
  const rank = (b: BrokerOptOut) =>
    listed.has(b.name) ? 0 : outcomes[b.name] && outcomes[b.name] !== "checked" ? 1 : outcomes[b.name] ? 3 : 2;
  const brokers = [...BROKER_OPT_OUTS].sort((a, b) => rank(a) - rank(b) || a.name.localeCompare(b.name));

  return (
    <div className="min-h-screen">
      <nav className="border-b border-gray-800 px-6 py-4 flex items-center justify-between">
        <div className="flex items-center gap-2">
          <Shield className="text-teal-400 w-5 h-5" aria-hidden="true" />
          <span className="text-white font-bold">VectorScrub</span>
        </div>
        <div className="flex items-center gap-4">
          <button
            onClick={() => router.push("/dashboard")}
            className="text-gray-400 hover:text-white text-sm font-medium transition-colors"
          >
            Dashboard
          </button>
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

      <main className="max-w-4xl mx-auto px-6 py-10">
        <h1 className="text-2xl font-bold text-white mb-2">Manual opt-out guide</h1>
        <p className="text-gray-400 text-sm mb-8">
          Many data brokers block automated tools, so we cannot scan or remove you from them
          automatically. Their own opt-out pages are free to use and take a few minutes each. Have your
          name, your current address and an email address ready; most sites ask for the link to your
          listing and send a confirmation email.
        </p>

        <section className="grid grid-cols-2 sm:grid-cols-4 gap-3 mb-6" aria-label="What this guide covers" data-testid="coverage-metrics">
          {[
            {
              n: registry ? `${registry.source.companies}` : "\u2026",
              label: "companies catalogued",
              hint: registry ? `${registry.source.sites} sites, from California's registry. A directory, not removal support.` : "California's registry is loading.",
            },
            { n: `${guided}`, label: "sites with step-by-step opt-outs", hint: "Instructions we maintain and link-check nightly." },
            {
              n: readableNow === null ? "\u2013" : `${readableNow}`,
              label: "sites our scans can read now",
              hint: readableNow === null ? "Not measured yet." : `Based on scans in the last ${publicCoverage?.window_days ?? 30} days.`,
            },
            {
              n: verifiedSites === null ? "\u2013" : `${verifiedSites}`,
              label: "sites with a verified removal",
              hint:
                verifiedSites === 0
                  ? "None yet. We count only removals our own check confirmed."
                  : "Sites where our own check confirmed a listing gone.",
            },
          ].map((m) => (
            <div key={m.label} className="bg-gray-900 border border-gray-800 rounded-xl p-3">
              <div className="text-white text-xl font-semibold">{m.n}</div>
              <div className="text-gray-300 text-xs">{m.label}</div>
              <div className="text-gray-500 text-xs mt-1">{m.hint}</div>
            </div>
          ))}
        </section>

        <section className="bg-gray-900 border border-gray-800 rounded-xl p-5 mb-6" aria-labelledby="how-heading">
          <h2 id="how-heading" className="text-white font-semibold text-sm mb-2">How a manual opt-out works here</h2>
          <ol className="list-decimal ml-5 text-sm text-gray-300 space-y-1">
            <li><strong className="text-white">Find your listing</strong> on the site and copy its address.</li>
            <li><strong className="text-white">Submit the opt-out</strong> on the site&apos;s own page, using the steps below.</li>
            <li><strong className="text-white">Confirm completion:</strong> click any confirmation email, then tell us on the Removals page.</li>
            <li><strong className="text-white">We follow up:</strong> we re-check the listing on a schedule and record whether it is still there, gone, or inconclusive.</li>
          </ol>
          <p className="text-gray-500 text-xs mt-2">
            What you tell us is recorded as your report. Only our own re-check can mark a listing removed.
          </p>
        </section>

        <section className="bg-gray-900 border border-gray-800 rounded-xl p-5 mb-8" aria-labelledby="add-heading" data-testid="add-listing" id="add-listing">
          <h2 id="add-heading" className="text-white font-semibold text-sm mb-1">Found yourself on a site we did not scan?</h2>
          <p className="text-gray-400 text-sm mb-3">
            Paste the link to your listing (the page about you, not the site&apos;s home page) and we will track it:
            the steps, your report, and our scheduled re-checks.
            We encrypt the link and only visit that page.
          </p>
          <form onSubmit={handleAddListing} className="flex flex-col sm:flex-row gap-2">
            <label htmlFor="listing-url" className="sr-only">Link to your listing</label>
            <input
              id="listing-url"
              type="url"
              inputMode="url"
              required
              placeholder="https://www.example.com/your-listing"
              value={newUrl}
              onChange={(e) => setNewUrl(e.target.value)}
              className="flex-1 bg-gray-800 border border-gray-700 rounded-lg px-3 py-2 text-white text-sm focus:outline-none focus:border-teal-500"
            />
            <button
              type="submit"
              disabled={adding || !newUrl.trim()}
              className="px-4 py-2 rounded-lg bg-teal-600 hover:bg-teal-500 disabled:opacity-50 text-white text-sm"
            >
              {adding ? "Adding\u2026" : "Track this listing"}
            </button>
          </form>
          {addMessage && (
            <p role="status" className={`text-xs mt-2 ${addMessage.ok ? "text-teal-300" : "text-red-400"}`}>
              {addMessage.text}
              {addMessage.ok && (
                <>
                  {" "}
                  <a href="/dashboard/removals" className="underline">Go to Removals</a>
                </>
              )}
            </p>
          )}
        </section>

        {/* California DROP: a California-residents-only service */}
        {residence !== null && residence !== "CA" ? (
          <p className="text-gray-500 text-xs mb-8" data-testid="drop-not-eligible">
            California&apos;s free DROP service, which sends one deletion request to every registered data
            broker, is only for California residents. Your saved address is in another state, so it is not
            shown here. Your rights depend on where you live; the company pages below are open to anyone.
          </p>
        ) : (
          <section
            className="bg-gray-900 border border-gray-800 rounded-xl p-5 mb-8"
            aria-labelledby="drop-heading"
            data-testid="drop-card"
          >
            <h2 id="drop-heading" className="text-white font-semibold text-sm mb-1">
              {residence === "CA"
                ? "You are in California: one free request covers every registered broker"
                : "Live in California? One free request covers every registered broker"}
            </h2>
            <p className="text-gray-400 text-sm mb-3">
              California&apos;s Delete Request and Opt-out Platform (DROP) lets California residents send a single
              deletion request to all registered data brokers. It is a free government service and{" "}
              <span className="text-gray-200">only California residents can use it</span>. Brokers must start
              processing requests from August 1, 2026, and it can take up to 90 days for a status to update.
              VectorScrub does not submit this for you; you do it yourself on the state&apos;s site.
            </p>
            <a
              href={DROP_URL}
              target="_blank"
              rel="noopener noreferrer"
              className="inline-flex items-center gap-1.5 text-teal-400 hover:text-teal-300 text-sm font-medium"
            >
              Go to California DROP <ExternalLink className="w-3.5 h-3.5" aria-hidden="true" />
            </a>
          </section>
        )}

        <div className="flex items-start gap-2 text-gray-500 text-xs mb-4">
          <Info className="w-4 h-4 flex-shrink-0 mt-0.5" aria-hidden="true" />
          <span>
            Every night we check that each site&apos;s opt-out page still opens. That only tells you the link is
            alive: it does not mean an automated opt-out works there, and some sites treat our check
            differently from a person in a browser. If a link does not work, search for the site&apos;s name
            plus &quot;opt out&quot;.
          </span>
        </div>

        {loading ? (
          <div className="text-gray-400 text-sm animate-pulse">Loading…</div>
        ) : (
          <ul className="space-y-3">
            {brokers.map((b) => {
              const outcome = outcomes[b.name];
              const page = health[b.name];
              const company = owners?.get(b.name);
              const exposure = exposures.find((e) => e.source_name === b.name && e.is_active) ?? exposures.find((e) => e.source_name === b.name);
              const request = exposure ? latestRequest(exposure.id, requests) : undefined;
              const route = routeStatus(page, outcome, Date.now(), isAutomationBlocked(request?.failure_reason));
              const track = tracker({
                listed: listed.has(b.name),
                addedByCustomer: exposure?.origin === "customer",
                scan: outcome,
                request,
              });
              return (
                <li key={b.name} id={`broker-${b.name}`} className="bg-gray-900 border border-gray-800 rounded-xl p-5 scroll-mt-6">
                  <div className="flex flex-wrap items-center gap-2 mb-1">
                    <h3 className="text-white font-medium text-sm">{b.name}</h3>
                    {listed.has(b.name) && (
                      <span className="text-xs px-2 py-0.5 rounded-full border text-red-300 border-red-800 bg-red-950">
                        You are listed here
                      </span>
                    )}
                    <span
                      className={`text-xs px-2 py-0.5 rounded-full border ${ROUTE_STATUS[route].style}`}
                      title={ROUTE_STATUS[route].meaning}
                    >
                      {ROUTE_STATUS[route].text}
                    </span>
                  </div>
                  <p className="text-gray-500 text-xs mb-2">{ROUTE_STATUS[route].meaning}</p>
                  <div className="border border-gray-800 rounded-lg p-3 mb-3" data-testid={`tracker-${b.name}`}>
                    <div className="text-sm text-white font-medium">{track.headline}</div>
                    <ul className="mt-1 space-y-0.5 text-xs">
                      {track.steps.map((st) => (
                        <li key={st.label} className={STEP_STYLE[st.state]}>
                          <span aria-hidden="true" className="inline-block w-4">{STEP_MARK[st.state]}</span>
                          {st.label}: <span className="text-gray-300">{st.detail}</span>
                        </li>
                      ))}
                    </ul>
                    <p className="text-xs text-gray-300 mt-2">
                      <span className="text-gray-500">Next: </span>
                      {track.nextAction}
                      {request && (request.status === "failed" || request.status === "submitted") && (
                        <>
                          {" "}
                          <a href="/dashboard/removals" className="text-teal-400 underline">Removals page</a>
                        </>
                      )}
                    </p>
                  </div>
                  {page && (
                    <p className="flex flex-wrap items-center gap-2 text-xs mb-1">
                      {!isStale(page) && page.status === "captcha" && (
                        <span className={`px-2 py-0.5 rounded-full border ${PAGE_STATUS.captcha.style}`}>
                          {PAGE_STATUS.captcha.text}
                        </span>
                      )}
                      <span className="text-gray-500">Opt-out page
                        checked {formatDate(page.checked_at)}
                        {page.status !== "reachable" && page.status !== "captcha" && page.last_reachable_at
                          ? ` · last opened ${formatDate(page.last_reachable_at)}`
                          : ""}
                      </span>
                    </p>
                  )}
                  <p className="text-gray-400 text-sm">{b.instructions}</p>
                  {company && (
                    <SiteNetwork
                      company={company}
                      current={b.name}
                      guided={guidedSites}
                      exposures={exposures}
                      requests={requests}
                      onTrack={trackOn}
                    />
                  )}
                  <div className="flex flex-wrap items-center gap-x-4 gap-y-1 mt-3 text-sm">
                    {b.method === "email" && b.optOutEmail ? (
                      <a
                        href={`mailto:${b.optOutEmail}?subject=${encodeURIComponent("Opt-Out Request")}`}
                        className="inline-flex items-center gap-1.5 text-teal-400 hover:text-teal-300 font-medium"
                      >
                        <Mail className="w-3.5 h-3.5" aria-hidden="true" /> Email {b.optOutEmail}
                      </a>
                    ) : (
                      b.optOutUrl && (
                        <a
                          href={b.optOutUrl}
                          target="_blank"
                          rel="noopener noreferrer"
                          className="inline-flex items-center gap-1.5 text-teal-400 hover:text-teal-300 font-medium"
                        >
                          Open opt-out page <ExternalLink className="w-3.5 h-3.5" aria-hidden="true" />
                        </a>
                      )
                    )}
                    <span className="text-gray-500">Usually takes about {b.estimatedDays} {b.estimatedDays === 1 ? "day" : "days"}</span>
                    {b.requiresIdVerification && (
                      <span className="text-gray-500">May ask you to verify your identity</span>
                    )}
                  </div>
                </li>
              );
            })}
          </ul>
        )}

        <CaRegistrySearch
          data={registry}
          failed={registryFailed}
          pageHealth={registryHealth}
          ourSites={new Set(BROKER_OPT_OUTS.map((b) => b.name))}
        />
      </main>
    </div>
  );
}
