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

import type { RemovalStatus } from "@/types";

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
