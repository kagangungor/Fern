import { areDistinctExeInstalls } from '../dockHelpers';
import type { AppInfo, AppMediaSession } from './types';

// Browser hosts (Edge/Chrome/Brave/ApplicationFrameHost) run every PWA/UWP window, so the
// title is part of the identity or two PWAs would merge into one item.
export const HOST_PROCESSES = ['msedge.exe', 'chrome.exe', 'brave.exe', 'applicationframehost.exe'];

export function isBrowserHost(path: string) {
  const p = path.toLowerCase();
  return HOST_PROCESSES.some(host => p.includes(host));
}

// Stable identity for a dock item.
export function appIdentity(p: string, executable?: string, name?: string) {
  if (!p) return "";
  const normalized = p.toLowerCase().replace(/\\/g, '/');
  // Shell application ids (AUMIDs) and bare names are unique on their own.
  if (!normalized.includes('/')) return normalized;
  if (name && isBrowserHost(normalized)) {
    return `${normalized}:${name.toLowerCase()}`;
  }
  if (executable) return `${normalized}:${executable.toLowerCase()}`;
  return normalized;
}

// "shell:AppsFolder\Microsoft.WindowsTerminal_8wekyb3d8bbwe!App" -> "microsoft.windowsterminal_8wekyb3d8bbwe"
export function packageFamilyFromAumid(path: string): string | undefined {
  const m = path.match(/^(?:shell:appsfolder\\)?([a-z0-9.-]+_[a-z0-9]{13})!/i);
  return m?.[1].toLowerCase();
}

// "C:\Program Files\WindowsApps\Microsoft.WindowsTerminal_1.25.3131.0_x64__8wekyb3d8bbwe\WindowsTerminal.exe"
// -> "microsoft.windowsterminal_8wekyb3d8bbwe"
export function packageFamilyFromWindowsAppsPath(path: string): string | undefined {
  const m = path.match(/[\\/]windowsapps[\\/]([a-z0-9.-]+)_[^\\/]*__([a-z0-9]{13})[\\/]/i);
  return m ? `${m[1]}_${m[2]}`.toLowerCase() : undefined;
}

export const itemKey = (app: AppInfo) => appIdentity(app.path, app.executable, app.name);

export const fileOf = (p: string) =>
  (p.split("/").pop()?.split("\\").pop()?.toLowerCase() || "").replace(/\.lnk$/, "");

// AUMIDs identify one app. Two PWAs in the same browser must never match on the shared exe name.
export const isIdentifier = (p: string) => !p.includes('/') && !p.includes('\\');

export const isFernWindow = (app: { path: string }) => /(^|[\\/])fern\.exe$/i.test(app.path);

// Fuzzy identity: a running window and its installed or pinned entry can have different
// paths (exe, .lnk, AUMID), so exact keys miss.
export function isSameApp(a: AppInfo, b: AppInfo): boolean {
  if (!a.path || !b.path) return false;
  if (itemKey(a) === itemKey(b)) return true;
  // Browser hosts run many apps under one exe, only the full identity can match those.
  if (isBrowserHost(a.path) || isBrowserHost(b.path)) return false;
  if (areDistinctExeInstalls(a.path, b.path)) return false;
  const aId = isIdentifier(a.path);
  const bId = isIdentifier(b.path);
  // Two opaque shell ids match only exactly (checked above).
  if (aId && bId) return false;
  if (aId !== bId) {
    // One side is an AUMID, the other a file path. AUMIDs contain the product name, so match
    // on overlapping display names plus the exe stem inside the id.
    const fileSide = aId ? b : a;
    const idLower = (aId ? a : b).path.toLowerCase();
    const na = a.name.toLowerCase();
    const nb = b.name.toLowerCase();
    if (na !== nb && !na.includes(nb) && !nb.includes(na)) return false;
    const stem = (fileSide.executable?.toLowerCase() || fileOf(fileSide.path)).replace(
      /\.(exe|lnk)$/,
      ""
    );
    return stem.length >= 3 && idLower.includes(stem);
  }
  if (appIdentity(a.path) === appIdentity(b.path)) return true;
  const normExe = (s: string) => (s.endsWith(".exe") ? s : `${s}.exe`);
  const aExe = a.executable?.toLowerCase() || fileOf(a.path);
  const bExe = b.executable?.toLowerCase() || fileOf(b.path);
  if (!aExe || !bExe) return false;
  if (normExe(aExe) !== normExe(bExe)) return false;
  const aIsLnk = a.path.toLowerCase().endsWith('.lnk');
  const bIsLnk = b.path.toLowerCase().endsWith('.lnk');
  if (aIsLnk !== bIsLnk) {
    const na = a.name.trim().toLowerCase();
    const nb = b.name.trim().toLowerCase();
    if (na && nb && na !== nb && !na.includes(nb) && !nb.includes(na)) {
      return false;
    }
  }
  return true;
}

export function parseDrives(val: string | null): string[] {
  try {
    const parsed = JSON.parse(val ?? "[]");
    return Array.isArray(parsed) ? parsed.filter((d): d is string => typeof d === "string") : [];
  } catch {
    return [];
  }
}

// Start button icon: a public asset, or the data URI of an uploaded icon ("custom:" prefix).
export function resolveStartIcon(startIcon: string): string {
  if (startIcon === "windows") return "/windows.png";
  if (startIcon.startsWith("custom:")) return startIcon.slice("custom:".length);
  return "/fern.png";
}

export function sessionForApp(app: AppInfo, sessions: AppMediaSession[]): AppMediaSession | undefined {
  if (sessions.length === 0) return undefined;
  const appNameWords = (app.name || "")
    .toLowerCase()
    .split(/[^a-z0-9]+/)
    .filter((w) => w.length >= 3);
  const pathParts = (app.path || "")
    .toLowerCase()
    .split(/[\\/]/)
    .filter(Boolean);
  const pathDirs = pathParts.slice(0, -1);

  const primaryMatch = sessions.find((s) => {
    const id = s.app_id.toLowerCase().replace(/\.exe$/, "");
    const head = id.split(/[.\s_!\\/]+/)[0];
    const bangTail = id.includes("!") ? id.split("!").pop()?.split(/[.\s_\\/]+/)[0] : undefined;
    const tokens = [head, bangTail].filter((tok): tok is string => Boolean(tok && tok.length >= 3));
    return tokens.some((tok) => appNameWords.includes(tok) || pathDirs.includes(tok));
  });
  if (primaryMatch) return primaryMatch;

  const exe = (app.executable || pathParts[pathParts.length - 1] || "").toLowerCase().replace(/\.exe$/, "");
  if (!exe) return undefined;
  return sessions.find((s) => s.app_id.toLowerCase().replace(/\.exe$/, "").includes(exe));
}

export const FERN_MENU_IDS = ["calendar", "settings", "taskmgr", "restart", "clearicons", "quit"];

export function parseMenuItems(val: unknown): string[] {
  try {
    const parsed = typeof val === "string" ? JSON.parse(val) : val;
    return Array.isArray(parsed) ? parsed.filter((id): id is string => FERN_MENU_IDS.includes(id)) : FERN_MENU_IDS;
  } catch {
    return FERN_MENU_IDS;
  }
}
