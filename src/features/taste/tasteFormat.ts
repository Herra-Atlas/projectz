import type { TasteSection } from "./types";

/** Display names for the stored section ids, in the order the profile prints them. */
export const SECTION_ORDER: TasteSection[] = [
  "identity",
  "work",
  "preferences",
  "communication",
  "interests",
  "dislikes",
  "current",
];

export const SECTION_LABELS: Record<TasteSection, string> = {
  identity: "Identity",
  work: "Work",
  preferences: "Preferences",
  communication: "Communication",
  interests: "Interests",
  dislikes: "Dislikes",
  current: "Current focus",
};

/** A confidence as the percentage the profile row shows. */
export function confidenceLabel(confidence: number): string {
  return `${Math.round(confidence * 100)}%`;
}

/** A UTC stamp as a short local date, or an empty string when absent. */
export function shortDate(stamp: string | null | undefined): string {
  if (!stamp) return "";
  const date = new Date(stamp);
  if (Number.isNaN(date.getTime())) return stamp;
  return date.toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

/** A UTC stamp as a local date and time, for a run's own row. */
export function dateTime(stamp: string | null | undefined): string {
  if (!stamp) return "";
  const date = new Date(stamp);
  if (Number.isNaN(date.getTime())) return stamp;
  return date.toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

/** How long ago a fact was last seen, in the same words the backend uses. */
export function seenLabel(stamp: string | null | undefined): string {
  if (!stamp) return "";
  const date = new Date(stamp);
  if (Number.isNaN(date.getTime())) return "";
  const days = Math.floor((Date.now() - date.getTime()) / 86_400_000);
  if (days <= 0) return "today";
  if (days === 1) return "yesterday";
  if (days < 30) return `${days} days ago`;
  const months = Math.floor(days / 30);
  return months === 1 ? "a month ago" : `${months} months ago`;
}
