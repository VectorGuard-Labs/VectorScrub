"use client";

import { useMemo, useState } from "react";
import { ExternalLink, Search } from "lucide-react";
import type { CaBroker } from "@/lib/ca-brokers.generated";
import { deletionSummary, safeHref, searchBrokers } from "@/lib/caBrokerSearch";

const PAGE = 25;

export interface RegistryData {
  brokers: CaBroker[];
  source: { label: string; importedOn: string; companies: number; sites: number };
}

/**
 * Every company in the California Data Broker Registry, searchable by company or by the site it
 * runs, each with the page where the company says to send privacy requests. The page loads the
 * data on demand and hands it in, so the rest of the guide does not carry it.
 */
export default function CaRegistrySearch({
  data,
  failed,
  ourSites,
}: {
  data: RegistryData | null;
  failed: boolean;
  ourSites: Set<string>;
}) {
  const [query, setQuery] = useState("");
  const [shown, setShown] = useState(PAGE);

  const results = useMemo(() => (data ? searchBrokers(data.brokers, query) : []), [data, query]);

  if (failed) {
    return <p className="text-gray-500 text-sm">The registry could not be loaded. Please try again later.</p>;
  }
  if (!data) {
    return <p className="text-gray-400 text-sm animate-pulse">Loading the registry…</p>;
  }

  return (
    <section aria-labelledby="registry-heading" className="mt-12">
      <h2 id="registry-heading" className="text-white font-semibold text-lg mb-1">
        Every registered data broker
      </h2>
      <p className="text-gray-400 text-sm mb-4">
        California requires data brokers to register. This is that list: {data.source.companies} companies
        running {data.source.sites} sites. We do not scan these sites for you (most are not people-search
        sites), but each company names a page for privacy requests, and you can use it. Search by company
        or by a site you found yourself on.
      </p>

      <label htmlFor="registry-search" className="sr-only">
        Search registered data brokers by company or site
      </label>
      <div className="relative mb-2">
        <Search className="w-4 h-4 text-gray-500 absolute left-3 top-3" aria-hidden="true" />
        <input
          id="registry-search"
          type="search"
          value={query}
          onChange={(e) => {
            setQuery(e.target.value);
            setShown(PAGE);
          }}
          placeholder="Company or site, for example zabasearch.com"
          className="w-full bg-gray-900 border border-gray-700 text-white text-sm rounded-lg pl-9 pr-3 py-2.5 focus:ring-2 focus:ring-teal-600 focus:outline-none"
          autoComplete="off"
          spellCheck={false}
        />
      </div>
      <p className="text-gray-500 text-xs mb-4" role="status" aria-live="polite">
        {query.trim()
          ? `${results.length} ${results.length === 1 ? "company matches" : "companies match"}`
          : `${results.length} companies`}
      </p>

      <ul className="space-y-3">
        {results.slice(0, shown).map((b) => {
          const links = b.rights.map((r) => safeHref(r)).filter((h): h is string => h !== null);
          const stats = deletionSummary(b);
          const ours = b.sites.filter((s) => ourSites.has(s));
          return (
            <li key={b.id} className="bg-gray-900 border border-gray-800 rounded-xl p-5">
              <div className="flex flex-wrap items-center gap-2 mb-1">
                <h3 className="text-white font-medium text-sm">{b.name}</h3>
                {b.dba && <span className="text-gray-500 text-xs">({b.dba})</span>}
                {ours.length > 0 && (
                  <span className="text-xs px-2 py-0.5 rounded-full border text-teal-300 border-teal-800 bg-teal-950">
                    Runs {ours.length === 1 ? "a site" : "sites"} in our guide
                  </span>
                )}
                {b.fcra && (
                  <span className="text-xs px-2 py-0.5 rounded-full border text-gray-300 border-gray-700 bg-gray-800">
                    Regulated by the FCRA
                  </span>
                )}
                {!b.us && (
                  <span className="text-xs px-2 py-0.5 rounded-full border text-gray-300 border-gray-700 bg-gray-800">
                    Based outside the US
                  </span>
                )}
              </div>
              <p className="text-gray-400 text-xs mb-2 break-words">
                {b.sites.length === 1 ? "Site: " : `Runs ${b.sites.length} sites: `}
                {b.sites.slice(0, 8).join(", ")}
                {b.sites.length > 8 ? `, and ${b.sites.length - 8} more` : ""}
              </p>
              {stats && <p className="text-gray-500 text-xs mb-2">{stats} Self-reported, not audited.</p>}
              <div className="flex flex-wrap gap-x-4 gap-y-1 text-sm">
                {links.length === 0 ? (
                  <span className="text-gray-500">No privacy-request page listed</span>
                ) : (
                  links.map((href, i) => (
                    <a
                      key={href}
                      href={href}
                      target="_blank"
                      rel="noopener noreferrer"
                      className="inline-flex items-center gap-1.5 text-teal-400 hover:text-teal-300 font-medium break-all"
                    >
                      {i === 0 ? "Privacy-request page" : "Another page"}
                      <ExternalLink className="w-3.5 h-3.5 flex-shrink-0" aria-hidden="true" />
                    </a>
                  ))
                )}
              </div>
            </li>
          );
        })}
      </ul>

      {shown < results.length && (
        <button
          onClick={() => setShown((n) => n + PAGE)}
          className="mt-4 text-teal-400 hover:text-teal-300 text-sm font-medium"
        >
          Show {Math.min(PAGE, results.length - shown)} more
        </button>
      )}
      {results.length === 0 && (
        <p className="text-gray-500 text-sm mt-2">
          No registered company runs that site. It may not be registered, or may be listed under a different name.
        </p>
      )}

      <p className="text-gray-600 text-xs mt-6">
        Source: {data.source.label}, imported {data.source.importedOn}. Figures about deletion requests are
        the companies&apos; own reports. The registry changes; use the company&apos;s page for current steps.
      </p>
    </section>
  );
}
