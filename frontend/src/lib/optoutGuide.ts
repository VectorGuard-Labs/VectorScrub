import type { OptOutHealth, RemovalRequest, SourceOutcome } from "@/types";
import { isStale } from "@/lib/optoutHealth";

/**
 * One label per broker for "can you use this route right now", from what we actually measured.
 * None of them says a removal will work: "Accessible" only means the opt-out page opened.
 */
export type RouteStatus = "accessible" | "manual_required" | "user_action" | "unavailable" | "unverified";

export const ROUTE_STATUS: Record<RouteStatus, { text: string; meaning: string; style: string }> = {
  accessible: {
    text: "Opt-out page accessible",
    meaning: "The opt-out page opened at our last check. That does not mean the site accepts or completes the removal.",
    style: "text-green-400 border-green-800 bg-green-950",
  },
  manual_required: {
    text: "Manual required",
    meaning: "The site blocks our automated tools, so you submit the opt-out yourself.",
    style: "text-orange-300 border-orange-800 bg-orange-950",
  },
  user_action: {
    text: "Find your listing yourself",
    meaning: "We cannot search this site for you. Search it in your browser, then add the listing below to track it.",
    style: "text-orange-300 border-orange-800 bg-orange-950",
  },
  unavailable: {
    text: "Opt-out page unavailable",
    meaning: "The opt-out page did not open at our last check. Try again later, or search for the site's name plus \u201copt out\u201d.",
    style: "text-red-300 border-red-800 bg-red-950",
  },
  unverified: {
    text: "Unverified",
    meaning: "We have not recently confirmed this opt-out page. The steps may have changed.",
    style: "text-gray-300 border-gray-700 bg-gray-800",
  },
};

/**
 * Precedence: a dead opt-out page matters most, then "we cannot search it for you", then "it blocks
 * us", then a live page. No recent check at all is "unverified".
 */
export function routeStatus(
  page: OptOutHealth | undefined,
  scan: SourceOutcome | undefined,
  now: number = Date.now(),
  /** Our own removal attempt for this customer was refused by the site. Outranks the nightly
   *  page check, which uses a plain request that some sites treat differently from our browser. */
  removalBlocked: boolean = false
): RouteStatus {
  const fresh = page && !isStale(page, now) ? page : undefined;
  if (fresh?.status === "not_found" || fresh?.status === "unavailable") return "unavailable";
  if (removalBlocked) return "manual_required";
  if (scan === "not_supported") return "user_action";
  if (scan === "blocked" || fresh?.status === "blocked" || fresh?.status === "captcha") return "manual_required";
  if (fresh?.status === "reachable") return "accessible";
  return "unverified";
}

export type StepState = "done" | "pending" | "not_applicable" | "unknown";

export interface TrackerStep {
  label: string;
  state: StepState;
  detail: string;
}

export interface Tracker {
  /** Short headline for where this broker stands for this customer. */
  headline: string;
  steps: TrackerStep[];
  nextAction: string;
}

/**
 * The customer's own progress on one broker, from their listing and removal request. A report by
 * the customer and a verification by us are separate steps, and only the latter can say "removed".
 */
export function tracker(opts: {
  listed: boolean;
  addedByCustomer: boolean;
  scan: SourceOutcome | undefined;
  request: RemovalRequest | undefined;
}): Tracker {
  const { listed, addedByCustomer, scan, request: r } = opts;

  const detected: TrackerStep = listed
    ? { label: "Listing", state: "done", detail: addedByCustomer ? "Added by you" : "Found by our scan" }
    : scan === "checked"
      ? { label: "Listing", state: "not_applicable", detail: "Our last scan did not find you here" }
      : { label: "Listing", state: "unknown", detail: "Not checked: search the site yourself" };

  if (!listed || !r) {
    return {
      headline: listed ? "Waiting for your decision" : scan === "checked" ? "Not found in our last scan" : "Not checked",
      steps: [detected],
      nextAction: listed
        ? "Authorize the removal on your dashboard, or follow the steps below and tell us on the Removals page."
        : "If you find yourself here, add the listing below so we can track it and re-check it.",
    };
  }

  const reported = r.manual_reported_at !== null || (r.submitted_at !== null && r.procedure_used !== "manual");
  const submitted: TrackerStep =
    r.procedure_used === "manual" && r.manual_reported_at
      ? { label: "Opt-out submitted", state: "done", detail: "Reported by you (not verified)" }
      : r.submitted_at
        ? { label: "Opt-out submitted", state: "done", detail: "Sent by VectorScrub" }
        : r.status === "authorized"
          ? { label: "Opt-out submitted", state: "pending", detail: "Queued: we will try to send it" }
          : { label: "Opt-out submitted", state: "pending", detail: "Not yet reported" };

  const email: TrackerStep =
    r.manual_email_confirmation === "confirmed"
      ? { label: "Email confirmation", state: "done", detail: "You clicked the site's link" }
      : r.manual_email_confirmation === "not_required"
        ? { label: "Email confirmation", state: "not_applicable", detail: "Not required, as you reported" }
        : { label: "Email confirmation", state: "pending", detail: "If the site emails you, click the link" };

  const verified: TrackerStep =
    r.status === "verified_removed"
      ? { label: "Independent verification", state: "done", detail: "Our check found the listing gone" }
      : r.last_check_result === "still_present"
        ? { label: "Independent verification", state: "pending", detail: "Last check still saw the listing" }
        : r.last_check_result === "inconclusive" || r.status === "unverifiable"
          ? { label: "Independent verification", state: "unknown", detail: "Inconclusive: we could not read the page" }
          : r.status === "rejected"
            ? { label: "Independent verification", state: "pending", detail: "Still listed after 45 days" }
            : { label: "Independent verification", state: "pending", detail: reported ? "Scheduled" : "Not yet performed" };

  let headline: string;
  let nextAction: string;
  if (r.status === "verified_removed") {
    headline = "Verified removed";
    nextAction = "Nothing to do. We keep watching for it to come back.";
  } else if (r.status === "failed") {
    headline = "Awaiting your action";
    nextAction = "Submit your request through the site's opt-out page, then report it on the Removals page so we can follow up.";
  } else if (r.status === "authorized") {
    headline = "Authorized";
    nextAction = "Nothing yet: we will try to send it and tell you if it needs you.";
  } else if (r.status === "rejected") {
    headline = "Still listed";
    nextAction = "Try the opt-out again or contact the site directly.";
  } else if (r.status === "unverifiable") {
    headline = "Inconclusive";
    nextAction = "We could not tell. Look at the listing yourself; if it is still there, try again.";
  } else if (r.status === "reappeared") {
    headline = "Reappeared";
    nextAction = "Authorize a new removal on your dashboard.";
  } else {
    headline = r.procedure_used === "manual" ? "Reported submitted, awaiting verification" : "Submitted, awaiting verification";
    nextAction = "Nothing to do. We re-check on a schedule (about 3, 7, 14 and 30 days after submission).";
  }
  return { headline, steps: [detected, submitted, email, verified], nextAction };
}
