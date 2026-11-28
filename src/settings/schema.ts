import type { MonitorMode } from "./types";

export const SETTINGS_SCHEMA_VERSION = 2;
export const SCHEMA_VERSION_KEY = "schemaVersion";

const LEGACY_KEY_ALIASES: ReadonlyArray<readonly [string, string]> = [
  ["fern-monitor-mode", "fern-dock-monitor-mode"],
  ["fern-monitor-id", "fern-dock-monitor-id"],
  ["fern-language-code", "fern-language"],
];

const MONITOR_MODE_KEYS = [
  "fern-dock-monitor-mode",
  "fern-notch-monitor-mode",
] as const;

const WINDOW_MODE_KEYS = [
  "fern-dock-mode",
  "fern-notch-mode",
] as const;

const WINDOW_MODE_MAP_KEYS = [
  "fern-dock-mode-by-monitor",
  "fern-notch-mode-by-monitor",
] as const;

const NON_EMPTY_KNOWN_KEYS = [
  "fern-dock-mode",
  "fern-notch-mode",
  "fern-scale",
] as const;

export interface SettingsMigrationReport {
  fromVersion: number;
  toVersion: number;
  migrated: boolean;
  normalizedKeys: string[];
  deprecatedKeysMapped: string[];
}

export interface SettingsMigrationResult {
  settings: Record<string, unknown>;
  report: SettingsMigrationReport;
}

/**
 * Detect the settings schema version from a raw settings record.
 * Returns 1 when `schemaVersion` is missing or invalid, or its integer value when >= 2.
 */
export function detectSchemaVersion(raw: Record<string, unknown> | null | undefined): number {
  if (!raw || typeof raw !== "object") return 1;
  const value = raw[SCHEMA_VERSION_KEY];
  if (typeof value === "number" && Number.isInteger(value) && value >= 2) {
    return value;
  }
  return 1;
}

export function normalizeMonitorMode(raw: unknown): MonitorMode {
  if (typeof raw !== "string") return "primary";
  const normalized = raw.trim().toLowerCase();
  if (normalized === "all" || normalized === "specific") {
    return normalized;
  }
  return "primary";
}

export function normalizeWindowMode(raw: unknown, fallback: "smart" | "fixed" = "smart"): string {
  if (typeof raw !== "string") return fallback;
  const trimmed = raw.trim();
  if (!trimmed) return fallback;
  return trimmed === "auto-hide" ? "smart" : trimmed;
}

/**
 * Mirrors backend `monitors::resolve_mode_for_label` for frontend validation & tests.
 */
export function resolveMonitorWindowMode(options: {
  kind: "dock" | "notch";
  monitorMode: unknown;
  monitorCount: number;
  monitorId: string;
  defaultMode: unknown;
  modeByMonitorRaw: unknown;
}): string {
  const fallback = options.kind === "dock" ? "smart" : "fixed";
  const defaultMode = normalizeWindowMode(options.defaultMode, fallback);
  const monMode = normalizeMonitorMode(options.monitorMode);

  if (!(options.monitorCount > 1 && monMode === "all")) {
    return defaultMode;
  }

  let map: Record<string, unknown> | null = null;
  if (typeof options.modeByMonitorRaw === "string") {
    try {
      const parsed = JSON.parse(options.modeByMonitorRaw);
      if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) {
        map = parsed as Record<string, unknown>;
      }
    } catch {
      map = null;
    }
  } else if (
    options.modeByMonitorRaw &&
    typeof options.modeByMonitorRaw === "object" &&
    !Array.isArray(options.modeByMonitorRaw)
  ) {
    map = options.modeByMonitorRaw as Record<string, unknown>;
  }

  if (!map || !(options.monitorId in map)) {
    return defaultMode;
  }
  return normalizeWindowMode(map[options.monitorId], fallback);
}

function pushUnique(list: string[], item: string) {
  if (!list.includes(item)) {
    list.push(item);
  }
}

function normalizeWindowModeMap(val: unknown): string | null {
  let entries: Record<string, unknown> | null = null;
  let changed = typeof val !== "string";

  if (typeof val === "string") {
    try {
      const parsed = JSON.parse(val);
      if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) {
        entries = { ...(parsed as Record<string, unknown>) };
      }
    } catch {
      return null;
    }
  } else if (val && typeof val === "object" && !Array.isArray(val)) {
    entries = { ...(val as Record<string, unknown>) };
  }

  if (!entries) return null;

  for (const [monId, entryVal] of Object.entries(entries)) {
    if (typeof entryVal === "string") {
      const trimmed = entryVal.trim();
      if (trimmed === "auto-hide") {
        entries[monId] = "smart";
        changed = true;
      } else if (trimmed !== entryVal) {
        entries[monId] = trimmed;
        changed = true;
      }
    }
  }

  return changed ? JSON.stringify(entries) : null;
}

/**
 * Idempotently migrate a settings record to `SETTINGS_SCHEMA_VERSION` without
 * mutating the input object. Mirrors the Rust `settings_schema::migrate_settings_map` rules.
 */
export function migrateSettingsRecord(raw: Record<string, unknown>): SettingsMigrationResult {
  const fromVersion = detectSchemaVersion(raw);
  const settings: Record<string, unknown> = { ...raw };

  if (fromVersion >= SETTINGS_SCHEMA_VERSION) {
    return {
      settings,
      report: {
        fromVersion,
        toVersion: fromVersion,
        migrated: false,
        normalizedKeys: [],
        deprecatedKeysMapped: [],
      },
    };
  }

  const normalizedKeys: string[] = [];
  const deprecatedKeysMapped: string[] = [];

  // 1. Map deprecated/legacy key aliases without overwriting already-set canonical keys.
  for (const [oldKey, newKey] of LEGACY_KEY_ALIASES) {
    if (Object.prototype.hasOwnProperty.call(settings, oldKey)) {
      const oldVal = settings[oldKey];
      delete settings[oldKey];
      const canonicalHasValue =
        settings[newKey] !== undefined && settings[newKey] !== null;
      if (!canonicalHasValue && oldVal !== undefined && oldVal !== null) {
        settings[newKey] = oldVal;
        pushUnique(deprecatedKeysMapped, `${oldKey}->${newKey}`);
      }
    }
  }

  // 2. Strip null/undefined tombstones on fern-* keys.
  for (const key of Object.keys(settings)) {
    if (key.startsWith("fern-") && (settings[key] === null || settings[key] === undefined)) {
      delete settings[key];
      pushUnique(normalizedKeys, key);
    }
  }

  // 3. Normalize boolean/number primitives on fern-* keys to strings.
  for (const [key, val] of Object.entries(settings)) {
    if (!key.startsWith("fern-")) continue;
    if (typeof val === "boolean") {
      settings[key] = val ? "true" : "false";
      pushUnique(normalizedKeys, key);
    } else if (typeof val === "number" && Number.isFinite(val)) {
      settings[key] = String(val);
      pushUnique(normalizedKeys, key);
    }
  }

  // 4. Normalize legacy "auto-hide" window mode values -> "smart".
  for (const key of WINDOW_MODE_KEYS) {
    const val = settings[key];
    if (typeof val === "string") {
      const trimmed = val.trim();
      if (trimmed === "auto-hide") {
        settings[key] = "smart";
        pushUnique(normalizedKeys, key);
      } else if (trimmed !== val && trimmed.length > 0) {
        settings[key] = trimmed;
        pushUnique(normalizedKeys, key);
      }
    }
  }

  // 5. Normalize per-monitor window mode JSON maps.
  for (const key of WINDOW_MODE_MAP_KEYS) {
    if (Object.prototype.hasOwnProperty.call(settings, key)) {
      const normalizedJson = normalizeWindowModeMap(settings[key]);
      if (normalizedJson !== null) {
        settings[key] = normalizedJson;
        pushUnique(normalizedKeys, key);
      }
    }
  }

  // 6. Normalize multi-monitor mode values to valid MonitorMode strings.
  for (const key of MONITOR_MODE_KEYS) {
    if (Object.prototype.hasOwnProperty.call(settings, key)) {
      const normalized = normalizeMonitorMode(settings[key]);
      if (settings[key] !== normalized) {
        settings[key] = normalized;
        pushUnique(normalizedKeys, key);
      }
    }
  }

  // 7. Strip empty/whitespace-only corrupt strings for known non-empty keys.
  for (const key of NON_EMPTY_KNOWN_KEYS) {
    const val = settings[key];
    if (typeof val === "string" && val.trim().length === 0) {
      delete settings[key];
      pushUnique(normalizedKeys, key);
    }
  }

  // 8. Stamp schemaVersion = 2.
  settings[SCHEMA_VERSION_KEY] = SETTINGS_SCHEMA_VERSION;

  normalizedKeys.sort();
  deprecatedKeysMapped.sort();

  return {
    settings,
    report: {
      fromVersion,
      toVersion: SETTINGS_SCHEMA_VERSION,
      migrated: true,
      normalizedKeys,
      deprecatedKeysMapped,
    },
  };
}
