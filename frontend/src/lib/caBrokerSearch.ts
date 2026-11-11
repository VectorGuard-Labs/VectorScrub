import type { CaBroker } from "@/lib/ca-brokers.generated";

/** The company a site belongs to, for "who runs this site?" lookups. */
export function ownersBySite(brokers: CaBroker[]): Map<string, CaBroker> {
  const owners = new Map<string, CaBroker>();
  for (const b of brokers) {
    for (const site of b.sites) if (!owners.has(site)) owners.set(site, b);
  }
  return owners;
}

/** Lowercase, without a scheme, "www." or path, so "https://www.ZabaSearch.com/x" finds "zabasearch.com". */
export function normalizeQuery(raw: string): string {
  return raw
    .trim()
    .toLowerCase()
    .replace(/^[a-z][a-z0-9+.-]*:\/\//, "")
    .replace(/^www\./, "")
    .split(/[/?#]/)[0];
}

/**
 * Companies matching a search: by company name, DBA, or any site they run. Exact site matches
 * come first, then names that start with the query, then everything else alphabetically
 * (the input is already sorted by name, which the sort below keeps stable).
 */
export function searchBrokers(brokers: CaBroker[], raw: string): CaBroker[] {
  const q = normalizeQuery(raw);
  if (!q) return brokers;
  const scored: { b: CaBroker; rank: number }[] = [];
  for (const b of brokers) {
    const name = b.name.toLowerCase();
    const dba = (b.dba ?? "").toLowerCase();
    let rank = -1;
    if (b.sites.includes(q)) rank = 0;
    else if (name.startsWith(q) || dba.startsWith(q)) rank = 1;
    else if (name.includes(q) || dba.includes(q)) rank = 2;
    else if (b.sites.some((s) => s.includes(q))) rank = 3;
    if (rank >= 0) scored.push({ b, rank });
  }
  return scored.sort((a, c) => a.rank - c.rank).map((s) => s.b);
}

/** Only ordinary web links are ever rendered, whatever the data says. */
export function safeHref(url: string): string | null {
  try {
    const u = new URL(url);
    return u.protocol === "https:" || u.protocol === "http:" ? u.toString() : null;
  } catch {
    return null;
  }
}

/** Below this many requests, a company's own compliance figure says little. */
export const MIN_REQUESTS_FOR_RATE = 20;

/**
 * The company's own report of what it did with deletion requests in 2024, as a sentence, or
 * null when there is nothing meaningful to say. These are self-reported and not audited.
 */
export function deletionSummary(b: CaBroker): string | null {
  const d = b.deletion;
  if (!d || d.received < MIN_REQUESTS_FOR_RATE) return null;
  const complied = Math.round(((d.whole + d.part) / d.received) * 100);
  const days = d.medianDays && d.medianDays > 0 ? `, median ${d.medianDays} days to respond` : "";
  return `Reports ${d.received.toLocaleString("en-US")} deletion requests in 2024; says it complied with ${complied}%${days}.`;
}
