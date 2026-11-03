import { useSyncExternalStore } from "react";
import { invoke } from "@tauri-apps/api/core";
import en from "./locales/en.json";
import tr from "./locales/tr.json";

type Messages = Record<string, string>;

// Add a language by dropping src/locales/<code>.json here and listing it, then add the same
// file to LOCALES in src-tauri/src/i18n.rs so the tray menu is translated too.
// Missing keys fall back to English, so a partial translation never breaks the UI.
export const LANGUAGES: Record<string, { name: string; messages: Messages }> = {
  en: { name: "English", messages: en },
  tr: { name: "Türkçe", messages: tr },
};

const STORAGE_KEY = "fern-language";
const listeners = new Set<() => void>();

function readPreference(): string {
  try {
    return localStorage.getItem(STORAGE_KEY) || "system";
  } catch {
    return "system";
  }
}

function resolve(preference: string): string {
  if (preference in LANGUAGES) return preference;
  const system = (navigator.language || "en").toLowerCase().split("-")[0];
  return system in LANGUAGES ? system : "en";
}

export function getLanguagePreference(): string {
  return readPreference();
}

export function setLanguagePreference(value: string) {
  localStorage.setItem(STORAGE_KEY, value);
  invoke("save_setting", { key: STORAGE_KEY, value }).catch(console.error);
  listeners.forEach((l) => l());
}

export function t(key: string, params?: Record<string, string | number>): string {
  const messages = LANGUAGES[resolve(readPreference())].messages;
  const text = messages[key] ?? en[key as keyof typeof en] ?? key;
  if (!params) return text;
  return text.replace(/\{(\w+)\}/g, (m, name) => (name in params ? String(params[name]) : m));
}

/** Locale for dates and times, so they match the chosen UI language. */
export function dateLocale(): string {
  return resolve(readPreference());
}

/**
 * 0 = Sunday, 1 = Monday.
 * Returns the first day of the week for the active locale.
 * For Turkish ('tr'), weeks start on Monday (1). Default for 'en' is Sunday (0).
 */
export function firstDayOfWeek(): number {
  const loc = dateLocale();
  if (loc === "tr") return 1;
  try {
    const localeObj = new Intl.Locale(loc);
    const weekInfo = (localeObj as any).getWeekInfo?.() ?? (localeObj as any).weekInfo;
    if (weekInfo && typeof weekInfo.firstDay === "number") {
      return weekInfo.firstDay % 7;
    }
  } catch {}
  return 0;
}

/** Returns the start of the week for a given date according to the active locale. */
export function startOfWeek(d: Date): Date {
  const startDay = firstDayOfWeek();
  const offset = (d.getDay() - startDay + 7) % 7;
  const res = new Date(d);
  res.setDate(res.getDate() - offset);
  return res;
}

/** Weekday names starting on the locale's first day of week. */
export function weekdayNames(width: "short" | "narrow"): string[] {
  const startDay = firstDayOfWeek();
  return Array.from({ length: 7 }, (_, i) =>
    new Date(2023, 0, 1 + startDay + i).toLocaleDateString(dateLocale(), { weekday: width })
  );
}

export function timeAgo(ms: number | null, translate: (key: string, params?: Record<string, string | number>) => string): string {
  if (!ms) return translate("common.ago.never");
  const mins = Math.round((Date.now() - ms) / 60000);
  if (mins < 1) return translate("common.ago.now");
  if (mins < 60) return translate("common.ago.minutes", { count: mins });
  const hours = Math.round(mins / 60);
  return hours < 24
    ? translate("common.ago.hours", { count: hours })
    : translate("common.ago.days", { count: Math.round(hours / 24) });
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  // Other windows change the setting through localStorage.
  const onStorage = (e: StorageEvent) => {
    if (e.key === STORAGE_KEY || e.key === null) listener();
  };
  window.addEventListener("storage", onStorage);
  return () => {
    listeners.delete(listener);
    window.removeEventListener("storage", onStorage);
  };
}

/** Re-renders the calling component when the language changes; returns the translate function. */
export function useT() {
  useSyncExternalStore(subscribe, readPreference);
  return t;
}

export type TFunction = typeof t;
