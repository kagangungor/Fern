export interface DockAppIdentity {
  path: string;
  executable?: string;
  name: string;
}

export function parseDockNames(raw: unknown): Record<string, string> {
  if (raw && typeof raw === "object" && !Array.isArray(raw)) {
    return raw as Record<string, string>;
  }
  if (typeof raw !== "string") return {};
  try {
    const parsed = JSON.parse(raw);
    return parsed && typeof parsed === "object" && !Array.isArray(parsed) ? parsed : {};
  } catch {
    return {};
  }
}

export function applyDockNameOverride(
  names: Record<string, string>,
  key: string,
  defaultName: string,
  nextName: string
): Record<string, string> {
  const next = { ...names };
  const trimmed = nextName.trim();
  if (trimmed && trimmed !== defaultName) {
    next[key] = trimmed;
  } else {
    delete next[key];
  }
  return next;
}

export function resolveDockDisplayName(
  app: { name: string },
  key: string,
  names: Record<string, string>,
  localize: (name: string) => string
): string {
  return names[key] || localize(app.name);
}

export function isAppHiddenInDock(
  app: DockAppIdentity,
  itemKey: string,
  hiddenApps: string[]
): boolean {
  if (hiddenApps.length === 0) return false;
  const key0 = itemKey.toLowerCase();
  const key1 = app.path?.toLowerCase();
  const key2 = app.executable?.toLowerCase();
  const key3 = app.name?.toLowerCase();
  return hiddenApps.some((h) => {
    const hl = h.toLowerCase();
    return hl === key0 || hl === key1 || hl === key2 || hl === key3;
  });
}

export function hideIdentifierForApp(
  app: DockAppIdentity,
  itemKey: string,
  isBrowserHostApp: boolean
): string {
  if (isBrowserHostApp) return itemKey;
  return app.executable || app.path || app.name;
}
