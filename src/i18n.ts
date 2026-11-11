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
export function startOfWeek(d: Date, startDay: number = firstDayOfWeek()): Date {
  const offset = (d.getDay() - startDay + 7) % 7;
  const res = new Date(d);
  res.setDate(res.getDate() - offset);
  return res;
}

/** Weekday names starting on the locale's first day of week or provided start day (0 = Sunday). */
export function weekdayNames(width: "short" | "narrow", startDay: number = firstDayOfWeek()): string[] {
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

const SYSTEM_APP_KEYS: Record<string, string> = {
  // English keys & common exe names
  "file explorer": "app.fileExplorer",
  "explorer": "app.fileExplorer",
  "control panel": "app.controlPanel",
  "task manager": "app.taskManager",
  "taskmgr": "app.taskManager",
  "settings": "app.settings",
  "notepad": "app.notepad",
  "calculator": "app.calculator",
  "command prompt": "app.commandPrompt",
  "cmd": "app.commandPrompt",
  "terminal": "app.terminal",
  "windows terminal": "app.terminal",
  "registry editor": "app.registryEditor",
  "regedit": "app.registryEditor",
  "snipping tool": "app.snippingTool",
  "recycle bin": "app.recycleBin",
  "windows security": "app.windowsSecurity",
  "camera": "app.camera",
  "clock": "app.clock",
  "photos": "app.photos",
  "weather": "app.weather",
  "paint": "app.paint",
  "sound recorder": "app.soundRecorder",
  "voice recorder": "app.soundRecorder",
  "magnifier": "app.magnifier",
  "remote desktop connection": "app.remoteDesktop",
  "start": "dockmenu.start",

  // Turkish variants
  "dosya gezgini": "app.fileExplorer",
  "denetim masası": "app.controlPanel",
  "denetim masasi": "app.controlPanel",
  "görev yöneticisi": "app.taskManager",
  "gorev yoneticisi": "app.taskManager",
  "ayarlar": "app.settings",
  "not defteri": "app.notepad",
  "hesap makinesi": "app.calculator",
  "komut istemi": "app.commandPrompt",
  "kayıt defteri düzenleyicisi": "app.registryEditor",
  "kayit defteri duzenleyicisi": "app.registryEditor",
  "ekran alıntısı aracı": "app.snippingTool",
  "ekran alintisi araci": "app.snippingTool",
  "geri dönüşüm kutusu": "app.recycleBin",
  "geri donusum kutusu": "app.recycleBin",
  "windows güvenliği": "app.windowsSecurity",
  "windows guvenligi": "app.windowsSecurity",
  "kamera": "app.camera",
  "saat": "app.clock",
  "fotoğraflar": "app.photos",
  "fotograflar": "app.photos",
  "hava durumu": "app.weather",
  "ses kaydedicisi": "app.soundRecorder",
  "büyüteç": "app.magnifier",
  "buyutec": "app.magnifier",
  "uzak masaüstü bağlantısı": "app.remoteDesktop",
  "uzak masaustu baglantisi": "app.remoteDesktop",
  "başlat": "dockmenu.start",
  "baslat": "dockmenu.start",
};

/**
 * Localizes standard system application names (like File Explorer -> Dosya Gezgini,
 * Control Panel -> Denetim Masası, Start -> Başlat) according to the active language.
 */
export function localizeAppName(name: string | undefined | null): string {
  if (!name) return "";
  const trimmed = name.trim();
  const lower = trimmed.toLowerCase();
  const key = SYSTEM_APP_KEYS[lower];
  if (key) {
    return t(key);
  }
  return trimmed;
}

