// @ts-expect-error -- bun:test is provided by the Bun test runner without @types/bun installed
import { describe, expect, test } from "bun:test";
import { DEFAULT_PANEL_ORDER, parsePanelOrder } from "./panelOrder";

describe("panelOrder", () => {
  test("includes ai in DEFAULT_PANEL_ORDER", () => {
    expect(DEFAULT_PANEL_ORDER).toEqual([
      "command-center",
      "status",
      "music",
      "calendar",
      "shelf",
      "ai",
    ]);
  });

  test("preserves saved order and appends missing panels including ai", () => {
    expect(parsePanelOrder('["music","status"]')).toEqual([
      "music",
      "status",
      "command-center",
      "calendar",
      "shelf",
      "ai",
    ]);
  });

  test("strips unknown panel ids and deduplicates entries", () => {
    expect(parsePanelOrder('["ai","unknown","music","ai"]')).toEqual([
      "ai",
      "music",
      "command-center",
      "status",
      "calendar",
      "shelf",
    ]);
  });

  test("returns null for null, undefined, empty, or invalid inputs", () => {
    expect(parsePanelOrder(null)).toBeNull();
    expect(parsePanelOrder(undefined)).toBeNull();
    expect(parsePanelOrder("")).toBeNull();
    expect(parsePanelOrder("not-json")).toBeNull();
    expect(parsePanelOrder('{"not":"array"}')).toBeNull();
  });
});
