import { invoke } from "@tauri-apps/api/core";
import type { SearchApp } from "../types";

export const APPS_CACHE_KEY = "fern-launcher-apps-cache";
export const RECENTS_KEY = "fern-launcher-recents";

const MAX_RECENTS = 8;

interface RawAppInfo {
  name: string;
  path: string;
  icon?: string | null;
  is_running?: boolean;
  hwnd?: number | null;
  args?: string | null;
}

let appsCache: SearchApp[] | null = null;
let fetchedFromIpc = false;
let inFlight: Promise<SearchApp[]> | null = null;

export function appKey(app: SearchApp): string {
  return `${app.path}\u0000${app.name}`;
}

function normalizeApp(raw: RawAppInfo, forceRunning?: boolean): SearchApp {
  const app: SearchApp = {
    name: raw.name,
    path: raw.path,
    icon: raw.icon ?? null,
    is_running: forceRunning ?? Boolean(raw.is_running),
  };
  if (typeof raw.hwnd === "number") {
    app.hwnd = raw.hwnd;
  }
  if (typeof raw.args === "string" && raw.args.length > 0) {
    app.args = raw.args;
  }
  return app;
}

function readStoredApps(): SearchApp[] {
  if (typeof localStorage === "undefined") return [];
  try {
    const raw = localStorage.getItem(APPS_CACHE_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    const apps: SearchApp[] = [];
    for (const item of parsed) {
      if (
        item &&
        typeof item === "object" &&
        typeof item.name === "string" &&
        typeof item.path === "string" &&
        item.path !== "start"
      ) {
        apps.push(
          normalizeApp(
            {
              name: item.name,
              path: item.path,
              icon: null,
              is_running: false,
              args: typeof item.args === "string" ? item.args : undefined,
            },
            false
          )
        );
      }
    }
    return apps;
  } catch {
    return [];
  }
}

function writeStoredApps(apps: SearchApp[]): void {
  if (typeof localStorage === "undefined") return;
  try {
    const stripped = apps.map((app) => ({
      name: app.name,
      path: app.path,
      icon: null,
      is_running: false,
      ...(app.args ? { args: app.args } : {}),
    }));
    localStorage.setItem(APPS_CACHE_KEY, JSON.stringify(stripped));
  } catch {
    // Ignore storage quota or privacy-mode errors.
  }
}

export function loadInstalledApps(forceRefresh = false): {
  cached: SearchApp[];
  ready: Promise<SearchApp[]>;
} {
  if (appsCache === null) {
    appsCache = readStoredApps();
  }
  const cached = appsCache;

  if (fetchedFromIpc && !forceRefresh) {
    return { cached, ready: Promise.resolve(cached) };
  }

  if (!inFlight || forceRefresh) {
    inFlight = invoke<RawAppInfo[]>("get_installed_apps")
      .then((list) => {
        const normalized = Array.isArray(list)
          ? list
              .filter(
                (item): item is RawAppInfo =>
                  Boolean(item) &&
                  typeof item.name === "string" &&
                  typeof item.path === "string" &&
                  item.path !== "start"
              )
              .map((item) => normalizeApp(item, false))
          : [];
        appsCache = normalized;
        fetchedFromIpc = true;
        writeStoredApps(normalized);
        return normalized;
      })
      .catch(() => cached)
      .finally(() => {
        inFlight = null;
      });
  }

  return { cached, ready: inFlight };
}

export function readRecents(): string[] {
  if (typeof localStorage === "undefined") return [];
  try {
    const raw = localStorage.getItem(RECENTS_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed.filter((v): v is string => typeof v === "string").slice(0, MAX_RECENTS);
  } catch {
    return [];
  }
}

export function recordRecent(app: SearchApp): void {
  if (typeof localStorage === "undefined") return;
  const key = appKey(app);
  const next = [key, ...readRecents().filter((k) => k !== key)].slice(0, MAX_RECENTS);
  try {
    localStorage.setItem(RECENTS_KEY, JSON.stringify(next));
  } catch {
    // Ignore storage errors.
  }
}

export async function fetchRunningApps(): Promise<SearchApp[]> {
  try {
    const list = await invoke<RawAppInfo[]>("get_active_windows");
    if (!Array.isArray(list)) return [];
    return list
      .filter(
        (item): item is RawAppInfo =>
          Boolean(item) &&
          typeof item.name === "string" &&
          typeof item.path === "string" &&
          item.path !== "start"
      )
      .map((item) => normalizeApp(item, true));
  } catch {
    return [];
  }
}
