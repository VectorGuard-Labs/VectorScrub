"use client";

import { useState } from "react";
import { ExternalLink } from "lucide-react";
import type { CaBroker } from "@/lib/ca-brokers.generated";
import type { Exposure, RemovalRequest } from "@/types";
import { latestRequest } from "@/lib/report";
import { safeHref } from "@/lib/caBrokerSearch";

/** How many sites to show before "Show all". */
const COLLAPSED = 8;

/** Where the customer stands on one site of the network, from their own listings only. */
function siteState(site: string, exposures: Exposure[], requests: RemovalRequest[]): { text: string; style: string } | null {
  const mine = exposures.filter((e) => e.source_name === site);
  if (mine.length === 0) return null;
  const req = mine.map((e) => latestRequest(e.id, requests)).find(Boolean);
  if (req?.status === "verified_removed") return { text: "Removal verified", style: "text-green-400 border-green-800 bg-green-950" };
  if (req && (req.status === "submitted" || req.status === "acknowledged")) {
    return { text: "Awaiting verification", style: "text-blue-300 border-blue-800 bg-blue-950" };
  }
  if (mine.some((e) => e.is_active)) return { text: "You are listed here", style: "text-red-300 border-red-800 bg-red-950" };
  return { text: "Removed", style: "text-green-400 border-green-800 bg-green-950" };
}

/**
 * Every site one company runs, as a checklist. One company's sites often share the same records,
 * but an opt-out on one site does not, as far as we know, cover the others, so each needs a look.
 * Site names come from the company's own filing with California's registry.
 */
export default function SiteNetwork({
  company,
  current,
  guided,
  exposures,
  requests,
  onTrack,
}: {
  company: CaBroker;
  current: string;
  /** Sites that have their own card (steps) in this guide. */
  guided: Set<string>;
  exposures: Exposure[];
  requests: RemovalRequest[];
  /** Prefill the "track a listing" form with this site. */
  onTrack: (site: string) => void;
}) {
  const [expanded, setExpanded] = useState(false);
  const others = company.sites.filter((s) => s !== current);
  if (others.length === 0) {
    return <p className="text-gray-500 text-xs mt-1">Run by {company.name}.</p>;
  }
  const sites = [current, ...others];
  const shown = expanded ? sites : sites.slice(0, COLLAPSED);

  return (
    <details className="mt-2 text-xs" data-testid={`network-${current}`}>
      <summary className="cursor-pointer text-gray-400 hover:text-gray-200">
        Run by {company.name}, which runs {sites.length} sites. Check each one: an opt-out here may not cover the others.
      </summary>
      <ul className="mt-2 space-y-1.5">
        {shown.map((site) => {
          const href = safeHref(`https://${site}`);
          const state = siteState(site, exposures, requests);
          return (
            <li key={site} className="flex flex-wrap items-center gap-x-3 gap-y-1">
              <span className="text-gray-200 font-medium min-w-[10rem]">{site}</span>
              {site === current && <span className="text-gray-500">(this card)</span>}
              {state && <span className={`px-2 py-0.5 rounded-full border ${state.style}`}>{state.text}</span>}
              {href && (
                <a
                  href={href}
                  target="_blank"
                  rel="noopener noreferrer"
                  className="inline-flex items-center gap-1 text-teal-400 hover:text-teal-300"
                >
                  Search this site <ExternalLink className="w-3 h-3" aria-hidden="true" />
                </a>
              )}
              {site !== current && guided.has(site) && (
                <a href={`#broker-${site}`} className="text-teal-400 hover:text-teal-300 underline">
                  Opt-out steps
                </a>
              )}
              {site !== current && !guided.has(site) && !state && (
                <button type="button" onClick={() => onTrack(site)} className="text-gray-300 hover:text-white underline">
                  Found yourself? Track it
                </button>
              )}
            </li>
          );
        })}
      </ul>
      {sites.length > COLLAPSED && (
        <button type="button" onClick={() => setExpanded(!expanded)} className="mt-2 text-gray-300 underline">
          {expanded ? "Show fewer" : `Show all ${sites.length} sites`}
        </button>
      )}
      <p className="text-gray-500 mt-2">
        Sites listed by the company in its filing with California&apos;s data broker registry. Not every site
        publishes people listings. Where there are no steps here, look for an &quot;opt out&quot; or
        &quot;privacy&quot; link on the site.
      </p>
    </details>
  );
}
