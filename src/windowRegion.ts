// Computes the window regions sent to `set_window_regions` (see window_region.rs).
// The hold-while-shrinking logic is adapted from Bloom (GPL-3.0).

/** A rectangle in a window's CSS pixels. */
export interface RegionRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

interface HeldRegion {
  rects: RegionRect[];
  until: number;
}

export interface RegionState {
  key: string;
  current: RegionRect[];
  held: HeldRegion[];
}

export function padRect(rect: RegionRect, pad: number): RegionRect {
  return { x: rect.x - pad, y: rect.y - pad, width: rect.width + pad * 2, height: rect.height + pad * 2 };
}

/** Whole pixels, rounded outwards; null if nothing of it is inside the window. */
export function clipToWindow(rect: RegionRect, width: number, height: number): RegionRect | null {
  const left = Math.max(0, Math.floor(rect.x));
  const top = Math.max(0, Math.floor(rect.y));
  const right = Math.min(width, Math.ceil(rect.x + rect.width));
  const bottom = Math.min(height, Math.ceil(rect.y + rect.height));
  if (right <= left || bottom <= top) return null;
  return { x: left, y: top, width: right - left, height: bottom - top };
}

export function fullWindow(width: number, height: number): RegionRect {
  return { x: 0, y: 0, width, height };
}

/**
 * The notch pill's box from the size it is animating toward. The pill is centered and hangs from the
 * top edge; `pad` covers its shadow, its glow and the spring's overshoot.
 */
export function notchBox(windowWidth: number, scale: number, pillWidth: number, pillHeight: number, pad: number): RegionRect {
  const width = (pillWidth + pad * 2) * scale;
  return { x: (windowWidth - width) / 2, y: 0, width, height: (pillHeight + pad) * scale };
}

/**
 * A band along the bottom edge for the dock, as wide as the widest the bar has been and as tall as the
 * tallest plus `lift` for the icons rising above it, so its reveal and width animations aren't clipped.
 * The bar is centered in the window; `widthPad` is the room on each side of it.
 */
export function dockBand(
  windowWidth: number,
  windowHeight: number,
  barWidth: number,
  barHeight: number,
  widthPad: number,
  lift: number,
): RegionRect {
  const width = barWidth + widthPad * 2;
  const height = barHeight + lift;
  return { x: (windowWidth - width) / 2, y: windowHeight - height, width, height };
}

/**
 * The volume and brightness cards: a vertically centered strip at each side of the window. Both are
 * always included, so a switch between them can't race the region. `pad` covers their flares and blur.
 */
export function edgeCards(
  windowWidth: number,
  windowHeight: number,
  scale: number,
  cardWidth: number,
  cardHeight: number,
  pad: number,
): RegionRect[] {
  const width = (cardWidth + pad) * scale;
  const height = (cardHeight + pad * 2) * scale;
  const y = (windowHeight - height) / 2;
  return [
    { x: 0, y, width, height },
    { x: windowWidth - width, y, width, height },
  ];
}

/** Boxes for the elements matching each selector, padded for what they paint outside themselves. */
export function measureElements(selectors: [string, number][]): RegionRect[] {
  return selectors.flatMap(([selector, pad]) =>
    Array.from(document.querySelectorAll(selector)).map((el) => {
      const r = el.getBoundingClientRect();
      return padRect({ x: r.left, y: r.top, width: r.width, height: r.height }, pad);
    }),
  );
}

function regionKey(rects: RegionRect[]): string {
  return rects.map((r) => `${r.x},${r.y},${r.width},${r.height}`).join(";");
}

/**
 * Folds the rectangles the window needs now into its region. A region that grows takes effect at once;
 * when it shrinks, the previous rectangles are held for `holdMs` so an animation leaving them isn't
 * clipped. `changed` says whether the region differs from the last one sent, and `nextExpiry` when a
 * held region runs out and the region should be recomputed. The backend turns the rectangles into
 * physical pixels, so a change of `pixelRatio` also counts as a change.
 */
export function nextRegion(
  state: RegionState,
  current: RegionRect[],
  now: number,
  holdMs: number,
  pixelRatio = 1,
): { state: RegionState; rects: RegionRect[]; changed: boolean; nextExpiry: number | null } {
  let held = state.held;
  if (state.current.length > 0 && regionKey(current) !== regionKey(state.current)) {
    held = [...held, { rects: state.current, until: now + holdMs }];
  }
  held = held.filter((h) => h.until > now);
  const rects = [...current, ...held.flatMap((h) => h.rects)];
  const key = `${pixelRatio}|${regionKey(rects)}`;
  const nextExpiry = held.length > 0 ? Math.min(...held.map((h) => h.until)) : null;
  return { state: { key, current, held }, rects, changed: key !== state.key, nextExpiry };
}

/**
 * Returns a function taking the rectangles a window needs. It clips them to the window and calls `send`
 * whenever the region changes, recomputing on its own when a held region expires or the display scale
 * changes. Meant to live as long as its window.
 */
export function createRegionSender(send: (rects: RegionRect[]) => void, holdMs = 700) {
  let state: RegionState = { key: "", current: [], held: [] };
  let latest: RegionRect[] = [];
  let timer: ReturnType<typeof setTimeout> | null = null;
  const update = (wanted: RegionRect[]) => {
    latest = wanted;
    const current = wanted
      .map((r) => clipToWindow(r, window.innerWidth, window.innerHeight))
      .filter((r): r is RegionRect => r !== null);
    const next = nextRegion(state, current, performance.now(), holdMs, window.devicePixelRatio);
    state = next.state;
    if (next.changed && next.rects.length > 0) send(next.rects);
    if (timer) clearTimeout(timer);
    timer = next.nextExpiry === null ? null : setTimeout(() => update(latest), next.nextExpiry - performance.now() + 5);
  };
  // A resolution query only matches the ratio it was made for, so each change re-arms it.
  const watchPixelRatio = () => {
    const query = window.matchMedia(`(resolution: ${window.devicePixelRatio}dppx)`);
    query.addEventListener(
      "change",
      () => {
        watchPixelRatio();
        update(latest);
      },
      { once: true },
    );
  };
  watchPixelRatio();
  return update;
}
