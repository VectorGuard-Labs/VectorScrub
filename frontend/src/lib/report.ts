import type { Exposure, RemovalRequest } from "@/types";

/**
 * The outcome classes in the customer's privacy report. They are deliberately few and plain, and
 * "submitted" is never reported as "removed": only our own check can produce "Verified removed".
 */
export type ReportOutcome =
  | "verified_removed"
  | "awaiting_broker"
  | "request_submitted"
  | "customer_action"
  | "inconclusive"
  | "unsuccessful"
  | "not_authorized";

export const OUTCOME_ORDER: ReportOutcome[] = [
  "verified_removed",
  "awaiting_broker",
  "request_submitted",
  "customer_action",
  "not_authorized",
  "inconclusive",
  "unsuccessful",
];

export const OUTCOME_LABEL: Record<ReportOutcome, string> = {
  verified_removed: "Verified removed",
  awaiting_broker: "Awaiting broker response",
  request_submitted: "Request submitted",
  customer_action: "Customer action required",
  not_authorized: "Waiting for your decision",
  inconclusive: "Inconclusive",
  unsuccessful: "Removal unsuccessful",
};

export const OUTCOME_MEANING: Record<ReportOutcome, string> = {
  verified_removed: "Our own check found the listing gone.",
  awaiting_broker: "A request was sent and our last check still saw the listing. Sites can take weeks.",
  request_submitted: "A request was sent (or you told us you sent it). We have not yet had a clear result.",
  customer_action: "This one needs you: the site blocks our tools, or the automatic attempt failed.",
  not_authorized: "We found this listing. Nothing is sent until you approve it.",
  inconclusive: "We could not read the page well enough to say whether the listing is gone.",
  unsuccessful: "The listing was still there when we stopped checking (45 days).",
};

/** Newest request for an exposure, if any. */
export function latestRequest(exposureId: string, requests: RemovalRequest[]): RemovalRequest | undefined {
  return requests
    .filter((r) => r.exposure_id === exposureId)
    .sort((a, b) => b.created_at.localeCompare(a.created_at))[0];
}

export function reportOutcome(exposure: Exposure, req: RemovalRequest | undefined): ReportOutcome {
  if (!req) return exposure.is_active ? "not_authorized" : "verified_removed";
  switch (req.status) {
    case "verified_removed":
      return "verified_removed";
    case "rejected":
      return "unsuccessful";
    case "unverifiable":
      return "inconclusive";
    case "failed":
    case "authorized":
      return req.status === "authorized" ? "request_submitted" : "customer_action";
    case "reappeared":
      return "customer_action";
    case "submitted":
    case "acknowledged":
      if (req.last_check_result === "inconclusive") return "inconclusive";
      if (req.last_check_result === "still_present") return "awaiting_broker";
      return "request_submitted";
  }
}
