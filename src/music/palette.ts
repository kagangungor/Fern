// Color grading from artwork, after Panic's iTunes 11 algorithm
// (https://blog.panic.com/itunes-11-and-colors/): the most common perceptually distinct colors
// (YUV distance) become card + accent, with white or black text, whichever reads.
// Adapted from Nectar (GPL-3.0).

export interface ArtTheme {
  background: string; // CSS color, "" = keep the default card
  text: string; // near-white or near-black, whichever reads on the card
  accent: string; // CSS color for progress fill and visualizer
}

export const FALLBACK_THEME: ArtTheme = { background: "", text: "", accent: "" };

export type RGB = [number, number, number];

export interface Bucket {
  color: RGB;
  n: number; // pixel count
}

// WCAG 2.x relative luminance.
export function luminance([r, g, b]: RGB): number {
  const f = (c: number) => {
    const s = c / 255;
    return s <= 0.03928 ? s / 12.92 : Math.pow((s + 0.055) / 1.055, 2.4);
  };
  return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b);
}

export function contrast(l1: number, l2: number): number {
  const [hi, lo] = l1 > l2 ? [l1, l2] : [l2, l1];
  return (hi + 0.05) / (lo + 0.05);
}

// YUV separates "are these colors different" better than RGB.
export function yuvDistance(a: RGB, b: RGB): number {
  const [r1, g1, b1] = a.map((v) => v / 255);
  const [r2, g2, b2] = b.map((v) => v / 255);
  const dy = 0.299 * (r1 - r2) + 0.587 * (g1 - g2) + 0.114 * (b1 - b2);
  const du = -0.14713 * (r1 - r2) - 0.28886 * (g1 - g2) + 0.436 * (b1 - b2);
  const dv = 0.615 * (r1 - r2) - 0.51499 * (g1 - g2) - 0.10001 * (b1 - b2);
  return Math.hypot(dy, du, dv);
}

function mix(a: RGB, b: RGB, t: number): RGB {
  return [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t];
}

const css = (c: RGB) => `rgb(${c.map(Math.round).join(" ")})`;

// Smaller grids average away thin features, the small vivid regions good accents come from.
const SIZE = 64;
// Minimum YUV distance between two picked colors.
const DISTINCT = 0.25;
// A bucket with every channel below this is black as far as the card is concerned.
const DARK_CHANNEL = 48;
// How much of the cover has to be black before it counts as a black cover, rather than a colorful
// one sitting in shadow.
const DARK_SHARE = 0.7;
const MIN_CONTRAST = 4.5;

const WHITE: RGB = [250, 250, 250];
const BLACK: RGB = [18, 18, 18];
const WHITE_L = luminance(WHITE);
const BLACK_L = luminance(BLACK);
// The dark base the card is anchored to, and how much of the cover color survives over it.
const NEUTRAL: RGB = [23, 23, 23];
const BG_STRENGTH = 0.75;

const isBlack = ([r, g, b]: RGB) => Math.max(r, g, b) < DARK_CHANNEL;
// Colorfulness (channel spread), the tiebreaker between equally readable accents.
const chroma = ([r, g, b]: RGB) => Math.max(r, g, b) - Math.min(r, g, b);

// Tallies pixels (RGBA) into 12-bit color buckets, most frequent first.
export function tallyPixels(data: ArrayLike<number>): Bucket[] {
  const tallies = new Map<number, { n: number; r: number; g: number; b: number }>();
  for (let i = 0; i + 3 < data.length; i += 4) {
    if (data[i + 3] < 128) continue;
    const key = ((data[i] >> 4) << 8) | ((data[i + 1] >> 4) << 4) | (data[i + 2] >> 4);
    const t = tallies.get(key) ?? { n: 0, r: 0, g: 0, b: 0 };
    t.n++;
    t.r += data[i];
    t.g += data[i + 1];
    t.b += data[i + 2];
    tallies.set(key, t);
  }
  return [...tallies.values()]
    .sort((a, b) => b.n - a.n)
    .map((t) => ({ color: [t.r / t.n, t.g / t.n, t.b / t.n] as RGB, n: t.n }));
}

// Card color over the dark base, nudged until white or black text passes 4.5:1. The accent candidate
// needing the fewest nudges wins; ties go to the more colorful one.
function finish(bgSource: RGB, accentCandidates: RGB[]): ArtTheme {
  let bg = mix(NEUTRAL, bgSource, BG_STRENGTH);
  const towardWhite = contrast(luminance(bg), WHITE_L) >= contrast(luminance(bg), BLACK_L);
  const textColor = towardWhite ? WHITE : BLACK;
  for (let i = 0; i < 40 && contrast(luminance(bg), towardWhite ? WHITE_L : BLACK_L) < MIN_CONTRAST; i++) {
    bg = mix(bg, towardWhite ? BLACK : WHITE, 0.05);
  }
  const bgL = luminance(bg);

  const nudge = (color: RGB) => {
    let value = color;
    let steps = 0;
    while (steps < 12 && contrast(luminance(value), bgL) < MIN_CONTRAST) {
      value = mix(value, textColor, 0.15);
      steps++;
    }
    return contrast(luminance(value), bgL) >= MIN_CONTRAST ? { base: color, value, steps } : null;
  };
  const best = accentCandidates
    .map(nudge)
    .filter((c) => c !== null)
    .sort((a, b) => a.steps - b.steps || chroma(b.base) - chroma(a.base))[0];

  return { background: css(bg), text: css(textColor), accent: css(best?.value ?? textColor) };
}

export function themeFromBuckets(buckets: Bucket[]): ArtTheme {
  if (buckets.length === 0) return FALLBACK_THEME;

  // Three most common distinct colors: the card, then two accent candidates.
  const distinct: Bucket[] = [];
  for (const b of buckets) {
    if (distinct.every((d) => yuvDistance(d.color, b.color) >= DISTINCT)) {
      distinct.push(b);
      if (distinct.length === 3) break;
    }
  }

  // Dark covers put black in the top bucket, and a black card throws the cover's color away. When
  // black really is most of the cover that's the honest answer; otherwise it's only the shadow the
  // color lives in, so the most common non-black color is promoted. That one comes from the raw list:
  // DISTINCT also measures distance from black, which is what would drop a dark navy cover's color.
  const pixels = buckets.reduce((n, b) => n + b.n, 0);
  const blacks = buckets.reduce((n, b) => n + (isBlack(b.color) ? b.n : 0), 0);
  const color = buckets.find((b) => !isBlack(b.color));
  const bg = blacks < pixels * DARK_SHARE && color ? color : distinct[0];

  return finish(bg.color, distinct.filter((b) => b !== bg).map((b) => b.color));
}

export async function artTheme(art: string): Promise<ArtTheme> {
  try {
    const img = new Image();
    img.src = art;
    await img.decode();
    const canvas = new OffscreenCanvas(SIZE, SIZE);
    const ctx = canvas.getContext("2d", { willReadFrequently: true });
    if (!ctx) return FALLBACK_THEME;
    ctx.drawImage(img, 0, 0, SIZE, SIZE);
    return themeFromBuckets(tallyPixels(ctx.getImageData(0, 0, SIZE, SIZE).data));
  } catch {
    return FALLBACK_THEME;
  }
}
