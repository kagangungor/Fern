import { describe, expect, test } from "bun:test";
import {
  SCHEMA_VERSION_KEY,
  SETTINGS_SCHEMA_VERSION,
  detectSchemaVersion,
  migrateSettingsRecord,
  normalizeMonitorMode,
  resolveMonitorWindowMode,
} from "../src/settings/schema";

describe("settings schema v2 migration", () => {
  test("detectSchemaVersion returns 1 for missing or invalid values and integer >= 2 otherwise", () => {
    expect(detectSchemaVersion({})).toBe(1);
    expect(detectSchemaVersion(null)).toBe(1);
    expect(detectSchemaVersion(undefined)).toBe(1);
    expect(detectSchemaVersion({ [SCHEMA_VERSION_KEY]: 1 })).toBe(1);
    expect(detectSchemaVersion({ [SCHEMA_VERSION_KEY]: 0 })).toBe(1);
    expect(detectSchemaVersion({ [SCHEMA_VERSION_KEY]: -2 })).toBe(1);
    expect(detectSchemaVersion({ [SCHEMA_VERSION_KEY]: 2.5 })).toBe(1);
    expect(detectSchemaVersion({ [SCHEMA_VERSION_KEY]: "2" })).toBe(1);
    expect(detectSchemaVersion({ [SCHEMA_VERSION_KEY]: "corrupt" })).toBe(1);
    expect(detectSchemaVersion({ [SCHEMA_VERSION_KEY]: true })).toBe(1);
    expect(detectSchemaVersion({ [SCHEMA_VERSION_KEY]: 2 })).toBe(2);
    expect(detectSchemaVersion({ [SCHEMA_VERSION_KEY]: 3 })).toBe(3);
  });

  test("fresh empty object gets schemaVersion 2", () => {
    const { settings, report } = migrateSettingsRecord({});
    expect(report.migrated).toBe(true);
    expect(report.fromVersion).toBe(1);
    expect(report.toVersion).toBe(SETTINGS_SCHEMA_VERSION);
    expect(settings[SCHEMA_VERSION_KEY]).toBe(2);
    expect(report.normalizedKeys).toEqual([]);
    expect(report.deprecatedKeysMapped).toEqual([]);
  });

  test("migrates v1 settings without mutating input and preserves unknown/custom keys", () => {
    const input: Record<string, unknown> = {
      "fern-theme-mode": "dark",
      "fern-dock-enabled": "true",
      "fern-notch-enabled": false,
      "fern-scale": 1.25,
      "fern-dock-mode": "auto-hide",
      "custom-plugin-pref": { enabled: true, tags: ["a", "b"] },
    };
    const snapshot = structuredClone(input);

    const { settings, report } = migrateSettingsRecord(input);
    expect(input).toEqual(snapshot);
    expect(report.migrated).toBe(true);
    expect(report.fromVersion).toBe(1);
    expect(report.toVersion).toBe(2);
    expect(settings[SCHEMA_VERSION_KEY]).toBe(2);
    expect(settings["fern-theme-mode"]).toBe("dark");
    expect(settings["fern-dock-enabled"]).toBe("true");
    expect(settings["fern-notch-enabled"]).toBe("false");
    expect(settings["fern-scale"]).toBe("1.25");
    expect(settings["fern-dock-mode"]).toBe("smart");
    expect(settings["custom-plugin-pref"]).toEqual({ enabled: true, tags: ["a", "b"] });
    expect(report.normalizedKeys).toEqual([
      "fern-dock-mode",
      "fern-notch-enabled",
      "fern-scale",
    ]);
  });

  test("already-v2 settings record is idempotent", () => {
    const v2: Record<string, unknown> = {
      [SCHEMA_VERSION_KEY]: 2,
      "fern-dock-mode": "smart",
      "fern-theme-mode": "light",
    };

    const first = migrateSettingsRecord(v2);
    expect(first.report.migrated).toBe(false);
    expect(first.report.fromVersion).toBe(2);
    expect(first.report.toVersion).toBe(2);
    expect(first.settings).toEqual(v2);

    const second = migrateSettingsRecord(first.settings);
    expect(second.report.migrated).toBe(false);
    expect(second.settings).toEqual(v2);
  });

  test("corrupt schemaVersion falls back to v1 and upgrades cleanly", () => {
    const { settings, report } = migrateSettingsRecord({
      [SCHEMA_VERSION_KEY]: "not-a-number",
      "fern-theme-style": "glass",
    });

    expect(report.migrated).toBe(true);
    expect(report.fromVersion).toBe(1);
    expect(report.toVersion).toBe(2);
    expect(settings[SCHEMA_VERSION_KEY]).toBe(2);
    expect(settings["fern-theme-style"]).toBe("glass");
  });

  test("maps legacy key aliases without overwriting already-set canonical keys", () => {
    const { settings, report } = migrateSettingsRecord({
      "fern-monitor-mode": "all",
      "fern-dock-monitor-mode": "specific",
      "fern-monitor-id": "DISPLAY2",
      "fern-language-code": "tr",
    });

    expect(report.migrated).toBe(true);
    expect(settings["fern-dock-monitor-mode"]).toBe("specific");
    expect(settings["fern-dock-monitor-id"]).toBe("DISPLAY2");
    expect(settings["fern-language"]).toBe("tr");
    expect(settings["fern-monitor-mode"]).toBeUndefined();
    expect(settings["fern-monitor-id"]).toBeUndefined();
    expect(settings["fern-language-code"]).toBeUndefined();
    expect(report.deprecatedKeysMapped).toEqual([
      "fern-language-code->fern-language",
      "fern-monitor-id->fern-dock-monitor-id",
    ]);
  });

  test("normalizes multi-monitor settings, per-monitor maps, and strips null tombstones", () => {
    const { settings, report } = migrateSettingsRecord({
      "fern-dock-monitor-mode": "ALL",
      "fern-notch-monitor-mode": "invalid-mode",
      "fern-dock-mode-by-monitor": JSON.stringify({ DISPLAY1: "auto-hide", DISPLAY2: "fixed" }),
      "fern-notch-mode-by-monitor": { DISPLAY1: "auto-hide" },
      "fern-weather-city": null,
      "fern-scale": "  ",
    });

    expect(report.migrated).toBe(true);
    expect(settings["fern-dock-monitor-mode"]).toBe("all");
    expect(settings["fern-notch-monitor-mode"]).toBe("primary");
    expect(JSON.parse(settings["fern-dock-mode-by-monitor"] as string)).toEqual({
      DISPLAY1: "smart",
      DISPLAY2: "fixed",
    });
    expect(JSON.parse(settings["fern-notch-mode-by-monitor"] as string)).toEqual({
      DISPLAY1: "smart",
    });
    expect(settings["fern-weather-city"]).toBeUndefined();
    expect(settings["fern-scale"]).toBeUndefined();
  });

  test("multi-monitor mode helpers resolve primary, all, and legacy auto-hide overrides", () => {
    expect(normalizeMonitorMode("all")).toBe("all");
    expect(normalizeMonitorMode("SPECIFIC")).toBe("specific");
    expect(normalizeMonitorMode("bogus")).toBe("primary");
    expect(normalizeMonitorMode(undefined)).toBe("primary");

    // Single monitor ignores per-monitor overrides
    expect(
      resolveMonitorWindowMode({
        kind: "dock",
        monitorMode: "all",
        monitorCount: 1,
        monitorId: "DISPLAY1",
        defaultMode: "auto-hide",
        modeByMonitorRaw: JSON.stringify({ DISPLAY1: "fixed" }),
      })
    ).toBe("smart");

    // Multi-monitor in "all" mode applies per-monitor override and normalizes auto-hide
    expect(
      resolveMonitorWindowMode({
        kind: "dock",
        monitorMode: "all",
        monitorCount: 2,
        monitorId: "DISPLAY2",
        defaultMode: "fixed",
        modeByMonitorRaw: JSON.stringify({ DISPLAY2: "auto-hide" }),
      })
    ).toBe("smart");

    // Missing monitor entry falls back to default mode
    expect(
      resolveMonitorWindowMode({
        kind: "notch",
        monitorMode: "all",
        monitorCount: 2,
        monitorId: "DISPLAY3",
        defaultMode: undefined,
        modeByMonitorRaw: JSON.stringify({ DISPLAY1: "smart" }),
      })
    ).toBe("fixed");
  });
});
