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

export function formatDate(iso: string): string {
  return new Date(iso).toLocaleDateString("en-US", {
    month: "short",
    day: "numeric",
    year: "numeric",
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
