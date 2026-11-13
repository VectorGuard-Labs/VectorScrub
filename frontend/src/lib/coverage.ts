import { SCAN_SOURCES, type ScanSource } from "@/lib/brokers.generated";

/** The public, aggregate "which sites could we read lately" figure from GET /public/coverage. */
export interface PublicCoverage {
  window_days: number;
  /** How many recent scans the figures come from. 0 means nothing has been measured yet. */
  scans: number;
  sources: { source: string; readable: boolean }[];
  /** Sites where our own check has verified at least one removal (scan-found listings only). */
  verified_sources: string[];
  verified_removals: number;
}

export type PlanTier = "free" | "personal" | "premium";

const ORDER: Record<PlanTier, number> = { free: 0, personal: 1, premium: 2 };

/** Every source a plan scans: its own and all below it. */
export function sourcesForPlan(tier: PlanTier): ScanSource[] {
  return SCAN_SOURCES.filter((s) => ORDER[s.tier] <= ORDER[tier]);
}

export interface PlanCoverage {
  /** Brokers the plan lists. */
  listed: number;
  /** Of those, how many recent scans could read. Null until at least one scan has been measured. */
  readable: number | null;
}

/**
 * How many of a plan's brokers recent scans could actually read. A source with no recent
 * measurement is not counted as readable: the figure only ever claims what has been observed.
 */
export function planCoverage(tier: PlanTier, coverage: PublicCoverage | null): PlanCoverage {
  const sources = sourcesForPlan(tier);
  if (!coverage || coverage.scans <= 0) return { listed: sources.length, readable: null };
  const readable = new Set(coverage.sources.filter((s) => s.readable).map((s) => s.source));
  return { listed: sources.length, readable: sources.filter((s) => !s.manual && readable.has(s.name)).length };
}

/**
 * Server-side fetch of the public figure. Never throws: if the API is down or slow the page
 * simply shows its general wording instead of a number. Cached for five minutes.
 */
export async function fetchPublicCoverage(): Promise<PublicCoverage | null> {
  const base = (process.env.API_URL ?? "http://localhost:8080").replace(/\/+$/, "");
  try {
    const res = await fetch(`${base}/api/v1/public/coverage`, {
      next: { revalidate: 300 },
      signal: AbortSignal.timeout(2000),
    });
    if (!res.ok) return null;
    const body: unknown = await res.json();
    if (!body || typeof body !== "object") return null;
    const b = body as Partial<PublicCoverage>;
    if (typeof b.scans !== "number" || !Array.isArray(b.sources)) return null;
    return {
      window_days: typeof b.window_days === "number" ? b.window_days : 30,
      scans: b.scans,
      sources: b.sources.filter(
        (s): s is { source: string; readable: boolean } =>
          !!s && typeof s.source === "string" && typeof s.readable === "boolean"
      ),
      verified_sources: Array.isArray(b.verified_sources) ? b.verified_sources.filter((v): v is string => typeof v === "string") : [],
      verified_removals: typeof b.verified_removals === "number" ? b.verified_removals : 0,
    };
  } catch {
    return null;
  }
}

/** Browser-side fetch of the same public figure (same-origin, through the app's /api proxy). */
export async function getPublicCoverageClient(): Promise<PublicCoverage | null> {
  try {
    const res = await fetch("/api/v1/public/coverage", { credentials: "same-origin" });
    if (!res.ok) return null;
    const b = (await res.json()) as Partial<PublicCoverage>;
    if (typeof b.scans !== "number" || !Array.isArray(b.sources)) return null;
    return {
      window_days: typeof b.window_days === "number" ? b.window_days : 30,
      scans: b.scans,
      sources: b.sources,
      verified_sources: Array.isArray(b.verified_sources) ? b.verified_sources : [],
      verified_removals: typeof b.verified_removals === "number" ? b.verified_removals : 0,
    };
  } catch {
    return null;
  }
}
