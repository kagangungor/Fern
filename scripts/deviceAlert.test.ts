import { describe, expect, test } from "bun:test";
import {
  DEVICE_ALERT_RECOVERY_MS,
  enqueueDeviceAlert,
  shouldClearDeviceAlertOnModeChange,
  shouldPreserveModeOnHover,
  shouldScheduleOnScreenRecovery,
} from "../src/devices/deviceAlertState";

describe("enqueueDeviceAlert", () => {
  test("presents immediately when no alert is active", () => {
    const incoming = { id: "kbd-1", connected: false };
    expect(enqueueDeviceAlert(null, [], incoming)).toEqual({
      action: "present",
      queue: [],
    });
  });

  test("replaces immediately when the same device id transitions (disconnect -> reconnect)", () => {
    const current = { id: "kbd-1", connected: false };
    const incoming = { id: "kbd-1", connected: true };
    expect(enqueueDeviceAlert(current, [{ id: "mouse-1", connected: true }], incoming)).toEqual({
      action: "present",
      queue: [{ id: "mouse-1", connected: true }],
    });
  });

  test("queues and deduplicates when a different id arrives while an alert is active", () => {
    const current = { id: "kbd-classic", connected: false };
    const queue = [
      { id: "mouse-1", connected: true },
      { id: "kbd-le", connected: false },
    ];
    const incoming = { id: "kbd-le", connected: true };
    expect(enqueueDeviceAlert(current, queue, incoming, 3)).toEqual({
      action: "queue",
      queue: [
        { id: "mouse-1", connected: true },
        { id: "kbd-le", connected: true },
      ],
    });
  });

  test("caps queued alerts to maxQueue", () => {
    const current = { id: "d0", connected: false };
    const queue = [
      { id: "d1", connected: true },
      { id: "d2", connected: true },
      { id: "d3", connected: true },
    ];
    const incoming = { id: "d4", connected: true };
    expect(enqueueDeviceAlert(current, queue, incoming, 3)).toEqual({
      action: "queue",
      queue: [
        { id: "d2", connected: true },
        { id: "d3", connected: true },
        { id: "d4", connected: true },
      ],
    });
  });
});

describe("shouldPreserveModeOnHover", () => {
  test("preserves shelf and device modes so hover does not abort active alerts", () => {
    expect(shouldPreserveModeOnHover("device")).toBe(true);
    expect(shouldPreserveModeOnHover("shelf")).toBe(true);
  });

  test("allows hover to switch out of normal modes", () => {
    expect(shouldPreserveModeOnHover("status")).toBe(false);
    expect(shouldPreserveModeOnHover("music")).toBe(false);
    expect(shouldPreserveModeOnHover("command-center")).toBe(false);
  });
});

describe("shouldClearDeviceAlertOnModeChange", () => {
  test("clears an active alert if fernMode moved away from device", () => {
    expect(shouldClearDeviceAlertOnModeChange("shelf", true)).toBe(true);
    expect(shouldClearDeviceAlertOnModeChange("status", true)).toBe(true);
    expect(shouldClearDeviceAlertOnModeChange("music", true)).toBe(true);
  });

  test("does not clear while still in device mode or when no alert is active", () => {
    expect(shouldClearDeviceAlertOnModeChange("device", true)).toBe(false);
    expect(shouldClearDeviceAlertOnModeChange("status", false)).toBe(false);
  });
});

describe("shouldScheduleOnScreenRecovery", () => {
  test("schedules recovery if onScreen is still true when alert is no longer active", () => {
    expect(shouldScheduleOnScreenRecovery(true, "status", false)).toBe(true);
    expect(shouldScheduleOnScreenRecovery(true, "music", false)).toBe(true);
    expect(shouldScheduleOnScreenRecovery(true, "status", true)).toBe(true);
    expect(shouldScheduleOnScreenRecovery(true, "device", false)).toBe(true);
  });

  test("does not schedule recovery while device alert is actively showing or already off-screen", () => {
    expect(shouldScheduleOnScreenRecovery(true, "device", true)).toBe(false);
    expect(shouldScheduleOnScreenRecovery(false, "status", false)).toBe(false);
    expect(shouldScheduleOnScreenRecovery(false, "device", false)).toBe(false);
  });

  test("recovery timeout stays comfortably above the 120ms exit animation", () => {
    expect(DEVICE_ALERT_RECOVERY_MS).toBeGreaterThan(120);
  });
});
