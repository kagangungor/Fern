import { dateLocale } from "./i18n";

export type NotchDateFormat = "weekday" | "date";
export type NotchDatePosition = "left" | "right";

export const parseNotchDateFormat = (v: unknown): NotchDateFormat => (v === "date" ? "date" : "weekday");
export const parseNotchDatePosition = (v: unknown): NotchDatePosition => (v === "right" ? "right" : "left");

/** "Pzt 24 Kas" / "Mon Nov 24", or just "24 Kas" / "Nov 24". Weekday comes first in every locale. */
export function formatNotchDate(now: Date, format: NotchDateFormat): string {
  const locale = dateLocale();
  const day = now.toLocaleDateString(locale, { day: "numeric", month: "short" });
  if (format === "date") return day;
  return `${now.toLocaleDateString(locale, { weekday: "short" })} ${day}`;
}
