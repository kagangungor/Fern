import { describe, expect, test } from "bun:test";
import { clipToWindow, dockBand, edgeCards, nextRegion, notchBox, type RegionState } from "../src/windowRegion";
import { restingInterval } from "../src/restingInterval";

const empty: RegionState = { key: "", current: [], held: [] };
const big = { x: 0, y: 0, width: 200, height: 100 };
const small = { x: 50, y: 0, width: 100, height: 40 };

describe("clipToWindow", () => {
  test("rounds outwards and clamps to the window", () => {
    expect(clipToWindow({ x: -5.5, y: 1.2, width: 20, height: 10 }, 100, 100)).toEqual({ x: 0, y: 1, width: 15, height: 11 });
  });

  test("drops what is entirely outside", () => {
    expect(clipToWindow({ x: 0, y: -50, width: 10, height: 50 }, 100, 100)).toBeNull();
  });
});

describe("notchBox", () => {
  test("is centered, hangs from the top and scales", () => {
    expect(notchBox(1000, 2, 100, 40, 10)).toEqual({ x: 380, y: 0, width: 240, height: 100 });
  });
});

describe("dockBand", () => {
  test("is centered along the bottom edge", () => {
    expect(dockBand(1000, 600, 400, 60, 100, 40)).toEqual({ x: 200, y: 500, width: 600, height: 100 });
  });
});

describe("edgeCards", () => {
  test("gives a vertically centered strip on each side, padded and scaled", () => {
    expect(edgeCards(1000, 600, 2, 40, 100, 10)).toEqual([
      { x: 0, y: 180, width: 100, height: 240 },
      { x: 900, y: 180, width: 100, height: 240 },
    ]);
  });
});

describe("restingInterval", () => {
  test("runs at the normal pace while the user is around", () => {
    expect(restingInterval(3000, { resting: false, locked: false })).toBe(3000);
  });

  test("slows down while resting", () => {
    expect(restingInterval(3000, { resting: true, locked: false })).toBe(12000);
  });

  test("stops while locked", () => {
    expect(restingInterval(3000, { resting: true, locked: true })).toBeNull();
  });
});

describe("nextRegion", () => {
  test("a first region is sent as is", () => {
    const next = nextRegion(empty, [big], 0, 700);
    expect(next.changed).toBe(true);
    expect(next.rects).toEqual([big]);
    expect(next.nextExpiry).toBeNull();
  });

  test("the same region again is not a change", () => {
    const first = nextRegion(empty, [big], 0, 700);
    expect(nextRegion(first.state, [big], 10, 700).changed).toBe(false);
  });

  test("a shrinking region keeps the old rectangles until they expire", () => {
    const first = nextRegion(empty, [big], 0, 700);
    const shrunk = nextRegion(first.state, [small], 100, 700);
    expect(shrunk.rects).toEqual([small, big]);
    expect(shrunk.nextExpiry).toBe(800);

    const expired = nextRegion(shrunk.state, [small], 800, 700);
    expect(expired.rects).toEqual([small]);
    expect(expired.changed).toBe(true);
    expect(expired.nextExpiry).toBeNull();
  });

  test("a new display scale counts as a change", () => {
    const first = nextRegion(empty, [big], 0, 700, 1);
    expect(nextRegion(first.state, [big], 10, 700, 1.5).changed).toBe(true);
  });
});
