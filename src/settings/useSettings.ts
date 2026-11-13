import { useState, useEffect, useCallback } from "react";
import { parseCcLayout, parsePanelOrder, type CcLayout, type PanelId } from "../panelOrder";
import { invoke } from "@tauri-apps/api/core";
import { listen, emit } from "@tauri-apps/api/event";

import { getVersion } from "@tauri-apps/api/app";
import type { UpdateCheckResult } from "../updater";
import { useSettingsSync } from "../hooks/useSettingsSync";
import { hexToHsl, STYLE_OPACITY } from "../theme";
import { DEFAULT_DUAL_CLOCK_ZONE } from "../dualClock";
import type { WidgetConfig, MonitorInfo, MonitorMode } from "./types";

function saveSetting(key: string, value: string) {
  localStorage.setItem(key, value);
  invoke("save_setting", { key, value }).catch(console.error);
}

function readBool(val: string | null): boolean {
  return val === "true";
}

function parseDrives(val: string | null): string[] {
  try {
    const parsed = JSON.parse(val ?? "[]");
    return Array.isArray(parsed) ? parsed.filter((d): d is string => typeof d === "string") : [];
  } catch {
    return [];
  }
}

export const DOCK_MENU_IDS = ["calendar", "settings", "taskmgr", "restart", "clearicons", "quit"];

function parseMenuItems(val: unknown): string[] {
  try {
    const parsed = typeof val === "string" ? JSON.parse(val) : val;
    return Array.isArray(parsed) ? parsed.filter((id): id is string => DOCK_MENU_IDS.includes(id)) : DOCK_MENU_IDS;
  } catch {
    return DOCK_MENU_IDS;
  }
}

export function useSettings() {
  const [autostart, setAutostart] = useState(false);
  const [weatherEnabled, setWeatherEnabled] = useState(true);
  const [calendarEnabled, setCalendarEnabled] = useState(true);
  const [timerEnabled, setTimerEnabled] = useState(true);
  const [timerSoundEnabled, setTimerSoundEnabled] = useState(true);
  const [notchTimerProgress, setNotchTimerProgress] = useState(
    () => localStorage.getItem("fern-notch-timer-progress") !== "false"
  );
  const [stopwatchEnabled, setStopwatchEnabled] = useState(true);
  const [dualClockEnabled, setDualClockEnabled] = useState(
    () => localStorage.getItem("fern-dual-clock-enabled") === "true"
  );
  const [dualClockZone, setDualClockZone] = useState(
    () => localStorage.getItem("fern-dual-clock-zone") || DEFAULT_DUAL_CLOCK_ZONE
  );
  const [dualClockLabel, setDualClockLabel] = useState(
    () => localStorage.getItem("fern-dual-clock-label") || DEFAULT_DUAL_CLOCK_ZONE
  );
  const [musicModeEnabled, setMusicModeEnabled] = useState(true);
  const [musicCompactNotch, setMusicCompactNotch] = useState(true);
  const [notchScrollVolume, setNotchScrollVolume] = useState(
    () => localStorage.getItem("fern-notch-scroll-volume") === "true"
  );
  const [rememberMode, setRememberMode] = useState(
    () => localStorage.getItem("fern-notch-remember-mode") === "true"
  );
  const [shelfPersist, setShelfPersist] = useState(
    () => localStorage.getItem("fern-shelf-persist") === "true"
  );
  const [shelfKeepOpen, setShelfKeepOpen] = useState(
    () => localStorage.getItem("fern-shelf-keep-open") === "true"
  );
  const [volumeOverlayEnabled, setVolumeOverlayEnabled] = useState(true);
  const [volumeEdgeEnabled, setVolumeEdgeEnabled] = useState(
    () => localStorage.getItem("fern-volume-edge-enabled") !== "false"
  );
  const [brightnessOverlayEnabled, setBrightnessOverlayEnabled] = useState(
    () => localStorage.getItem("fern-brightness-overlay-enabled") !== "false"
  );
  const [brightnessEdgeEnabled, setBrightnessEdgeEnabled] = useState(
    () => localStorage.getItem("fern-brightness-edge-enabled") !== "false"
  );
  const [agentApiEnabled, setAgentApiEnabled] = useState(
    () => localStorage.getItem("fern-agent-api") === "true"
  );
  const [agentSoundsEnabled, setAgentSoundsEnabled] = useState(
    () => localStorage.getItem("fern-agent-sounds") !== "false"
  );
  const [notchEdgeDelay, setNotchEdgeDelay] = useState(
    () => localStorage.getItem("fern-notch-edge-delay") || "0"
  );
  const [volumeSide, setVolumeSide] = useState<"left" | "right">(
    () => (localStorage.getItem("fern-volume-side") as "left" | "right") || "left"
  );
  const [brightnessSide, setBrightnessSide] = useState<"left" | "right">(
    () => (localStorage.getItem("fern-brightness-side") as "left" | "right") || "right"
  );
  const [edgeDelay, setEdgeDelay] = useState(
    () => localStorage.getItem("fern-edge-delay") || "0"
  );
  const [dockAttentionEnabled, setDockAttentionEnabled] = useState(
    () => localStorage.getItem("fern-dock-attention-enabled") !== "false"
  );
  const [dockHideDelay, setDockHideDelay] = useState(
    () => localStorage.getItem("fern-dock-hide-delay") || "800"
  );
  const [glassWallpaperEnabled, setGlassWallpaperEnabled] = useState(
    () => localStorage.getItem("fern-glass-wallpaper") !== "false"
  );
  const [mediaAmbienceEnabled, setMediaAmbienceEnabled] = useState(true);
  const [artworkThemeEnabled, setArtworkThemeEnabled] = useState(true);
  const [mediaCompactGlowEnabled, setMediaCompactGlowEnabled] = useState(true);
  const [lyricsEnabled, setLyricsEnabled] = useState(
    () => localStorage.getItem("fern-lyrics-enabled") !== "false"
  );
  const [lyricsSeekEnabled, setLyricsSeekEnabled] = useState(
    () => localStorage.getItem("fern-lyrics-seek-enabled") !== "false"
  );
  const [mediaLayout, setMediaLayout] = useState<"classic" | "compact">(
    () => (localStorage.getItem("fern-media-layout") as "classic" | "compact") || "classic"
  );
  const [cornersEnabled, setCornersEnabled] = useState(
    () => localStorage.getItem("fern-corners-enabled") === "true"
  );
  const [showUpdateIndicator, setShowUpdateIndicator] = useState(
    () => localStorage.getItem("fern-show-update-indicator") !== "false"
  );
  const [weekStart, setWeekStart] = useState<"auto" | "sunday" | "monday">(() => (localStorage.getItem("fern-week-start") as "auto" | "sunday" | "monday") || "auto");
  const [timeFormat24h, setTimeFormat24h] = useState(
    () => localStorage.getItem("fern-time-format-24h") === "true"
  );
  const [memorySaver, setMemorySaver] = useState(
    () => localStorage.getItem("fern-memory-saver") === "true"
  );
  const [tempUnitFahrenheit, setTempUnitFahrenheit] = useState(false);
  const [cityName, setCityName] = useState("");
  const [cityQuery, setCityQuery] = useState("");
  const [citySearchResults, setCitySearchResults] = useState<
    Array<{ name: string; country: string; latitude: number; longitude: number }>
  >([]);
  const [showCityDropdown, setShowCityDropdown] = useState(false);
  const [statusWidgets, setStatusWidgets] = useState<WidgetConfig>({
    left: ["weather"],
    right: ["battery"],
  });
  const [dockEnabled, setDockEnabled] = useState(true);
  const [dockPreviewEnabled, setDockPreviewEnabled] = useState(true);
  const [dockConsoleWindows, setDockConsoleWindows] = useState(() => localStorage.getItem("fern-dock-console-windows") === "true");
  const [dockAppVolumeScroll, setDockAppVolumeScroll] = useState(() => localStorage.getItem("fern-dock-app-volume-scroll") !== "false");
  const [dockSearchEnabled, setDockSearchEnabled] = useState(true);
  const [dockSearchLauncher, setDockSearchLauncher] = useState(
    () => localStorage.getItem("fern-search-launcher") || "windows"
  );
  const [dockCalendarEnabled, setDockCalendarEnabled] = useState(true);
  const [dockTray, setDockTray] = useState(
    () => localStorage.getItem("fern-dock-tray-enabled") !== "false"
  );
  const [dockShowDesktop, setDockShowDesktop] = useState(
    () => localStorage.getItem("fern-dock-show-desktop-corner") === "true"
  );
  const [panelControlCenter, setPanelControlCenter] = useState(
    () => localStorage.getItem("fern-panel-control-center-enabled") !== "false"
  );
  const [panelOrder, setPanelOrder] = useState<PanelId[] | null>(
    () => parsePanelOrder(localStorage.getItem("fern-panel-order"))
  );
  const [ccLayout, setCcLayout] = useState<CcLayout>(
    () => parseCcLayout(localStorage.getItem("fern-cc-layout"))
  );
  const [panelShelf, setPanelShelf] = useState(
    () => localStorage.getItem("fern-panel-shelf-enabled") !== "false"
  );
  const [dockTaskView, setDockTaskView] = useState(
    () => localStorage.getItem("fern-dock-taskview-enabled") === "true"
  );
  const [dockRecycle, setDockRecycle] = useState(
    () => localStorage.getItem("fern-dock-recycle-enabled") === "true"
  );
  const [dockDrives, setDockDrives] = useState<string[]>(() => parseDrives(localStorage.getItem("fern-dock-drives")));
  const [dockIconOnly, setDockIconOnly] = useState(
    () => localStorage.getItem("fern-dock-icon-only") === "true"
  );
  const [startIcon, setStartIcon] = useState(
    () => localStorage.getItem("fern-start-icon") || "default"
  );
  const [dockAdaptive, setDockAdaptive] = useState(
    () => localStorage.getItem("fern-dock-adaptive") === "true"
  );
  const [dockWinNumberEnabled, setDockWinNumberEnabled] = useState(
    () => localStorage.getItem("fern-dock-win-number-enabled") !== "false"
  );
  const [dockMixedReorder, setDockMixedReorder] = useState(
    () => localStorage.getItem("fern-dock-mixed-reorder") === "true"
  );
  const [dockIndicator, setDockIndicator] = useState(() => localStorage.getItem("fern-dock-indicator") ?? "bar");
  const [dockMenuItems, setDockMenuItems] = useState<string[]>(() => parseMenuItems(localStorage.getItem("fern-dock-menu-items")));
  const [dockShowEndTask, setDockShowEndTask] = useState(() => localStorage.getItem("fern-dock-show-end-task") !== "false");
  const [panelStatus, setPanelStatus] = useState(() => localStorage.getItem("fern-panel-status-enabled") !== "false");
  const readScrollDirection = (v: string | null) => (v === "inverted" ? "inverted" : "natural");
  const [panelScrollDirection, setPanelScrollDirection] = useState(() => readScrollDirection(localStorage.getItem("fern-notch-scroll-direction")));
  const [dockMode, setDockMode] = useState(() => {
    const raw = localStorage.getItem("fern-dock-mode") || "smart";
    return raw === "auto-hide" ? "smart" : raw;
  });
  const [notchMode, setNotchMode] = useState("fixed");
  const [dockMonitorMode, setDockMonitorMode] = useState<MonitorMode>(
    () => (localStorage.getItem("fern-dock-monitor-mode") as MonitorMode) || "primary"
  );
  const [dockMonitorId, setDockMonitorId] = useState(
    () => localStorage.getItem("fern-dock-monitor-id") || ""
  );
  const [notchMonitorMode, setNotchMonitorMode] = useState<MonitorMode>(
    () => (localStorage.getItem("fern-notch-monitor-mode") as MonitorMode) || "primary"
  );
  const [notchMonitorId, setNotchMonitorId] = useState(
    () => localStorage.getItem("fern-notch-monitor-id") || ""
  );
  const [dockModeByMonitor, setDockModeByMonitor] = useState<Record<string, string>>({});
  const [notchModeByMonitor, setNotchModeByMonitor] = useState<Record<string, string>>({});
  const [monitors, setMonitors] = useState<MonitorInfo[]>([]);
  const [lowBatteryThreshold, setLowBatteryThreshold] = useState(20);
  const [hasBattery, setHasBattery] = useState(() => localStorage.getItem("fern-has-battery") !== "false");
  const [updateStatus, setUpdateStatus] = useState<
    "idle" | "checking" | "available" | "uptodate" | "error" | "downloading" | "installing"
  >("idle");
  const [updateVersion, setUpdateVersion] = useState("");
  const [appVersion, setAppVersion] = useState("");
  const [autoUpdate, setAutoUpdate] = useState(
    () => localStorage.getItem("fern-auto-update") === "true"
  );
  const [prereleaseChannel, setPrereleaseChannel] = useState(
    () => localStorage.getItem("fern-update-channel") === "prerelease"
  );
  const [scale, setScale] = useState(
    () => parseFloat(localStorage.getItem("fern-scale") || "1.0")
  );
  const [themeStyle, setThemeStyle] = useState(() => localStorage.getItem("fern-theme-style") || "solid");
  const [themeMode, setThemeMode] = useState(
    () => localStorage.getItem("fern-theme-mode") || "dark"
  );
  const [themeColor, setThemeColor] = useState(
    () => localStorage.getItem("fern-theme-color") || "#007aff"
  );
  const [themeOpacity, setThemeOpacity] = useState(() => {
    const val = localStorage.getItem("fern-theme-opacity");
    return val !== null ? parseFloat(val) : 0.8;
  });
  const [themeSaturation, setThemeSaturation] = useState(() => {
    const val = localStorage.getItem("fern-theme-saturation");
    return val !== null ? parseFloat(val) : 0.5;
  });
  const [themeBrightness, setThemeBrightness] = useState(() => {
    const val = localStorage.getItem("fern-theme-brightness");
    return val !== null ? parseFloat(val) : 0.15;
  });
  const [exportStatus, setExportStatus] = useState<"idle" | "exporting" | "success" | "error">("idle");
  const [importStatus, setImportStatus] = useState<"idle" | "importing" | "success" | "error">("idle");
  const [resetStatus, setResetStatus] = useState<"idle" | "confirm" | "resetting">("idle");
  const [uninstallStatus, setUninstallStatus] = useState<"idle" | "confirm" | "uninstalling" | "error">("idle");
  const [uninstallError, setUninstallError] = useState("");

  // ── Load all settings from backend + localStorage ──
  const loadAllSettings = useCallback(async () => {
    try {
      const settings: Record<string, string> = await invoke("load_settings");
      const getVal = (key: string) => {
        const val = settings[key];
        if (val !== undefined && val !== null) return String(val);
        return localStorage.getItem(key);
      };

      const apply = <T>(val: string | null, setter: (v: T) => void, transform: (v: string) => T) => {
        if (val !== null) setter(transform(val));
      };

      apply(getVal("fern-weather-enabled"), setWeatherEnabled, readBool);
      apply(getVal("fern-calendar-enabled"), setCalendarEnabled, readBool);
      apply(getVal("fern-timer-enabled") ?? getVal("fern-calendar-enabled"), setTimerEnabled, readBool);
      apply(getVal("fern-timer-sound-enabled"), setTimerSoundEnabled, readBool);
      apply(getVal("fern-notch-timer-progress"), setNotchTimerProgress, readBool);
      apply(getVal("fern-stopwatch-enabled") ?? getVal("fern-calendar-enabled"), setStopwatchEnabled, readBool);
      apply(getVal("fern-dual-clock-enabled"), setDualClockEnabled, readBool);
      apply(getVal("fern-dual-clock-zone"), setDualClockZone, (v) => v);
      apply(getVal("fern-dual-clock-label"), setDualClockLabel, (v) => v);
      apply(getVal("fern-music-mode-enabled"), setMusicModeEnabled, readBool);
      apply(getVal("fern-music-compact-notch"), setMusicCompactNotch, readBool);
      apply(getVal("fern-notch-scroll-volume"), setNotchScrollVolume, (v) => v === "true");
      apply(getVal("fern-shelf-persist"), setShelfPersist, (v) => v === "true");
      apply(getVal("fern-shelf-keep-open"), setShelfKeepOpen, (v) => v === "true");
      apply(getVal("fern-notch-remember-mode"), setRememberMode, (v) => v === "true");
      apply(getVal("fern-volume-overlay-enabled"), setVolumeOverlayEnabled, readBool);
      apply(getVal("fern-brightness-overlay-enabled"), setBrightnessOverlayEnabled, readBool);
      apply(getVal("fern-media-ambience-enabled"), setMediaAmbienceEnabled, readBool);
      apply(getVal("fern-media-artwork-theme"), setArtworkThemeEnabled, readBool);
      apply(getVal("fern-media-compact-glow-enabled"), setMediaCompactGlowEnabled, readBool);
      apply(getVal("fern-lyrics-enabled"), setLyricsEnabled, readBool);
      apply(getVal("fern-lyrics-seek-enabled"), setLyricsSeekEnabled, readBool);
      apply(getVal("fern-corners-enabled"), setCornersEnabled, readBool);
      apply(getVal("fern-time-format-24h"), setTimeFormat24h, readBool);
      apply(getVal("fern-memory-saver"), setMemorySaver, readBool);
      apply(getVal("fern-show-update-indicator"), setShowUpdateIndicator, readBool);
      apply(getVal("fern-auto-update"), setAutoUpdate, readBool);
      apply(getVal("fern-update-channel"), setPrereleaseChannel, (v) => v === "prerelease");
      apply(getVal("fern-volume-edge-enabled"), setVolumeEdgeEnabled, readBool);
      apply(getVal("fern-brightness-edge-enabled"), setBrightnessEdgeEnabled, readBool);
      apply(getVal("fern-agent-api"), setAgentApiEnabled, (v) => v === "true");
      apply(getVal("fern-agent-sounds"), setAgentSoundsEnabled, (v) => v !== "false");
      apply(getVal("fern-notch-edge-delay"), setNotchEdgeDelay, (v) => v);
      apply(getVal("fern-volume-side"), setVolumeSide, (v) => (v === "right" ? "right" : "left"));
      apply(getVal("fern-brightness-side"), setBrightnessSide, (v) => (v === "left" ? "left" : "right"));
      apply(getVal("fern-edge-delay"), setEdgeDelay, (v) => v);
      apply(getVal("fern-dock-attention-enabled"), setDockAttentionEnabled, (v) => v !== "false");
      apply(getVal("fern-dock-hide-delay"), setDockHideDelay, (v) => v);
      apply(getVal("fern-glass-wallpaper"), setGlassWallpaperEnabled, (v) => v !== "false");
      apply(getVal("fern-dock-enabled"), setDockEnabled, readBool);
      apply(getVal("fern-dock-preview-enabled"), setDockPreviewEnabled, readBool);
      apply(getVal("fern-dock-console-windows"), setDockConsoleWindows, readBool);
      apply(getVal("fern-dock-app-volume-scroll"), setDockAppVolumeScroll, (v) => v !== "false");
      apply(getVal("fern-week-start"), setWeekStart, (v) => (v as "auto" | "sunday" | "monday") || "auto");
      apply(getVal("fern-dock-search-enabled"), setDockSearchEnabled, readBool);
      apply(getVal("fern-search-launcher"), setDockSearchLauncher, (v) => String(v));
      apply(getVal("fern-dock-calendar-enabled"), setDockCalendarEnabled, readBool);
      apply(getVal("fern-dock-tray-enabled"), setDockTray, (v) => v !== "false");
      apply(getVal("fern-dock-taskview-enabled"), setDockTaskView, readBool);
      apply(getVal("fern-dock-show-desktop-corner"), setDockShowDesktop, readBool);
      apply(getVal("fern-panel-control-center-enabled"), setPanelControlCenter, (v) => v !== "false");
      apply(getVal("fern-panel-shelf-enabled"), setPanelShelf, (v) => v !== "false");
      apply(getVal("fern-panel-order"), setPanelOrder, (v) => parsePanelOrder(v));
      apply(getVal("fern-cc-layout"), setCcLayout, (v) => parseCcLayout(v));
      apply(getVal("fern-dock-recycle-enabled"), setDockRecycle, readBool);
      apply(getVal("fern-dock-indicator"), setDockIndicator, (v) => v ?? "bar");
      apply(getVal("fern-dock-menu-items"), setDockMenuItems, parseMenuItems);
      apply(getVal("fern-dock-show-end-task"), setDockShowEndTask, (v) => v !== "false");
      apply(getVal("fern-panel-status-enabled"), setPanelStatus, (v) => v !== "false");
      apply(getVal("fern-notch-scroll-direction"), setPanelScrollDirection, readScrollDirection);
      apply(getVal("fern-dock-drives"), setDockDrives, parseDrives);
      apply(getVal("fern-dock-icon-only"), setDockIconOnly, readBool);
      apply(getVal("fern-start-icon"), setStartIcon, (v) => v);
      apply(getVal("fern-dock-adaptive"), setDockAdaptive, readBool);
      apply(getVal("fern-dock-win-number-enabled"), setDockWinNumberEnabled, readBool);
      apply(getVal("fern-dock-mixed-reorder"), setDockMixedReorder, readBool);

      apply(getVal("fern-temp-unit"), setTempUnitFahrenheit, (v) => v === "fahrenheit");
      apply(getVal("fern-scale"), setScale, parseFloat);
      apply(getVal("fern-low-battery-threshold"), setLowBatteryThreshold, parseInt);

      apply(getVal("fern-notch-mode"), setNotchMode, (v) => (v === "auto-hide" ? "smart" : v));
      apply(getVal("fern-dock-mode"), setDockMode, (v) => (v === "auto-hide" ? "smart" : v));

      apply(getVal("fern-dock-monitor-mode"), setDockMonitorMode, (v) => v as MonitorMode);
      apply(getVal("fern-dock-monitor-id"), setDockMonitorId, (v) => v);
      apply(getVal("fern-notch-monitor-mode"), setNotchMonitorMode, (v) => v as MonitorMode);
      apply(getVal("fern-notch-monitor-id"), setNotchMonitorId, (v) => v);

      const dockByMonVal = getVal("fern-dock-mode-by-monitor");
      if (dockByMonVal) {
        try { setDockModeByMonitor(JSON.parse(dockByMonVal)); } catch {}
      }
      const notchByMonVal = getVal("fern-notch-mode-by-monitor");
      if (notchByMonVal) {
        try { setNotchModeByMonitor(JSON.parse(notchByMonVal)); } catch {}
      }

      const savedCity = getVal("fern-weather-city");
      if (savedCity) {
        setCityName(savedCity);
        setCityQuery(savedCity);
      }

      apply(getVal("fern-theme-style"), setThemeStyle, (v) => v);
      apply(getVal("fern-theme-mode"), setThemeMode, (v) => v);
      apply(getVal("fern-theme-color"), setThemeColor, (v) => v);
      apply(getVal("fern-theme-opacity"), setThemeOpacity, parseFloat);
      apply(getVal("fern-theme-saturation"), setThemeSaturation, parseFloat);
      apply(getVal("fern-theme-brightness"), setThemeBrightness, parseFloat);

      const widgetsVal = getVal("fern-status-widgets");
      if (widgetsVal) {
        try {
          const parsed = JSON.parse(widgetsVal);
          if (parsed && Array.isArray(parsed.left) && Array.isArray(parsed.right)) {
            setStatusWidgets(parsed);
          }
        } catch {}
      }
    } catch (e) {
      console.error("Failed to load settings:", e);
    }
  }, []);

  // ── Initialize on mount ──
  useEffect(() => {
    loadAllSettings();

    invoke<boolean>("get_autostart")
      .then(setAutostart)
      .catch(() => {});

    getVersion()
      .then((ver) => setAppVersion(ver))
      .catch(() => setAppVersion(""));

    invoke<boolean>("has_battery").then((present) => {
      setHasBattery(present);
      localStorage.setItem("fern-has-battery", String(present));
    }).catch(() => {});

    checkForUpdates(false);
  }, []);

  useEffect(() => {
    const refreshMonitors = () => {
      invoke<MonitorInfo[]>("get_monitors").then(setMonitors).catch(() => {});
    };
    refreshMonitors();
    const unlisten = listen("monitors-changed", refreshMonitors);
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  // ── Sync settings from other windows ──
  useSettingsSync({
    "fern-dock-mode": setDockMode,
    "fern-notch-mode": setNotchMode,
    "fern-dock-monitor-mode": (v) => setDockMonitorMode(v as MonitorMode),
    "fern-dock-monitor-id": setDockMonitorId,
    "fern-notch-monitor-mode": (v) => setNotchMonitorMode(v as MonitorMode),
    "fern-notch-monitor-id": setNotchMonitorId,
    "fern-dock-mode-by-monitor": (v) => { try { setDockModeByMonitor(JSON.parse(v)); } catch {} },
    "fern-notch-mode-by-monitor": (v) => { try { setNotchModeByMonitor(JSON.parse(v)); } catch {} },
    "fern-dock-enabled": setDockEnabled,
    "fern-dock-tray-enabled": (v) => setDockTray(v !== false && v !== "false"),
    "fern-dock-taskview-enabled": setDockTaskView,
    "fern-dock-show-desktop-corner": setDockShowDesktop,
    "fern-panel-control-center-enabled": (v) => setPanelControlCenter(v !== false && v !== "false"),
    "fern-panel-shelf-enabled": (v) => setPanelShelf(v !== false && v !== "false"),
    "fern-panel-order": (v) => setPanelOrder(parsePanelOrder(v === null ? null : String(v))),
    "fern-cc-layout": (v) => setCcLayout(parseCcLayout(v === null ? null : String(v))),
    "fern-dock-recycle-enabled": setDockRecycle,
    "fern-dock-indicator": (v) => setDockIndicator(String(v)),
    "fern-dock-menu-items": (v) => setDockMenuItems(parseMenuItems(v)),
    "fern-dock-show-end-task": (v) => setDockShowEndTask(v !== false && v !== "false"),
    "fern-panel-status-enabled": (v) => setPanelStatus(v !== false && v !== "false"),
    "fern-notch-scroll-direction": (v) => setPanelScrollDirection(String(v)),
    "fern-dock-drives": (v) => setDockDrives(parseDrives(v)),
    "fern-dock-icon-only": setDockIconOnly,
    "fern-start-icon": setStartIcon,
    "fern-dock-adaptive": setDockAdaptive,
    "fern-dock-win-number-enabled": setDockWinNumberEnabled,
    "fern-dock-preview-enabled": setDockPreviewEnabled,
    "fern-dock-console-windows": (v: any) => setDockConsoleWindows(v === true || v === "true"),
    "fern-dock-app-volume-scroll": (v: any) => setDockAppVolumeScroll(v !== false && v !== "false"),
    "fern-dock-search-enabled": setDockSearchEnabled,
    "fern-search-launcher": setDockSearchLauncher,
    "fern-dock-calendar-enabled": setDockCalendarEnabled,
    "fern-dock-mixed-reorder": setDockMixedReorder,
    "fern-weather-enabled": setWeatherEnabled,
    "fern-calendar-enabled": setCalendarEnabled,
    "fern-timer-enabled": setTimerEnabled,
    "fern-timer-sound-enabled": setTimerSoundEnabled,
    "fern-notch-timer-progress": setNotchTimerProgress,
    "fern-stopwatch-enabled": setStopwatchEnabled,
    "fern-dual-clock-enabled": setDualClockEnabled,
    "fern-dual-clock-zone": (v) => setDualClockZone(String(v)),
    "fern-dual-clock-label": (v) => setDualClockLabel(String(v)),
    "fern-music-mode-enabled": setMusicModeEnabled,
    "fern-music-compact-notch": setMusicCompactNotch,
    "fern-notch-scroll-volume": setNotchScrollVolume,
    "fern-shelf-persist": setShelfPersist,
    "fern-shelf-keep-open": setShelfKeepOpen,
    "fern-notch-remember-mode": setRememberMode,
    "fern-media-ambience-enabled": setMediaAmbienceEnabled,
    "fern-media-artwork-theme": setArtworkThemeEnabled,
    "fern-media-compact-glow-enabled": setMediaCompactGlowEnabled,
    "fern-lyrics-enabled": setLyricsEnabled,
    "fern-lyrics-seek-enabled": setLyricsSeekEnabled,
    "fern-media-layout": setMediaLayout,
    "fern-corners-enabled": setCornersEnabled,
    "fern-show-update-indicator": setShowUpdateIndicator,
    "fern-time-format-24h": setTimeFormat24h,
    "fern-week-start": (v: string) => setWeekStart((v as "auto" | "sunday" | "monday") || "auto"),
    "fern-memory-saver": setMemorySaver,
    "fern-low-battery-threshold": setLowBatteryThreshold,
    "fern-scale": setScale,
    "fern-temp-unit": (v) => setTempUnitFahrenheit(v === "fahrenheit"),
    "fern-auto-update": setAutoUpdate,
    "fern-update-channel": (v) => setPrereleaseChannel(v === "prerelease"),
    "fern-volume-overlay-enabled": setVolumeOverlayEnabled,
    "fern-volume-edge-enabled": setVolumeEdgeEnabled,
    "fern-brightness-overlay-enabled": setBrightnessOverlayEnabled,
    "fern-brightness-edge-enabled": setBrightnessEdgeEnabled,
    "fern-agent-api": (v) => setAgentApiEnabled(v === "true" || v === true),
    "fern-agent-sounds": (v) => setAgentSoundsEnabled(v !== "false" && v !== false),
    "fern-notch-edge-delay": setNotchEdgeDelay,
    "fern-volume-side": (v) => setVolumeSide(v === "right" ? "right" : "left"),
    "fern-brightness-side": (v) => setBrightnessSide(v === "left" ? "left" : "right"),
    "fern-edge-delay": setEdgeDelay,
    "fern-dock-attention-enabled": (v) => setDockAttentionEnabled(v !== "false" && v !== false),
    "fern-dock-hide-delay": setDockHideDelay,
    "fern-glass-wallpaper": (v) => setGlassWallpaperEnabled(v !== "false" && v !== false),
    "fern-theme-style": (v) => setThemeStyle(String(v)),
    "fern-theme-mode": setThemeMode,
    "fern-theme-color": setThemeColor,
    "fern-theme-opacity": setThemeOpacity,
    "fern-theme-saturation": setThemeSaturation,
    "fern-theme-brightness": setThemeBrightness,
    "fern-weather-city": (v) => {
      setCityName(v || "");
      setCityQuery(v || "");
    },
  });

  // ── Listen for system accent changes (adaptive theme) ──
  useEffect(() => {
    const unlisten = listen<string>("system-accent-changed", (event) => {
      const mode = localStorage.getItem("fern-theme-mode") || "dark";
      if (mode === "adaptive") {
        try {
          const hsl = hexToHsl(event.payload);
          setThemeSaturation(hsl.s / 100);
          setThemeBrightness(hsl.l / 100);
        } catch (e) {
          console.error("Failed to parse system accent color change HSL:", e);
        }
      }
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  // ── City search debounce ──
  useEffect(() => {
    if (cityQuery.trim().length < 2 || cityQuery === cityName) {
      setCitySearchResults([]);
      setShowCityDropdown(false);
      return;
    }

    const timeout = setTimeout(async () => {
      try {
        const res = await fetch(
          `https://geocoding-api.open-meteo.com/v1/search?name=${encodeURIComponent(cityQuery)}&count=5&language=en&format=json`
        );
        const data = await res.json();
        if (data.results && data.results.length > 0) {
          setCitySearchResults(
            data.results.map((r: any) => ({
              name: r.name,
              country: r.country || "",
              latitude: r.latitude,
              longitude: r.longitude,
            }))
          );
          setShowCityDropdown(true);
        } else {
          setCitySearchResults([]);
        }
      } catch {
        setCitySearchResults([]);
      }
    }, 300);

    return () => clearTimeout(timeout);
  }, [cityQuery, cityName]);

  // ── Update checker ──
  const checkForUpdates = async (manual = true) => {
    setUpdateStatus("checking");
    try {
      const result = await invoke<UpdateCheckResult>("check_for_updates", { force: manual });
      if (result.available) {
        setUpdateStatus("available");
        setUpdateVersion(result.version ?? "");
      } else {
        setUpdateStatus("uptodate");
      }
    } catch (e) {
      console.error("Updater error:", e);
      setUpdateStatus("error");
    }
  };

  const installUpdate = async () => {
    try {
      setUpdateStatus("downloading");
      await invoke("install_update");
      setUpdateStatus("idle");
    } catch (e) {
      console.error(e);
      setUpdateStatus("error");
    }
  };

  // Reflect install progress triggered from any window (including auto-update).
  useEffect(() => {
    const unlisten = listen<{ status: string; progress?: number }>("auto-update-status", (event) => {
      switch (event.payload.status) {
        case "downloading":
          setUpdateStatus("downloading");
          break;
        case "installing":
          setUpdateStatus("installing");
          break;
        case "done":
          setUpdateStatus((prev) => (prev === "downloading" || prev === "installing" ? "idle" : prev));
          break;
      }
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  // ── Autostart ──
  const toggleAutostart = async () => {
    try {
      const currentlyEnabled = await invoke<boolean>("get_autostart");
      if (currentlyEnabled) {
        await invoke("set_autostart", { enabled: false });
        setAutostart(false);
      } else {
        await invoke("set_autostart", { enabled: true });
        setAutostart(true);
      }
    } catch (err) {}
  };

  // ── Simple boolean toggles ──
  const toggleWeather = () => {
    const next = !weatherEnabled;
    setWeatherEnabled(next);
    saveSetting("fern-weather-enabled", String(next));
  };

  const toggleCalendar = () => {
    const next = !calendarEnabled;
    setCalendarEnabled(next);
    saveSetting("fern-calendar-enabled", String(next));
  };

  const toggleTimerEnabled = () => {
    const next = !timerEnabled;
    setTimerEnabled(next);
    saveSetting("fern-timer-enabled", String(next));
  };

  const toggleTimerSound = () => {
    const next = !timerSoundEnabled;
    setTimerSoundEnabled(next);
    saveSetting("fern-timer-sound-enabled", String(next));
  };

  const toggleNotchTimerProgress = () => {
    const next = !notchTimerProgress;
    setNotchTimerProgress(next);
    saveSetting("fern-notch-timer-progress", String(next));
  };

  const toggleStopwatchEnabled = () => {
    const next = !stopwatchEnabled;
    setStopwatchEnabled(next);
    saveSetting("fern-stopwatch-enabled", String(next));
  };

  const toggleDualClock = () => {
    const next = !dualClockEnabled;
    setDualClockEnabled(next);
    saveSetting("fern-dual-clock-enabled", String(next));
  };

  const selectDualClockZone = ({ label, zone }: { label: string; zone: string }) => {
    setDualClockZone(zone);
    setDualClockLabel(label);
    saveSetting("fern-dual-clock-zone", zone);
    saveSetting("fern-dual-clock-label", label);
  };

  const toggleMusicMode = () => {
    const next = !musicModeEnabled;
    setMusicModeEnabled(next);
    saveSetting("fern-music-mode-enabled", String(next));
  };

  const toggleMusicCompactNotch = () => {
    const next = !musicCompactNotch;
    setMusicCompactNotch(next);
    saveSetting("fern-music-compact-notch", String(next));
  };

  const toggleNotchScrollVolume = () => {
    const next = !notchScrollVolume;
    setNotchScrollVolume(next);
    saveSetting("fern-notch-scroll-volume", String(next));
  };

  const toggleRememberMode = () => {
    const next = !rememberMode;
    setRememberMode(next);
    saveSetting("fern-notch-remember-mode", String(next));
  };

  const toggleShelfPersist = () => {
    const next = !shelfPersist;
    setShelfPersist(next);
    saveSetting("fern-shelf-persist", String(next));
  };

  const toggleShelfKeepOpen = () => {
    const next = !shelfKeepOpen;
    setShelfKeepOpen(next);
    saveSetting("fern-shelf-keep-open", String(next));
  };

  const toggleMediaLayout = (layout: "classic" | "compact") => {
    setMediaLayout(layout);
    saveSetting("fern-media-layout", layout);
  };

  const toggleVolumeOverlay = () => {
    const next = !volumeOverlayEnabled;
    setVolumeOverlayEnabled(next);
    saveSetting("fern-volume-overlay-enabled", String(next));
  };

  const toggleVolumeEdge = () => {
    const next = !volumeEdgeEnabled;
    setVolumeEdgeEnabled(next);
    saveSetting("fern-volume-edge-enabled", String(next));
  };

  const toggleBrightnessOverlay = () => {
    const next = !brightnessOverlayEnabled;
    setBrightnessOverlayEnabled(next);
    saveSetting("fern-brightness-overlay-enabled", String(next));
  };

  const toggleBrightnessEdge = () => {
    const next = !brightnessEdgeEnabled;
    setBrightnessEdgeEnabled(next);
    saveSetting("fern-brightness-edge-enabled", String(next));
  };

  const toggleAgentApi = () => {
    const next = !agentApiEnabled;
    setAgentApiEnabled(next);
    saveSetting("fern-agent-api", String(next));
  };

  const toggleAgentSounds = () => {
    const next = !agentSoundsEnabled;
    setAgentSoundsEnabled(next);
    saveSetting("fern-agent-sounds", String(next));
  };

  const handleNotchEdgeDelayChange = (val: string) => {
    setNotchEdgeDelay(val);
    saveSetting("fern-notch-edge-delay", val);
  };

  const handleVolumeSideChange = (val: string) => {
    const side = val === "right" ? "right" : "left";
    setVolumeSide(side);
    saveSetting("fern-volume-side", side);
  };

  const handleBrightnessSideChange = (val: string) => {
    const side = val === "left" ? "left" : "right";
    setBrightnessSide(side);
    saveSetting("fern-brightness-side", side);
  };

  const handleEdgeDelayChange = (val: string) => {
    setEdgeDelay(val);
    saveSetting("fern-edge-delay", val);
  };

  const toggleDockAttention = () => {
    const next = !dockAttentionEnabled;
    setDockAttentionEnabled(next);
    saveSetting("fern-dock-attention-enabled", String(next));
  };

  const handleDockHideDelayChange = (val: string) => {
    setDockHideDelay(val);
    saveSetting("fern-dock-hide-delay", val);
  };

  const toggleGlassWallpaper = () => {
    const next = !glassWallpaperEnabled;
    setGlassWallpaperEnabled(next);
    saveSetting("fern-glass-wallpaper", String(next));
  };

  const toggleAmbience = () => {
    const next = !mediaAmbienceEnabled;
    setMediaAmbienceEnabled(next);
    saveSetting("fern-media-ambience-enabled", String(next));
  };

  const toggleArtworkTheme = () => {
    const next = !artworkThemeEnabled;
    setArtworkThemeEnabled(next);
    saveSetting("fern-media-artwork-theme", String(next));
  };

  const toggleCompactGlow = () => {
    const next = !mediaCompactGlowEnabled;
    setMediaCompactGlowEnabled(next);
    saveSetting("fern-media-compact-glow-enabled", String(next));
  };

  const toggleLyrics = () => {
    const next = !lyricsEnabled;
    setLyricsEnabled(next);
    saveSetting("fern-lyrics-enabled", String(next));
  };

  const toggleLyricsSeek = () => {
    const next = !lyricsSeekEnabled;
    setLyricsSeekEnabled(next);
    saveSetting("fern-lyrics-seek-enabled", String(next));
  };

  const toggleCorners = () => {
    const next = !cornersEnabled;
    setCornersEnabled(next);
    saveSetting("fern-corners-enabled", String(next));
  };

  const toggleUpdateIndicator = () => {
    const next = !showUpdateIndicator;
    setShowUpdateIndicator(next);
    saveSetting("fern-show-update-indicator", String(next));
  };

  const changeWeekStart = (val: "auto" | "sunday" | "monday") => {
    setWeekStart(val);
    saveSetting("fern-week-start", val);
  };

  const toggleTimeFormat24h = () => {
    const next = !timeFormat24h;
    setTimeFormat24h(next);
    saveSetting("fern-time-format-24h", String(next));
  };

  const toggleMemorySaver = () => {
    const next = !memorySaver;
    setMemorySaver(next);
    saveSetting("fern-memory-saver", String(next));
  };

  const toggleTempUnit = () => {
    const next = !tempUnitFahrenheit;
    setTempUnitFahrenheit(next);
    saveSetting("fern-temp-unit", next ? "fahrenheit" : "celsius");
  };

  const toggleDock = () => {
    const next = !dockEnabled;
    setDockEnabled(next);
    saveSetting("fern-dock-enabled", String(next));
  };

  const toggleDockConsoleWindows = () => {
    const next = !dockConsoleWindows;
    setDockConsoleWindows(next);
    saveSetting("fern-dock-console-windows", String(next));
  };

  const toggleDockAppVolumeScroll = () => {
    const next = !dockAppVolumeScroll;
    setDockAppVolumeScroll(next);
    saveSetting("fern-dock-app-volume-scroll", String(next));
  };

  const toggleDockPreview = () => {
    const next = !dockPreviewEnabled;
    setDockPreviewEnabled(next);
    saveSetting("fern-dock-preview-enabled", String(next));
  };

  const toggleDockSearch = () => {
    const next = !dockSearchEnabled;
    setDockSearchEnabled(next);
    saveSetting("fern-dock-search-enabled", String(next));
  };

  const setDockSearchLauncherValue = (val: string) => {
    setDockSearchLauncher(val);
    saveSetting("fern-search-launcher", val);
  };

  const toggleDockTray = () => {
    const next = !dockTray;
    setDockTray(next);
    saveSetting("fern-dock-tray-enabled", String(next));
  };

  const toggleDockShowDesktop = () => {
    const next = !dockShowDesktop;
    setDockShowDesktop(next);
    saveSetting("fern-dock-show-desktop-corner", String(next));
  };

  const togglePanelControlCenter = () => {
    const next = !panelControlCenter;
    setPanelControlCenter(next);
    saveSetting("fern-panel-control-center-enabled", String(next));
  };

  const reorderPanels = (order: PanelId[]) => {
    setPanelOrder(order);
    saveSetting("fern-panel-order", JSON.stringify(order));
  };

  const changeCcLayout = (next: CcLayout) => {
    setCcLayout(next);
    saveSetting("fern-cc-layout", JSON.stringify(next));
  };

  const resetCcLayout = () => {
    setCcLayout({});
    localStorage.removeItem("fern-cc-layout");
    invoke("save_setting", { key: "fern-cc-layout", value: null }).catch(() => {});
  };

  const resetPanelOrder = () => {
    setPanelOrder(null);
    localStorage.removeItem("fern-panel-order");
    invoke("save_setting", { key: "fern-panel-order", value: null }).catch(() => {});
  };

  const togglePanelShelf = () => {
    const next = !panelShelf;
    setPanelShelf(next);
    saveSetting("fern-panel-shelf-enabled", String(next));
  };

  const toggleDockTaskView = () => {
    const next = !dockTaskView;
    setDockTaskView(next);
    saveSetting("fern-dock-taskview-enabled", String(next));
  };

  const toggleDockRecycle = () => {
    const next = !dockRecycle;
    setDockRecycle(next);
    saveSetting("fern-dock-recycle-enabled", String(next));
  };

  const toggleDockDrive = (letter: string) => {
    const next = dockDrives.includes(letter) ? dockDrives.filter((d) => d !== letter) : [...dockDrives, letter].sort();
    setDockDrives(next);
    saveSetting("fern-dock-drives", JSON.stringify(next));
  };

  const toggleDockCalendar = () => {
    const next = !dockCalendarEnabled;
    setDockCalendarEnabled(next);
    saveSetting("fern-dock-calendar-enabled", String(next));
  };

  
  const changeDockIndicator = (value: string) => {
    setDockIndicator(value);
    saveSetting("fern-dock-indicator", value);
  };

  const toggleDockMenuItem = (id: string) => {
    const next = dockMenuItems.includes(id)
      ? dockMenuItems.filter((x) => x !== id)
      : DOCK_MENU_IDS.filter((x) => x === id || dockMenuItems.includes(x));
    setDockMenuItems(next);
    saveSetting("fern-dock-menu-items", JSON.stringify(next));
  };

  const toggleDockShowEndTask = () => {
    const next = !dockShowEndTask;
    setDockShowEndTask(next);
    saveSetting("fern-dock-show-end-task", String(next));
  };

  const togglePanelStatus = () => {
    const next = !panelStatus;
    setPanelStatus(next);
    saveSetting("fern-panel-status-enabled", String(next));
  };

  const changePanelScrollDirection = (value: string) => {
    const norm = value === "inverted" ? "inverted" : "natural";
    setPanelScrollDirection(norm);
    saveSetting("fern-notch-scroll-direction", norm);
  };

  const toggleDockMixedReorder = () => {
    const next = !dockMixedReorder;
    setDockMixedReorder(next);
    saveSetting("fern-dock-mixed-reorder", String(next));
  };

  const toggleDockIconOnly = () => {
    const next = !dockIconOnly;
    setDockIconOnly(next);
    saveSetting("fern-dock-icon-only", String(next));
  };

  const handleStartIconChange = (icon: string) => {
    setStartIcon(icon);
    saveSetting("fern-start-icon", icon);
  };

  const toggleDockAdaptive = () => {
    const next = !dockAdaptive;
    setDockAdaptive(next);
    saveSetting("fern-dock-adaptive", String(next));
  };

  const toggleDockWinNumber = () => {
    const next = !dockWinNumberEnabled;
    setDockWinNumberEnabled(next);
    saveSetting("fern-dock-win-number-enabled", String(next));
  };

  const toggleAutoUpdate = () => {
    const next = !autoUpdate;
    setAutoUpdate(next);
    saveSetting("fern-auto-update", String(next));
  };

  const togglePrereleaseChannel = async () => {
    const next = !prereleaseChannel;
    const value = next ? "prerelease" : "stable";
    setPrereleaseChannel(next);
    localStorage.setItem("fern-update-channel", value);
    await invoke("save_setting", { key: "fern-update-channel", value }).catch(console.error);
    checkForUpdates();
  };

  // ── Value setters ──
  const setDockModeValue = (newMode: string) => {
    setDockMode(newMode);
    saveSetting("fern-dock-mode", newMode);
    if (dockMonitorMode !== "all" || monitors.length <= 1) {
      setDockModeByMonitor({});
      saveSetting("fern-dock-mode-by-monitor", "{}");
    }
  };

  const setNotchModeValue = (newMode: string) => {
    setNotchMode(newMode);
    saveSetting("fern-notch-mode", newMode);
    if (notchMonitorMode !== "all" || monitors.length <= 1) {
      setNotchModeByMonitor({});
      saveSetting("fern-notch-mode-by-monitor", "{}");
    }
  };

  const setDockMonitorModeValue = (mode: MonitorMode) => {
    setDockMonitorMode(mode);
    saveSetting("fern-dock-monitor-mode", mode);
  };

  const setDockMonitorIdValue = (id: string) => {
    setDockMonitorId(id);
    saveSetting("fern-dock-monitor-id", id);
  };

  const setNotchMonitorModeValue = (mode: MonitorMode) => {
    setNotchMonitorMode(mode);
    saveSetting("fern-notch-monitor-mode", mode);
  };

  const setNotchMonitorIdValue = (id: string) => {
    setNotchMonitorId(id);
    saveSetting("fern-notch-monitor-id", id);
  };

  const setDockModeForMonitor = (monitorId: string, mode: string | null) => {
    setDockModeByMonitor((prev) => {
      const next = { ...prev };
      if (mode === null) delete next[monitorId]; else next[monitorId] = mode;
      saveSetting("fern-dock-mode-by-monitor", JSON.stringify(next));
      return next;
    });
  };

  const replaceDockModesByMonitor = (next: Record<string, string>) => {
    setDockModeByMonitor(next);
    saveSetting("fern-dock-mode-by-monitor", JSON.stringify(next));
  };

  const replaceNotchModesByMonitor = (next: Record<string, string>) => {
    setNotchModeByMonitor(next);
    saveSetting("fern-notch-mode-by-monitor", JSON.stringify(next));
  };

  const setNotchModeForMonitor = (monitorId: string, mode: string | null) => {
    setNotchModeByMonitor((prev) => {
      const next = { ...prev };
      if (mode === null) delete next[monitorId]; else next[monitorId] = mode;
      saveSetting("fern-notch-mode-by-monitor", JSON.stringify(next));
      return next;
    });
  };

  const handleThresholdChange = (val: number) => {
    setLowBatteryThreshold(val);
    saveSetting("fern-low-battery-threshold", val.toString());
  };

  const handleScaleChange = (val: number) => {
    setScale(val);
    saveSetting("fern-scale", val.toString());
  };

  const handleWidgetsChange = (config: WidgetConfig) => {
    setStatusWidgets(config);
    saveSetting("fern-status-widgets", JSON.stringify(config));
  };

  // ── Theme handlers ──
  const handleThemeModeChange = async (mode: string) => {
    setThemeMode(mode);
    saveSetting("fern-theme-mode", mode);

    if (mode === "adaptive") {
      try {
        const accentHex = await invoke<string>("get_system_accent_color");
        const hsl = hexToHsl(accentHex);
        setThemeSaturation(hsl.s / 100);
        saveSetting("fern-theme-saturation", String(hsl.s / 100));
        setThemeBrightness(hsl.l / 100);
        saveSetting("fern-theme-brightness", String(hsl.l / 100));
      } catch (e) {
        console.error("Failed to parse adaptive accent HSL:", e);
      }
    }
  };

  const handleThemeColorChange = (color: string) => {
    setThemeColor(color);
    saveSetting("fern-theme-color", color);

    try {
      const hsl = hexToHsl(color);
      setThemeSaturation(hsl.s / 100);
      saveSetting("fern-theme-saturation", String(hsl.s / 100));
      setThemeBrightness(hsl.l / 100);
      saveSetting("fern-theme-brightness", String(hsl.l / 100));
    } catch (e) {
      console.error("Failed to parse custom color HSL:", e);
    }
  };

  const handleOpacityChange = (value: number) => {
    setThemeOpacity(value);
    saveSetting("fern-theme-opacity", String(value));
  };

  const handleSaturationChange = (value: number) => {
    setThemeSaturation(value);
    saveSetting("fern-theme-saturation", String(value));
  };

  const handleBrightnessChange = (value: number) => {
    setThemeBrightness(value);
    saveSetting("fern-theme-brightness", String(value));
  };

  const handleThemeStyleChange = (style: string) => {
    setThemeStyle(style);
    saveSetting("fern-theme-style", style);
    // Each style is tuned for an opacity; the slider can still change it afterwards.
    const opacity = STYLE_OPACITY[style];
    if (opacity !== undefined) {
      setThemeOpacity(opacity);
      saveSetting("fern-theme-opacity", String(opacity));
    }
  };


  // ── City search ──
  const selectCity = async (city: {
    name: string;
    country: string;
    latitude: number;
    longitude: number;
  }) => {
    setCityName(city.name);
    setCityQuery(city.name);
    setShowCityDropdown(false);
    setCitySearchResults([]);
    localStorage.setItem("fern-weather-city", city.name);
    localStorage.setItem("fern-weather-lat", city.latitude.toString());
    localStorage.setItem("fern-weather-lon", city.longitude.toString());
    await invoke("save_setting", { key: "fern-weather-lat", value: city.latitude.toString() }).catch(() => {});
    await invoke("save_setting", { key: "fern-weather-lon", value: city.longitude.toString() }).catch(() => {});
    await invoke("save_setting", { key: "fern-weather-city", value: city.name }).catch(() => {});
    emit("weather-refresh", { lat: city.latitude, lon: city.longitude, city: city.name });
  };

  const handleCityClear = async () => {
    setCityName("");
    setCityQuery("");
    setShowCityDropdown(false);
    setCitySearchResults([]);
    localStorage.removeItem("fern-weather-city");
    localStorage.removeItem("fern-weather-lat");
    localStorage.removeItem("fern-weather-lon");
    await invoke("save_setting", { key: "fern-weather-lat", value: null }).catch(() => {});
    await invoke("save_setting", { key: "fern-weather-lon", value: null }).catch(() => {});
    await invoke("save_setting", { key: "fern-weather-city", value: null }).catch(() => {});
    emit("weather-refresh", true);
  };

  // ── Export / Import ──
  const handleExportSettings = async () => {
    setExportStatus("exporting");
    try {
      const saved = await invoke<boolean>("export_settings_dialog");
      if (saved) {
        setExportStatus("success");
        setTimeout(() => setExportStatus("idle"), 2000);
      } else {
        setExportStatus("idle");
      }
    } catch (e) {
      console.error("Export failed:", e);
      setExportStatus("error");
      setTimeout(() => setExportStatus("idle"), 3000);
    }
  };

  const handleImportSettings = async () => {
    setImportStatus("importing");
    try {
      const imported = await invoke<boolean>("import_settings_dialog");
      if (imported) {
        await loadAllSettings();
        setImportStatus("success");
        setTimeout(() => setImportStatus("idle"), 2000);
      } else {
        setImportStatus("idle");
      }
    } catch (e) {
      console.error("Import failed:", e);
      setImportStatus("error");
      setTimeout(() => setImportStatus("idle"), 3000);
    }
  };

  const handleResetSettings = async () => {
    if (resetStatus === "idle") {
      setResetStatus("confirm");
      setTimeout(() => setResetStatus((s) => (s === "confirm" ? "idle" : s)), 4000);
      return;
    }
    if (resetStatus !== "confirm") return;

    setResetStatus("resetting");
    try {
      await invoke("reset_settings");
      localStorage.clear();
      await invoke("restart_fern");
    } catch (e) {
      console.error("Reset failed:", e);
      setResetStatus("idle");
    }
  };

  const handleUninstallFern = async () => {
    if (uninstallStatus === "idle" || uninstallStatus === "error") {
      setUninstallStatus("confirm");
      setTimeout(() => setUninstallStatus((s) => (s === "confirm" ? "idle" : s)), 4000);
      return;
    }
    if (uninstallStatus !== "confirm") return;

    setUninstallStatus("uninstalling");
    try {
      await invoke("uninstall_fern");
    } catch (e) {
      console.error("Uninstall failed:", e);
      setUninstallError(typeof e === "string" ? e : String((e as Error)?.message ?? e));
      setUninstallStatus("error");
      setTimeout(() => setUninstallStatus((s) => (s === "error" ? "idle" : s)), 12000);
    }
  };

  return {
    // System
    autostart,
    toggleAutostart,
    autoUpdate,
    toggleAutoUpdate,
    prereleaseChannel,
    togglePrereleaseChannel,
    lowBatteryThreshold,
    handleThresholdChange,
    timeFormat24h,
    toggleTimeFormat24h,
    weekStart,
    changeWeekStart,
    memorySaver,
    toggleMemorySaver,
    showUpdateIndicator,
    toggleUpdateIndicator,
    scale,
    handleScaleChange,
    cornersEnabled,
    toggleCorners,

    // Theme
    themeStyle,
    handleThemeStyleChange,
    themeMode,
    handleThemeModeChange,
    themeColor,
    handleThemeColorChange,
    themeOpacity,
    handleOpacityChange,
    themeSaturation,
    handleSaturationChange,
    themeBrightness,
    handleBrightnessChange,
    glassWallpaperEnabled,
    toggleGlassWallpaper,

    // Notch
    notchMode,
    setNotchModeValue,
    calendarEnabled,
    toggleCalendar,
    timerEnabled,
    toggleTimerEnabled,
    timerSoundEnabled,
    toggleTimerSound,
    notchTimerProgress,
    toggleNotchTimerProgress,
    stopwatchEnabled,
    toggleStopwatchEnabled,
    dualClockEnabled,
    toggleDualClock,
    dualClockZone,
    dualClockLabel,
    selectDualClockZone,
    musicModeEnabled,
    toggleMusicMode,
    musicCompactNotch,
    toggleMusicCompactNotch,
    notchScrollVolume,
    toggleNotchScrollVolume,
    shelfPersist,
    toggleShelfPersist,
    shelfKeepOpen,
    toggleShelfKeepOpen,
    rememberMode,
    toggleRememberMode,
    mediaLayout,
    toggleMediaLayout,
    agentApiEnabled,
    toggleAgentApi,
    agentSoundsEnabled,
    toggleAgentSounds,
    notchEdgeDelay,
    handleNotchEdgeDelayChange,
    mediaAmbienceEnabled,
    artworkThemeEnabled,
    toggleArtworkTheme,
    toggleAmbience,
    mediaCompactGlowEnabled,
    toggleCompactGlow,
    lyricsEnabled,
    toggleLyrics,
    lyricsSeekEnabled,
    toggleLyricsSeek,

    // Weather
    weatherEnabled,
    toggleWeather,
    tempUnitFahrenheit,
    toggleTempUnit,
    cityName,
    cityQuery,
    setCityQuery,
    citySearchResults,
    showCityDropdown,
    setShowCityDropdown,
    selectCity,
    handleCityClear,

    // Widgets
    statusWidgets,
    handleWidgetsChange,
    hasBattery,

    // Dock
    dockEnabled,
    toggleDock,
    dockMode,
    setDockModeValue,
    dockPreviewEnabled,
    toggleDockPreview,
    dockConsoleWindows,
    toggleDockConsoleWindows,
    dockAppVolumeScroll,
    toggleDockAppVolumeScroll,
    dockSearchEnabled,
    toggleDockSearch,
    dockSearchLauncher,
    setDockSearchLauncherValue,
    dockCalendarEnabled,
    toggleDockCalendar,
    dockTray,
    toggleDockTray,
    dockTaskView,
    toggleDockTaskView,
    dockShowDesktop,
    toggleDockShowDesktop,
    panelControlCenter,
    togglePanelControlCenter,
    panelShelf,
    togglePanelShelf,
    panelOrder,
    reorderPanels,
    resetPanelOrder,
    ccLayout,
    changeCcLayout,
    resetCcLayout,
    dockRecycle,
    toggleDockRecycle,
    dockDrives,
    toggleDockDrive,
    dockIconOnly,
    toggleDockIconOnly,
    startIcon,
    handleStartIconChange,
    dockAdaptive,
    toggleDockAdaptive,
    dockWinNumberEnabled,
    toggleDockWinNumber,
    dockMixedReorder,
    toggleDockMixedReorder,
    dockIndicator,
    changeDockIndicator,
    dockMenuItems,
    toggleDockMenuItem,
    dockShowEndTask,
    toggleDockShowEndTask,
    panelStatus,
    togglePanelStatus,
    panelScrollDirection,
    changePanelScrollDirection,
    dockAttentionEnabled,
    toggleDockAttention,
    dockHideDelay,
    handleDockHideDelayChange,

    monitors,
    dockMonitorMode,
    setDockMonitorModeValue,
    dockMonitorId,
    setDockMonitorIdValue,
    notchMonitorMode,
    setNotchMonitorModeValue,
    notchMonitorId,
    setNotchMonitorIdValue,
    dockModeByMonitor,
    setDockModeForMonitor,
    replaceDockModesByMonitor,
    notchModeByMonitor,
    setNotchModeForMonitor,
    replaceNotchModesByMonitor,

    // Overlays
    volumeOverlayEnabled,
    toggleVolumeOverlay,
    volumeEdgeEnabled,
    toggleVolumeEdge,
    volumeSide,
    handleVolumeSideChange,
    brightnessOverlayEnabled,
    toggleBrightnessOverlay,
    brightnessEdgeEnabled,
    toggleBrightnessEdge,
    brightnessSide,
    handleBrightnessSideChange,
    edgeDelay,
    handleEdgeDelayChange,

    // Updates
    updateStatus,
    updateVersion,
    appVersion,
    checkForUpdates,
    installUpdate,

    exportStatus,
    importStatus,
    handleExportSettings,
    handleImportSettings,
    resetStatus,
    handleResetSettings,
    uninstallStatus,
    uninstallError,
    handleUninstallFern,

    // Utilities
    restartFern: () => invoke("restart_fern"),
    quitFern: () => invoke("quit_fern"),
  };
}
