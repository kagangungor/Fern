import type { WebEngine } from "./types";

export const IDLE_SECTIONS = ["recents", "running"] as const;
export type IdleSectionId = (typeof IDLE_SECTIONS)[number];

export const SEARCH_SECTIONS = [
  "calculator",
  "apps",
  "commands",
  "files",
  "web",
] as const;
export type SearchSectionId = (typeof SEARCH_SECTIONS)[number];

export const LAUNCHER_IDLE_ORDER_KEY = "fern-launcher-idle-order";
export const LAUNCHER_IDLE_DISABLED_KEY = "fern-launcher-idle-disabled";
export const LAUNCHER_SEARCH_ORDER_KEY = "fern-launcher-search-order";
export const LAUNCHER_SEARCH_DISABLED_KEY = "fern-launcher-search-disabled";
export const LAUNCHER_WEB_ENGINE_KEY = "fern-launcher-web-engine";
export const LAUNCHER_BROWSER_KEY = "fern-launcher-browser";

export function normalizeOrder<T extends string>(
  raw: unknown,
  defaults: readonly T[]
): T[] {
  const valid = new Set<string>(defaults);
  const parsed = parseStringArray(raw);
  const out: T[] = [];
  const seen = new Set<string>();
  for (const item of parsed) {
    if (valid.has(item) && !seen.has(item)) {
      out.push(item as T);
      seen.add(item);
    }
  }
  for (const def of defaults) {
    if (!seen.has(def)) {
      out.push(def);
    }
  }
  return out;
}

export function normalizeDisabled<T extends string>(
  raw: unknown,
  defaults: readonly T[]
): T[] {
  const valid = new Set<string>(defaults);
  const parsed = parseStringArray(raw);
  const out: T[] = [];
  const seen = new Set<string>();
  for (const item of parsed) {
    if (valid.has(item) && !seen.has(item)) {
      out.push(item as T);
      seen.add(item);
    }
  }
  return out;
}

function parseStringArray(raw: unknown): string[] {
  if (Array.isArray(raw)) {
    return raw.filter((v): v is string => typeof v === "string");
  }
  if (typeof raw === "string" && raw.trim()) {
    try {
      const parsed = JSON.parse(raw);
      if (Array.isArray(parsed)) {
        return parsed.filter((v): v is string => typeof v === "string");
      }
    } catch {
      return [];
    }
  }
  return [];
}

function readStorage(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

export function readIdleOrder(): IdleSectionId[] {
  return normalizeOrder(readStorage(LAUNCHER_IDLE_ORDER_KEY), IDLE_SECTIONS);
}

export function readIdleDisabled(): IdleSectionId[] {
  return normalizeDisabled(
    readStorage(LAUNCHER_IDLE_DISABLED_KEY),
    IDLE_SECTIONS
  );
}

export function readSearchOrder(): SearchSectionId[] {
  return normalizeOrder(
    readStorage(LAUNCHER_SEARCH_ORDER_KEY),
    SEARCH_SECTIONS
  );
}

export function readSearchDisabled(): SearchSectionId[] {
  return normalizeDisabled(
    readStorage(LAUNCHER_SEARCH_DISABLED_KEY),
    SEARCH_SECTIONS
  );
}

export function readWebEngine(): WebEngine {
  return readStorage(LAUNCHER_WEB_ENGINE_KEY) === "duckduckgo"
    ? "duckduckgo"
    : "google";
}

export function readBrowserPath(): string {
  return readStorage(LAUNCHER_BROWSER_KEY) ?? "";
}
