import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

export function cn(...inputs: ClassValue[]): string {
  return twMerge(clsx(inputs));
}

/** Map a privacy score (0–100) to a label and color class. */
export function privacyScoreLabel(score: number): {
  label: string;
  color: string;
} {
  if (score >= 80) return { label: "Good", color: "text-green-500" };
  if (score >= 60) return { label: "Fair", color: "text-yellow-500" };
  if (score >= 40) return { label: "At Risk", color: "text-orange-500" };
  return { label: "Exposed", color: "text-red-500" };
}

export function severityColor(severity: string): string {
  switch (severity) {
    case "critical":
      return "bg-red-100 text-red-800 border-red-200";
    case "high":
      return "bg-orange-100 text-orange-800 border-orange-200";
    case "medium":
      return "bg-yellow-100 text-yellow-800 border-yellow-200";
    default:
      return "bg-gray-100 text-gray-700 border-gray-200";
  }
}

export function formatDate(iso: string | null | undefined): string {
  if (!iso) return "—";

  const date = new Date(iso);

  // Check if date is valid
  if (isNaN(date.getTime())) {
    return "—";
  }

  return date.toLocaleDateString("en-US", {
    month: "short",
    day: "numeric",
    year: "numeric",
  });
}

/** A date with the time, in the viewer's own time zone: "Nov 11, 2026, 12:09 PM". */
export function formatDateTime(iso: string | null | undefined): string {
  if (!iso) return "—";

  const date = new Date(iso);
  if (isNaN(date.getTime())) {
    return "—";
  }

  return date.toLocaleString("en-US", {
    month: "short",
    day: "numeric",
    year: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
}

export function piiCategoryLabel(cat: string): string {
  const labels: Record<string, string> = {
    full_name: "Full Name",
    home_address: "Home Address",
    phone_number: "Phone Number",
    email_address: "Email Address",
    date_of_birth: "Date of Birth",
    family_association: "Family Members",
    employment_history: "Employment",
    social_media_handle: "Social Media",
    vehicle_record: "Vehicle Record",
    court_record: "Court Record",
    other: "Other",
  };
  return labels[cat] ?? cat;
}

import type { RemovalRequest, RemovalStatus } from "@/types";

export function removalStatusLabel(status: RemovalStatus): string {
  const labels: Record<RemovalStatus, string> = {
    authorized: "Authorized",
    submitted: "In Progress",
    acknowledged: "Acknowledged",
    verified_removed: "Verified Removed",
    rejected: "Rejected",
    unverifiable: "Unverifiable",
    reappeared: "Reappeared",
    failed: "Failed - Retry Available",
  };
  return labels[status];
}

export function removalStatusColor(status: RemovalStatus): string {
  switch (status) {
    case "verified_removed":
      return "text-green-400 border-green-800 bg-green-950";
    case "submitted":
    case "acknowledged":
      return "text-blue-400 border-blue-800 bg-blue-950";
    case "authorized":
      return "text-yellow-400 border-yellow-800 bg-yellow-950";
    case "rejected":
    case "unverifiable":
    case "reappeared":
    case "failed":
      return "text-red-400 border-red-800 bg-red-950";
  }
}

/** True when a removal failed because the site refuses our automated browser: retrying cannot help. */
export function isAutomationBlocked(reason: string | null | undefined): boolean {
  return !!reason && reason.includes("blocked our automated browser");
}

/** True when the site's opt-out address no longer works: also not fixed by retrying. */
export function isOptOutPageGone(reason: string | null | undefined): boolean {
  return !!reason && reason.includes("no longer exists");
}

export interface RemovalStage {
  label: string;
  /** One plain sentence on what is true now and what happens next. */
  detail: string;
  color: string;
}

const AMBER = "text-amber-300 border-amber-800 bg-amber-950";

/**
 * Where a removal really stands, in words that do not claim more than we know. "Submitted" is
 * split into what we documented and what the customer told us; a check we could not complete is
 * "inconclusive", never "removed".
 */
export function removalStage(req: RemovalRequest): RemovalStage {
  const byHand = req.procedure_used === "manual";
  const inconclusive = req.last_check_result === "inconclusive";
  switch (req.status) {
    case "authorized":
      return {
        label: "Authorized",
        detail: "You approved this. We will send the opt-out where the site allows it.",
        color: removalStatusColor(req.status),
      };
    case "submitted":
    case "acknowledged":
      return {
        label: byHand ? "Reported submitted by you" : "Submitted",
        detail:
          (byHand
            ? "This is what you told us. It is not verified. "
            : "We sent the opt-out. ") +
          (inconclusive
            ? "Our last check could not read the page, so the result is inconclusive. We will try again on schedule."
            : req.last_check_result === "still_present"
              ? "Our last check still saw the listing. Sites can take weeks. We will check again on schedule."
              : "We will check the listing on a schedule (about 3, 7, 14 and 30 days after submission)."),
        color: removalStatusColor(req.status),
      };
    case "verified_removed":
      return {
        label: "Verified removed",
        detail: "Our own check confirmed the listing is gone.",
        color: removalStatusColor(req.status),
      };
    case "rejected":
      return {
        label: "Still listed",
        detail: "The listing was still there when we stopped checking (45 days). You can try again or contact the site directly.",
        color: removalStatusColor(req.status),
      };
    case "unverifiable":
      return {
        label: "Inconclusive",
        detail: "We could not read the page well enough to say whether the listing is gone. That is not the same as removed.",
        color: AMBER,
      };
    case "reappeared":
      return {
        label: "Reappeared",
        detail: "A listing we had confirmed removed has come back. Authorize a new removal to act on it.",
        color: removalStatusColor(req.status),
      };
    case "failed":
      if (isAutomationBlocked(req.failure_reason) || isOptOutPageGone(req.failure_reason)) {
        return {
          label: "Manual action required",
          detail: "We cannot submit this one automatically. Your listing has not been removed yet.",
          color: AMBER,
        };
      }
      return {
        label: "Needs your attention",
        detail: "The automatic attempt failed. Your listing has not been removed.",
        color: removalStatusColor(req.status),
      };
  }
}
