import { describe, expect, test } from "bun:test";
import {
  THEME_PRESETS,
  contrastText,
  hslToHex,
  isLightSurface,
  materialPalette,
  resolveThemeMode,
} from "../src/theme";

describe("resolveThemeMode", () => {
  test("resolves auto based on OS dark scheme", () => {
    expect(resolveThemeMode("auto", true)).toBe("dark");
    expect(resolveThemeMode("auto", false)).toBe("light");
  });

  test("preserves explicit modes unchanged", () => {
    expect(resolveThemeMode("dark", false)).toBe("dark");
    expect(resolveThemeMode("light", true)).toBe("light");
    expect(resolveThemeMode("custom", true)).toBe("custom");
    expect(resolveThemeMode("adaptive", false)).toBe("adaptive");
  });
});

describe("contrastText", () => {
  test("picks dark text on bright colors and light text on dark colors", () => {
    expect(contrastText("#ffffff")).toBe("#000000");
    expect(contrastText("#000000")).toBe("#ffffff");
    expect(contrastText("#f5f5f7")).toBe("#000000");
    expect(contrastText("#1c1c1e")).toBe("#ffffff");
  });

  test("handles 3-digit shorthand hex", () => {
    expect(contrastText("#fff")).toBe("#000000");
    expect(contrastText("#000")).toBe("#ffffff");
  });

  test("isLightSurface keeps mid-gray and saturated dark colors in dark mode past 55%", () => {
    expect(isLightSurface(hslToHex(240, 4, 56))).toBe(false);
    expect(isLightSurface(hslToHex(220, 80, 60))).toBe(false);
    expect(isLightSurface(hslToHex(30, 18, 95))).toBe(true);
  });
});

describe("THEME_PRESETS", () => {
  test("provides 9 curated presets with valid hex color and accent", () => {
    expect(THEME_PRESETS).toHaveLength(9);
    for (const preset of THEME_PRESETS) {
      expect(preset.color).toMatch(/^#[0-9a-f]{6}$/i);
      expect(preset.accent).toMatch(/^#[0-9a-f]{6}$/i);
      expect(preset.saturation).toBeGreaterThanOrEqual(0);
      expect(preset.saturation).toBeLessThanOrEqual(1);
    }
  });
});

describe("hslToHex", () => {
  test("converts primary red and white accurately", () => {
    expect(hslToHex(0, 100, 50)).toBe("#ff0000");
    expect(hslToHex(0, 0, 100)).toBe("#ffffff");
  });
});

describe("materialPalette", () => {
  test("builds --fern-* CSS tokens including panel background", () => {
    const darkPalette = materialPalette("#0a84ff", false, 0.96, 1.0, 0.98);
    expect(darkPalette["--fern-accent"]).toBe("#0a84ff");
    expect(darkPalette["--fern-bg"]).toContain("hsla(");
    expect(darkPalette["--fern-bg-panel"]).toContain("hsla(");
    expect(darkPalette["--fern-highlight"]).toContain("hsla(");
    expect(darkPalette["--fern-on-highlight"]).toContain("hsla(");
  });
});
