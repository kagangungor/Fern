import { describe, expect, test } from "bun:test";
import {
  MAX_WIDGETS_PER_ZONE,
  computeWidgetsRowWidth,
  normalizeWidgetConfig,
  swapWidgetSide,
} from "../src/widgetConfig";

describe("normalizeWidgetConfig", () => {
  test("clamps each side to MAX_WIDGETS_PER_ZONE (2)", () => {
    expect(MAX_WIDGETS_PER_ZONE).toBe(2);
    expect(
      normalizeWidgetConfig({
        left: ["weather", "cpu", "ram"],
        right: ["battery", "disk", "net"],
      })
    ).toEqual({
      left: ["weather", "cpu"],
      right: ["battery", "disk"],
    });
  });
});

describe("swapWidgetSide", () => {
  test("blocks moving a widget into a side that already has 2 widgets", () => {
    const initial = { left: ["weather"], right: ["battery", "cpu"] };
    expect(swapWidgetSide(initial, "weather")).toEqual(initial);
  });

  test("moves a widget to the opposite side when space is available", () => {
    expect(swapWidgetSide({ left: ["weather", "cpu"], right: ["battery"] }, "weather")).toEqual({
      left: ["cpu"],
      right: ["battery", "weather"],
    });
  });
});

describe("computeWidgetsRowWidth", () => {
  test("uses the larger side count symmetrically on both left and right", () => {
    const asymmetric = computeWidgetsRowWidth(2, 0, 0, 0, 0);
    const balanced = computeWidgetsRowWidth(2, 2, 0, 0, 0);
    expect(asymmetric).toBe(balanced);
    expect(asymmetric).toBe(460);
  });

  test("preserves net, dual clock, and privacy width allowances", () => {
    expect(computeWidgetsRowWidth(1, 1, 45, 50, 22)).toBe(447);
    expect(computeWidgetsRowWidth(0, 0, 0, 40, 22)).toBe(0);
  });
});
