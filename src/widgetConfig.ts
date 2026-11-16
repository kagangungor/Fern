export interface WidgetConfig {
  left: string[];
  right: string[];
}

export const MAX_WIDGETS_PER_ZONE = 2;

export function normalizeWidgetConfig(cfg: WidgetConfig): WidgetConfig {
  return {
    left: cfg.left.slice(0, MAX_WIDGETS_PER_ZONE),
    right: cfg.right.slice(0, MAX_WIDGETS_PER_ZONE),
  };
}

export function swapWidgetSide(cfg: WidgetConfig, id: string): WidgetConfig {
  if (cfg.left.includes(id)) {
    if (cfg.right.length >= MAX_WIDGETS_PER_ZONE) return cfg;
    return { left: cfg.left.filter((x) => x !== id), right: [...cfg.right, id] };
  }
  if (cfg.right.includes(id)) {
    if (cfg.left.length >= MAX_WIDGETS_PER_ZONE) return cfg;
    return { right: cfg.right.filter((x) => x !== id), left: [...cfg.left, id] };
  }
  return cfg;
}

export function computeWidgetsRowWidth(
  leftCount: number,
  rightCount: number,
  netExtra: number,
  dualClockExtra: number,
  privacyExtra: number
): number {
  if (leftCount + rightCount <= 0) return 0;
  const symmetricCount = Math.max(leftCount, rightCount) * 2;
  return Math.min(200 + symmetricCount * 65 + netExtra, 530) + dualClockExtra + privacyExtra;
}
