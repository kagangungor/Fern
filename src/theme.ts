import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { applyBackdrop, enableBackdrop, usesBackdrop } from "./backdrop";

// Convert a hex color string to rgba with a specified alpha
function hexToRgba(hex: string, alpha: number): string {
  hex = hex.replace('#', '');
  if (hex.length === 3) {
    hex = hex.split('').map(char => char + char).join('');
  }
  const r = parseInt(hex.substring(0, 2), 16) || 0;
  const g = parseInt(hex.substring(2, 4), 16) || 0;
  const b = parseInt(hex.substring(4, 6), 16) || 0;
  return `rgba(${r}, ${g}, ${b}, ${alpha})`;
}

export interface HSL {
  h: number;
  s: number;
  l: number;
}

let isSettingsWindow = false;

// Convert Hex to HSL
export function hexToHsl(hex: string): HSL {
  hex = hex.replace('#', '');
  if (hex.length === 3) {
    hex = hex.split('').map(char => char + char).join('');
  }
  const r = (parseInt(hex.substring(0, 2), 16) || 0) / 255;
  const g = (parseInt(hex.substring(2, 4), 16) || 0) / 255;
  const b = (parseInt(hex.substring(4, 6), 16) || 0) / 255;

  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  let h = 0;
  let s = 0;
  const l = (max + min) / 2;

  if (max !== min) {
    const d = max - min;
    s = l > 0.5 ? d / (2 - max - min) : d / (max + min);
    switch (max) {
      case r:
        h = (g - b) / d + (g < b ? 6 : 0);
        break;
      case g:
        h = (b - r) / d + 2;
        break;
      case b:
        h = (r - g) / d + 4;
        break;
    }
    h /= 6;
  }

  return { h: Math.round(h * 360), s: Math.round(s * 100), l: Math.round(l * 100) };
}

// Convert HSL back to Hex color string
function hslToHex(h: number, s: number, l: number): string {
  s /= 100;
  l /= 100;
  const c = (1 - Math.abs(2 * l - 1)) * s;
  const x = c * (1 - Math.abs((h / 60) % 2 - 1));
  const m = l - c / 2;
  let r = 0, g = 0, b = 0;

  if (0 <= h && h < 60) {
    r = c; g = x; b = 0;
  } else if (60 <= h && h < 120) {
    r = x; g = c; b = 0;
  } else if (120 <= h && h < 180) {
    r = 0; g = c; b = x;
  } else if (180 <= h && h < 240) {
    r = 0; g = x; b = c;
  } else if (240 <= h && h < 300) {
    r = x; g = 0; b = c;
  } else if (300 <= h && h < 360) {
    r = c; g = 0; b = x;
  }

  const toHex = (val: number) => {
    const hexVal = Math.round((val + m) * 255).toString(16);
    return hexVal.length === 1 ? '0' + hexVal : hexVal;
  };

  return `#${toHex(r)}${toHex(g)}${toHex(b)}`;
}

export const DEFAULT_ACCENT_COLOR = "#0a84ff";

export const THEME_STYLES = ["solid", "frosted", "liquid", "material"] as const;

// Background opacity each style is designed around; picking a style sets it, the slider can still change it.
export const STYLE_OPACITY: Record<string, number> = {
  solid: 0.8,
  frosted: 0.6,
  liquid: 0.45,
  material: 0.96,
};

function isHex(color: string | null): color is string {
  return !!color && /^#[0-9a-f]{3}([0-9a-f]{3})?$/i.test(color);
}

const hsla = (h: number, s: number, l: number, a = 1) => `hsla(${h}, ${s}%, ${l}%, ${a})`;

// Material You style tonal palette: tinted surfaces and a pastel (dark) or deep (light) primary.
export function materialPalette(seed: string, light: boolean, op: number, opExpanded: number): Record<string, string> {
  const { h, s } = hexToHsl(seed);
  const tint = Math.min(s, 60);
  const shared = { "--fern-accent": seed };
  if (light) {
    return {
      ...shared,
      "--fern-bg": hsla(h, tint * 0.6, 95, op),
      "--fern-bg-expanded": hsla(h, tint * 0.55, 92, opExpanded),
      "--fern-text": hsla(h, 18, 12),
      "--fern-text-muted": hsla(h, 12, 28, 0.75),
      "--fern-border": hsla(h, 15, 30, 0.16),
      "--fern-group-bg": hsla(h, 35, 45, 0.08),
      "--fern-highlight": hsla(h, Math.max(s, 45), 38),
      "--fern-on-highlight": "#ffffff",
    };
  }
  return {
    ...shared,
    "--fern-bg": hsla(h, tint * 0.35, 9, op),
    "--fern-bg-expanded": hsla(h, tint * 0.35, 12, opExpanded),
    "--fern-text": hsla(h, 25, 92),
    "--fern-text-muted": hsla(h, 14, 78, 0.8),
    "--fern-border": hsla(h, 18, 70, 0.16),
    "--fern-group-bg": hsla(h, 35, 70, 0.08),
    "--fern-highlight": hsla(h, Math.max(s, 55), 78),
    "--fern-on-highlight": hsla(h, 55, 16),
  };
}

// The palette grows from the color the user picked, otherwise from the Windows accent.
async function materialSeed(mode: string, customColor: string): Promise<string> {
  if (mode === "custom" && isHex(customColor)) return customColor;
  const accent = await invoke<string>("get_system_accent_color").catch(() => null);
  if (isHex(accent)) return accent;
  return isHex(customColor) ? customColor : DEFAULT_ACCENT_COLOR;
}

let themeGeneration = 0;

export async function applyTheme(
  mode: string,
  customColor: string,
  opacity?: number,
  saturation?: number,
  brightness?: number
) {
  const root = document.documentElement;
  const generation = ++themeGeneration;

  const stored = localStorage.getItem("fern-theme-style") || "solid";
  const style = (THEME_STYLES as readonly string[]).includes(stored) ? stored : "solid";
  if (!isSettingsWindow) {
    root.classList.remove(...THEME_STYLES.map((s) => `style-${s}`));
    root.classList.add(`style-${style}`);
  } else {
    root.classList.remove(...THEME_STYLES.map((s) => `style-${s}`));
    root.classList.add('style-solid');
  }

  if (opacity === undefined || opacity === null) {
    const cachedOpacity = localStorage.getItem('fern-theme-opacity');
    opacity = cachedOpacity !== null ? parseFloat(cachedOpacity) : 0.80;
  }

  if (saturation === undefined || saturation === null) {
    const cachedSaturation = localStorage.getItem('fern-theme-saturation');
    saturation = cachedSaturation !== null ? parseFloat(cachedSaturation) : 0.50;
  }

  if (brightness === undefined || brightness === null) {
    const cachedBrightness = localStorage.getItem('fern-theme-brightness');
    brightness = cachedBrightness !== null ? parseFloat(cachedBrightness) : 0.15;
  }

  const op = opacity;
  const opExpanded = Math.min(1.0, opacity + 0.12);
  root.style.setProperty('--fern-opacity', String(op));
  root.style.setProperty('--fern-opacity-expanded', String(opExpanded));
  root.style.removeProperty('--fern-highlight');
  root.style.removeProperty('--fern-on-highlight');

  if (mode === 'light') {
    root.style.setProperty('--fern-bg', `rgba(255, 255, 255, ${op})`);
    root.style.setProperty('--fern-bg-expanded', `rgba(245, 245, 247, ${opExpanded})`);
    root.style.setProperty('--fern-text', '#1c1c1e');
    root.style.setProperty('--fern-text-muted', 'rgba(28, 28, 30, 0.65)');
    root.style.setProperty('--fern-border', 'rgba(0, 0, 0, 0.12)');
    root.style.setProperty('--fern-group-bg', 'rgba(0, 0, 0, 0.04)');
    root.style.setProperty('--fern-accent', '#007aff');
    root.style.setProperty('--fern-scrollbar-thumb', 'rgba(0, 0, 0, 0.15)');
    root.classList.add('light-mode');
    root.classList.remove('dark-mode');
    root.classList.add('theme-light');
    root.classList.remove('theme-dark', 'theme-custom', 'theme-adaptive');
  } else if (mode === 'custom') {
    const hsl = hexToHsl(customColor);
    hsl.s = saturation * 100;
    hsl.l = brightness * 100;
    const finalBgColor = hslToHex(hsl.h, hsl.s, hsl.l);
    const isLight = brightness > 0.55;

    root.style.setProperty('--fern-bg', hexToRgba(finalBgColor, op));
    root.style.setProperty('--fern-bg-expanded', hexToRgba(finalBgColor, opExpanded));
    root.style.setProperty('--fern-text', isLight ? '#1c1c1e' : '#ffffff');
    root.style.setProperty('--fern-text-muted', isLight ? 'rgba(28, 28, 30, 0.65)' : 'rgba(255, 255, 255, 0.65)');
    root.style.setProperty('--fern-border', isLight ? 'rgba(0, 0, 0, 0.12)' : 'rgba(255, 255, 255, 0.12)');
    root.style.setProperty('--fern-group-bg', isLight ? 'rgba(0, 0, 0, 0.04)' : 'rgba(255, 255, 255, 0.04)');
    root.style.setProperty('--fern-accent', customColor);
    root.style.setProperty('--fern-scrollbar-thumb', isLight ? 'rgba(0, 0, 0, 0.15)' : 'rgba(255, 255, 255, 0.15)');
    root.classList.add('theme-custom');
    root.classList.remove('theme-light', 'theme-dark', 'theme-adaptive');
    if (isLight) {
      root.classList.add('light-mode');
      root.classList.remove('dark-mode');
    } else {
      root.classList.add('dark-mode');
      root.classList.remove('light-mode');
    }
  } else if (mode === 'adaptive') {
    try {
      const accentHex = await invoke<string>('get_system_accent_color');
      if (generation !== themeGeneration) return;
      const hsl = hexToHsl(accentHex);
      hsl.s = saturation * 100;
      hsl.l = brightness * 100;
      const finalBgColor = hslToHex(hsl.h, hsl.s, hsl.l);
      const isLight = brightness > 0.55;

      root.style.setProperty('--fern-bg', hexToRgba(finalBgColor, op));
      root.style.setProperty('--fern-bg-expanded', hexToRgba(finalBgColor, opExpanded));
      root.style.setProperty('--fern-text', isLight ? '#1c1c1e' : '#ffffff');
      root.style.setProperty('--fern-text-muted', isLight ? 'rgba(28, 28, 30, 0.65)' : 'rgba(255, 255, 255, 0.65)');
      root.style.setProperty('--fern-border', isLight ? 'rgba(0, 0, 0, 0.12)' : 'rgba(255, 255, 255, 0.12)');
      root.style.setProperty('--fern-group-bg', isLight ? 'rgba(0, 0, 0, 0.04)' : 'rgba(255, 255, 255, 0.04)');
      root.style.setProperty('--fern-accent', accentHex);
      root.style.setProperty('--fern-scrollbar-thumb', isLight ? 'rgba(0, 0, 0, 0.15)' : 'rgba(255, 255, 255, 0.15)');
      root.classList.add('theme-adaptive');
      root.classList.remove('theme-light', 'theme-dark', 'theme-custom');
      if (isLight) {
        root.classList.add('light-mode');
        root.classList.remove('dark-mode');
      } else {
        root.classList.add('dark-mode');
        root.classList.remove('light-mode');
      }
    } catch (e) {
      console.error("Failed to apply adaptive theme:", e);
      return applyTheme('dark', customColor, op, saturation, brightness);
    }
  } else {
    // default/dark
    root.style.setProperty('--fern-bg', `rgba(0, 0, 0, ${op})`);
    root.style.setProperty('--fern-bg-expanded', `rgba(0, 0, 0, ${opExpanded})`);
    root.style.setProperty('--fern-text', '#ffffff');
    root.style.setProperty('--fern-text-muted', 'rgba(255, 255, 255, 0.6)');
    root.style.setProperty('--fern-border', 'rgba(255, 255, 255, 0.1)');
    root.style.setProperty('--fern-group-bg', 'rgba(255, 255, 255, 0.04)');
    root.style.setProperty('--fern-accent', '#007aff');
    root.style.setProperty('--fern-scrollbar-thumb', 'rgba(255, 255, 255, 0.1)');
    root.classList.add('dark-mode');
    root.classList.remove('light-mode');
    root.classList.add('theme-dark');
    root.classList.remove('theme-light', 'theme-custom', 'theme-adaptive');
  }

  if (!isSettingsWindow && style === "material") {
    const seed = await materialSeed(mode, customColor);
    if (generation !== themeGeneration) return;
    const palette = materialPalette(seed, root.classList.contains('light-mode'), op, opExpanded);
    for (const [name, value] of Object.entries(palette)) root.style.setProperty(name, value);
  }
  if (!isSettingsWindow) {
    await applyBackdrop(root, style);
  }
}

// `backdrop`: this window draws the notch or dock, which show the wallpaper in the glass styles.
// `isSettings`: Settings window keeps its own clean fixed appearance and ignores theme style changes
export function initTheme(options: { backdrop?: boolean; isSettings?: boolean } = {}) {
  if (options.backdrop) enableBackdrop();
  if (options.isSettings) isSettingsWindow = true;
  const syncMode = localStorage.getItem('fern-theme-mode') || 'dark';
  const syncColor = localStorage.getItem('fern-theme-color') || '#007aff';
  const syncOpacityVal = localStorage.getItem('fern-theme-opacity');
  const syncOpacity = syncOpacityVal !== null ? parseFloat(syncOpacityVal) : 0.80;
  const syncSaturationVal = localStorage.getItem('fern-theme-saturation');
  const syncSaturation = syncSaturationVal !== null ? parseFloat(syncSaturationVal) : 0.50;
  const syncBrightnessVal = localStorage.getItem('fern-theme-brightness');
  const syncBrightness = syncBrightnessVal !== null ? parseFloat(syncBrightnessVal) : 0.15;

  // Fast synchronous draw using cached localStorage
  applyTheme(syncMode, syncColor, syncOpacity, syncSaturation, syncBrightness);

  // Sync settings.json in background to avoid flickers
  invoke<Record<string, any>>('load_settings').then((settings) => {
    const mode = settings['fern-theme-mode'] ? String(settings['fern-theme-mode']) : syncMode;
    const color = settings['fern-theme-color'] ? String(settings['fern-theme-color']) : syncColor;
    const opacityVal = settings['fern-theme-opacity'] ? parseFloat(String(settings['fern-theme-opacity'])) : syncOpacity;
    const saturationVal = settings['fern-theme-saturation'] ? parseFloat(String(settings['fern-theme-saturation'])) : syncSaturation;
    const brightnessVal = settings['fern-theme-brightness'] ? parseFloat(String(settings['fern-theme-brightness'])) : syncBrightness;

    let surfaceChanged = false;
    for (const key of ['fern-theme-style', 'fern-glass-wallpaper']) {
      const val = settings[key];
      if (val !== undefined && val !== null && String(val) !== localStorage.getItem(key)) {
        localStorage.setItem(key, String(val));
        surfaceChanged = true;
      }
    }

    if (
      mode !== syncMode ||
      color !== syncColor ||
      opacityVal !== syncOpacity ||
      saturationVal !== syncSaturation ||
      brightnessVal !== syncBrightness ||
      surfaceChanged
    ) {
      localStorage.setItem('fern-theme-mode', mode);
      localStorage.setItem('fern-theme-color', color);
      localStorage.setItem('fern-theme-opacity', String(opacityVal));
      localStorage.setItem('fern-theme-saturation', String(saturationVal));
      localStorage.setItem('fern-theme-brightness', String(brightnessVal));
      applyTheme(mode, color, opacityVal, saturationVal, brightnessVal);
    }
  }).catch(console.error);

  // Helper: read all theme values from localStorage and apply
  const applyThemeFromStorage = () => {
    const mode = localStorage.getItem('fern-theme-mode') || 'dark';
    const color = localStorage.getItem('fern-theme-color') || '#007aff';
    const opacity = parseFloat(localStorage.getItem('fern-theme-opacity') || '0.80');
    const saturation = parseFloat(localStorage.getItem('fern-theme-saturation') || '0.50');
    const brightness = parseFloat(localStorage.getItem('fern-theme-brightness') || '0.15');
    applyTheme(mode, color, opacity, saturation, brightness);
  };

  const fernThemeKeys = [
    'fern-theme-mode',
    'fern-theme-color',
    'fern-theme-opacity',
    'fern-theme-saturation',
    'fern-theme-brightness',
    'fern-theme-style',
    'fern-glass-wallpaper',
    'fern-scale',
  ];

  // Listen to setting changes broadcasted from settings window
  // (localStorage sync is handled by saveAndLocal in Settings.tsx; we just apply the theme)
  const settingsPromise = listen<{ key: string; value: any }>('settings-changed', (event) => {
    const { key, value } = event.payload;
    if (fernThemeKeys.includes(key)) {
      if (value !== undefined && value !== null) {
        localStorage.setItem(key, String(value));
      }
      applyThemeFromStorage();
    }
  });

  // Windows sends this for accent, wallpaper and other system setting changes
  const accentPromise = listen<string>('system-accent-changed', () => {
    const style = localStorage.getItem('fern-theme-style');
    if ((localStorage.getItem('fern-theme-mode') || 'dark') === 'adaptive' || style === 'material' || usesBackdrop(style)) {
      applyThemeFromStorage();
    }
  });

  // The wallpaper backdrop is lined up with the window's place on its monitor
  const monitorsPromise = listen('monitors-changed', () => {
    if (usesBackdrop(localStorage.getItem('fern-theme-style'))) applyThemeFromStorage();
  });

  // Listen to external edits (e.g. settings.json edited outside the app)
  // localStorage sync is handled by useSettingsSync hook; we just apply the theme
  const externalPromise = listen<{ key: string; value: any }>('settings-external-changed', (event) => {
    const { key, value } = event.payload;
    if (fernThemeKeys.includes(key)) {
      if (value !== undefined && value !== null) {
        localStorage.setItem(key, String(value));
      }
      applyThemeFromStorage();
    }
  });

  return () => {
    settingsPromise.then(f => f());
    accentPromise.then(f => f());
    externalPromise.then(f => f());
    monitorsPromise.then(f => f());
  };
}
