import { describe, expect, test } from "bun:test";
import {
  DEFAULT_PANEL_ORDER,
  panelScrollStep,
  parsePanelOrder,
  parseScrollDirection,
  readRememberedPanel,
  saveRememberedPanel,
} from "../src/panelOrder";
import { accentFromBuckets, themeFromBuckets } from "../src/music/palette";

describe("parseScrollDirection & panelScrollStep", () => {
  test("defaults unknown values to natural and flips step when inverted", () => {
    expect(parseScrollDirection(null)).toBe("natural");
    expect(parseScrollDirection("natural")).toBe("natural");
    expect(parseScrollDirection("unexpected")).toBe("natural");
    expect(parseScrollDirection("inverted")).toBe("inverted");

    expect(panelScrollStep(60, "natural")).toBe(1);
    expect(panelScrollStep(-60, "natural")).toBe(-1);
    expect(panelScrollStep(60, "inverted")).toBe(-1);
    expect(panelScrollStep(-60, "inverted")).toBe(1);
  });
});

describe("parsePanelOrder", () => {
  test("preserves known panels and appends missing defaults", () => {
    expect(parsePanelOrder('["music","status"]')).toEqual([
      "music",
      "status",
      "command-center",
      "calendar",
      "shelf",
      "ai",
    ]);
    expect(parsePanelOrder("not json")).toBeNull();
    expect(parsePanelOrder(null)).toBeNull();
    expect(DEFAULT_PANEL_ORDER.length).toBe(6);
  });

  test("readRememberedPanel validates against available panels", () => {
    const store: Record<string, string> = {};
    (globalThis as any).localStorage = {
      getItem: (k: string) => store[k] ?? null,
      setItem: (k: string, v: string) => { store[k] = v; },
    };
    saveRememberedPanel("shelf");
    expect(readRememberedPanel(["status", "music", "shelf"])).toBe("shelf");
    expect(readRememberedPanel(["status", "music"])).toBeNull();
    saveRememberedPanel("invalid-panel");
    expect(readRememberedPanel(["status", "music", "shelf"])).toBe("shelf");
  });
});

describe("themeFromBuckets & accentFromBuckets", () => {
  test("produces readable text contrast on bright covers and dark-safe vibrant accents", () => {
    const theme = themeFromBuckets([
      { color: [245, 245, 240], n: 900 },
      { color: [40, 120, 220], n: 100 },
    ]);
    expect(theme.background).not.toBe("");
    expect(theme.text).toBe("rgb(18 18 18)");
    expect(theme.accent).not.toBe("");

    const darkSurfaceAccent = accentFromBuckets([
      { color: [245, 240, 232], n: 700 },
      { color: [185, 28, 28], n: 200 },
    ], false);
    expect(darkSurfaceAccent).not.toBe("");
    expect(darkSurfaceAccent).not.toBe("rgb(18 18 18)");
  });
});
