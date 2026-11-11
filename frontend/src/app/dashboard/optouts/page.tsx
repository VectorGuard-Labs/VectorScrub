"use client";

import { useEffect, useMemo, useState } from "react";
import { useRouter } from "next/navigation";
import { Shield, LogOut, ExternalLink, Mail, Info } from "lucide-react";
import { getOptOutHealth, listExposures, listScans, logout } from "@/lib/api";
import { formatDate } from "@/lib/utils";
import { BROKER_OPT_OUTS, type BrokerOptOut } from "@/lib/brokers.generated";
import CaRegistrySearch, { type RegistryData } from "@/components/CaRegistrySearch";
import { ownersBySite } from "@/lib/caBrokerSearch";
import type { OptOutHealth, OptOutPageStatus, SourceOutcome } from "@/types";

// California's own free deletion service for residents. It is run by the state, not by us.
const DROP_URL = "https://consumer.drop.privacy.ca.gov/";

// A check older than this is shown as stale rather than as current.
const STALE_AFTER_DAYS = 3;

const PAGE_STATUS: Record<OptOutPageStatus, { text: string; style: string }> = {
  reachable: { text: "Page opens", style: "text-green-400 border-green-800 bg-green-950" },
  captcha: { text: "Page opens, form has a CAPTCHA", style: "text-yellow-300 border-yellow-800 bg-yellow-950" },
  blocked: { text: "Blocked our check", style: "text-orange-300 border-orange-800 bg-orange-950" },
  not_found: { text: "Link not working", style: "text-red-300 border-red-800 bg-red-950" },
  unavailable: { text: "Could not check", style: "text-orange-300 border-orange-800 bg-orange-950" },
};

function isStale(h: OptOutHealth): boolean {
  return Date.now() - new Date(h.checked_at).getTime() > STALE_AFTER_DAYS * 86_400_000;
}

const OUTCOME_LABEL: Record<SourceOutcome, { text: string; style: string }> = {
  checked: { text: "Checked in last scan", style: "text-green-400 border-green-800 bg-green-950" },
  blocked: { text: "Blocked our scan", style: "text-orange-300 border-orange-800 bg-orange-950" },
  not_found: { text: "Could not check", style: "text-orange-300 border-orange-800 bg-orange-950" },
  unavailable: { text: "Could not check", style: "text-orange-300 border-orange-800 bg-orange-950" },
  unreadable: { text: "Could not check", style: "text-orange-300 border-orange-800 bg-orange-950" },
  not_supported: { text: "Search it yourself", style: "text-orange-300 border-orange-800 bg-orange-950" },
};

export default function OptOutsPage() {
  const router = useRouter();
  const [outcomes, setOutcomes] = useState<Record<string, SourceOutcome>>({});
  const [listed, setListed] = useState<Set<string>>(new Set());
  const [health, setHealth] = useState<Record<string, OptOutHealth>>({});
  const [registry, setRegistry] = useState<RegistryData | null>(null);
  const [registryFailed, setRegistryFailed] = useState(false);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        // The page check is a nice-to-have: the guide works without it.
        const [scans, exposures, checks] = await Promise.all([
          listScans(),
          listExposures(),
          getOptOutHealth().catch(() => [] as OptOutHealth[]),
        ]);
        if (cancelled) return;
        setHealth(Object.fromEntries(checks.map((c) => [c.source, c])));
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

        {/* California DROP */}
        <section className="bg-gray-900 border border-gray-800 rounded-xl p-5 mb-8" aria-labelledby="drop-heading">
          <h2 id="drop-heading" className="text-white font-semibold text-sm mb-1">
            Live in California? One free request covers every registered broker
          </h2>
          <p className="text-gray-400 text-sm mb-3">
            California&apos;s Delete Request and Opt-out Platform (DROP) lets residents send a single deletion
            request to all registered data brokers. It is a free government service. Brokers must start
            processing requests from August 1, 2026, and it can take up to 90 days for a status to
            update. VectorScrub does not submit this for you; you do it yourself on the state&apos;s site.
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
              const siblings = company ? company.sites.filter((s) => s !== b.name) : [];
              return (
                <li key={b.name} className="bg-gray-900 border border-gray-800 rounded-xl p-5">
                  <div className="flex flex-wrap items-center gap-2 mb-1">
                    <h3 className="text-white font-medium text-sm">{b.name}</h3>
                    {listed.has(b.name) && (
                      <span className="text-xs px-2 py-0.5 rounded-full border text-red-300 border-red-800 bg-red-950">
                        You are listed here
                      </span>
                    )}
                    {outcome && (
                      <span className={`text-xs px-2 py-0.5 rounded-full border ${OUTCOME_LABEL[outcome].style}`}>
                        {OUTCOME_LABEL[outcome].text}
                      </span>
                    )}
                  </div>
                  {page && (
                    <p className="flex flex-wrap items-center gap-2 text-xs mb-1">
                      {isStale(page) ? (
                        <span className="px-2 py-0.5 rounded-full border text-gray-400 border-gray-700 bg-gray-800">
                          Not checked recently
                        </span>
                      ) : (
                        <span className={`px-2 py-0.5 rounded-full border ${PAGE_STATUS[page.status].style}`}>
                          {PAGE_STATUS[page.status].text}
                        </span>
                      )}
                      <span className="text-gray-500">
                        Checked {formatDate(page.checked_at)}
                        {page.status !== "reachable" && page.status !== "captcha" && page.last_reachable_at
                          ? ` · last opened ${formatDate(page.last_reachable_at)}`
                          : ""}
                      </span>
                    </p>
                  )}
                  <p className="text-gray-400 text-sm">{b.instructions}</p>
                  {company && (
                    <p className="text-gray-500 text-xs mt-1">
                      Run by {company.name}
                      {siblings.length > 0
                        ? `, which also runs ${siblings.slice(0, 4).join(", ")}${
                            siblings.length > 4 ? ` and ${siblings.length - 4} more` : ""
                          }.`
                        : "."}
                    </p>
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
                    <span className="text-gray-500">Usually takes about {b.estimatedDays} days</span>
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
          ourSites={new Set(BROKER_OPT_OUTS.map((b) => b.name))}
        />
      </main>
    </div>
  );
}
