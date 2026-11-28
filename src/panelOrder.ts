export const DEFAULT_PANEL_ORDER = ["command-center", "status", "music", "calendar", "shelf", "ai"] as const;

export type PanelId = (typeof DEFAULT_PANEL_ORDER)[number];

export function parsePanelOrder(raw: string | null | undefined): PanelId[] | null {
  if (!raw) return null;
  try {
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return null;
    const known = parsed.filter((id): id is PanelId => (DEFAULT_PANEL_ORDER as readonly string[]).includes(id));
    const unique = Array.from(new Set(known));
    return [...unique, ...DEFAULT_PANEL_ORDER.filter((id) => !unique.includes(id))];
  } catch {
    return null;
  }
}

export const CC_TILES = ["network", "dock-mode", "bluetooth", "notch-mode"] as const;
export const CC_BUTTONS = ["dnd", "battery-saver", "tray", "notifications", "settings", "restart"] as const;
export const CC_SLIDERS = ["volume", "brightness"] as const;

export type CcKind = "tiles" | "buttons" | "sliders";
export type CcLayout = Partial<Record<CcKind, string[]>>;

export const CC_DEFAULTS: Record<CcKind, readonly string[]> = {
  tiles: CC_TILES,
  buttons: CC_BUTTONS,
  sliders: CC_SLIDERS,
};

function valid(saved: unknown, defaults: readonly string[]): string[] | undefined {
  if (!Array.isArray(saved)) return undefined;
  return Array.from(new Set(saved.filter((id): id is string => defaults.includes(id as string))));
}

export function parseCcLayout(raw: string | null | undefined): CcLayout {
  if (!raw) return {};
  try {
    const parsed = JSON.parse(raw);
    return {
      tiles: valid(parsed?.tiles, CC_TILES),
      buttons: valid(parsed?.buttons, CC_BUTTONS),
      sliders: valid(parsed?.sliders, CC_SLIDERS),
    };
  } catch {
    return {};
  }
}

export function resolveCcLayout(layout: CcLayout): Record<CcKind, string[]> {
  return {
    tiles: layout.tiles ?? [...CC_TILES],
    buttons: layout.buttons ?? [...CC_BUTTONS],
    sliders: layout.sliders ?? [...CC_SLIDERS],
  };
}

export type ScrollDirection = "natural" | "inverted";

export function parseScrollDirection(raw: string | null | undefined): ScrollDirection {
  return raw === "inverted" ? "inverted" : "natural";
}

export function panelScrollStep(accumDelta: number, direction: ScrollDirection): 1 | -1 {
  const step = accumDelta > 0 ? 1 : -1;
  return direction === "inverted" ? (step === 1 ? -1 : 1) : step;
}

const LAST_PANEL_KEY = "fern-notch-last-panel";

export function readRememberedPanel(available: readonly PanelId[]): PanelId | null {
  try {
    const raw = localStorage.getItem(LAST_PANEL_KEY);
    if (!raw) return null;
    return available.includes(raw as PanelId) ? (raw as PanelId) : null;
  } catch {
    return null;
  }
}

export function resolveInitialHoverPanel(
  available: readonly PanelId[],
  remembered: PanelId | null,
): PanelId {
  if (remembered && available.includes(remembered)) return remembered;
  return available[0] ?? "status";
}

export function saveRememberedPanel(panel: string): void {
  if (!(DEFAULT_PANEL_ORDER as readonly string[]).includes(panel)) return;
  try {
    localStorage.setItem(LAST_PANEL_KEY, panel);
  } catch { /* storage unavailable */ }
}
