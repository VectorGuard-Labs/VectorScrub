import type { OptOutHealth, OptOutPageStatus } from "@/types";

/** A check older than this is shown as stale rather than as current. */
export const STALE_AFTER_DAYS = 3;

/** How a page-check result is labelled and coloured. "Page opens" never means a removal works. */
export const PAGE_STATUS: Record<OptOutPageStatus, { text: string; style: string }> = {
  reachable: { text: "Opt-out page opened at last check", style: "text-green-400 border-green-800 bg-green-950" },
  captcha: { text: "Opt-out page opened; form has a CAPTCHA", style: "text-yellow-300 border-yellow-800 bg-yellow-950" },
  blocked: { text: "Blocked our check", style: "text-orange-300 border-orange-800 bg-orange-950" },
  not_found: { text: "Link not working", style: "text-red-300 border-red-800 bg-red-950" },
  unavailable: { text: "Could not check", style: "text-orange-300 border-orange-800 bg-orange-950" },
};

export function isStale(h: OptOutHealth, now: number = Date.now()): boolean {
  return now - new Date(h.checked_at).getTime() > STALE_AFTER_DAYS * 86_400_000;
}

export function byKey(checks: OptOutHealth[]): Record<string, OptOutHealth> {
  return Object.fromEntries(checks.map((c) => [c.source, c]));
}
