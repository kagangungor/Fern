import { useState, useEffect, useCallback } from "react";
import { parseCcLayout, parsePanelOrder, type CcLayout, type PanelId } from "../panelOrder";
import { invoke } from "@tauri-apps/api/core";
import { listen, emit } from "@tauri-apps/api/event";
import { enable, disable, isEnabled } from "@tauri-apps/plugin-autostart";
import { getVersion } from "@tauri-apps/api/app";
import type { UpdateCheckResult } from "../updater";
import { useSettingsSync } from "../hooks/useSettingsSync";
import { hexToHsl } from "../theme";
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
  const [mediaAmbienceEnabled, setMediaAmbienceEnabled] = useState(true);
  const [mediaCompactGlowEnabled, setMediaCompactGlowEnabled] = useState(true);
  const [mediaLayout, setMediaLayout] = useState<"classic" | "compact">(
    () => (localStorage.getItem("fern-media-layout") as "classic" | "compact") || "classic"
  );
  const [cornersEnabled, setCornersEnabled] = useState(
    () => localStorage.getItem("fern-corners-enabled") === "true"
  );
  const [showUpdateIndicator, setShowUpdateIndicator] = useState(
    () => localStorage.getItem("fern-show-update-indicator") !== "false"
  );
  const [timeFormat24h, setTimeFormat24h] = useState(
    () => localStorage.getItem("fern-time-format-24h") === "true"
  );
  const [legacyMode, setLegacyMode] = useState(
    () => localStorage.getItem("fern-legacy-mode") === "true"
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
  const [dockSearchEnabled, setDockSearchEnabled] = useState(true);
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
      apply(getVal("fern-notch-remember-mode"), setRememberMode, (v) => v === "true");
      apply(getVal("fern-volume-overlay-enabled"), setVolumeOverlayEnabled, readBool);
      apply(getVal("fern-brightness-overlay-enabled"), setBrightnessOverlayEnabled, readBool);
      apply(getVal("fern-media-ambience-enabled"), setMediaAmbienceEnabled, readBool);
      apply(getVal("fern-media-compact-glow-enabled"), setMediaCompactGlowEnabled, readBool);
      apply(getVal("fern-corners-enabled"), setCornersEnabled, readBool);
      apply(getVal("fern-time-format-24h"), setTimeFormat24h, readBool);
      apply(getVal("fern-legacy-mode"), setLegacyMode, readBool);
      apply(getVal("fern-show-update-indicator"), setShowUpdateIndicator, readBool);
      apply(getVal("fern-auto-update"), setAutoUpdate, readBool);
      apply(getVal("fern-update-channel"), setPrereleaseChannel, (v) => v === "prerelease");
      apply(getVal("fern-volume-edge-enabled"), setVolumeEdgeEnabled, readBool);
      apply(getVal("fern-brightness-edge-enabled"), setBrightnessEdgeEnabled, readBool);
      apply(getVal("fern-dock-enabled"), setDockEnabled, readBool);
      apply(getVal("fern-dock-preview-enabled"), setDockPreviewEnabled, readBool);
      apply(getVal("fern-dock-search-enabled"), setDockSearchEnabled, readBool);
      apply(getVal("fern-dock-calendar-enabled"), setDockCalendarEnabled, readBool);
      apply(getVal("fern-dock-tray-enabled"), setDockTray, (v) => v !== "false");
      apply(getVal("fern-dock-taskview-enabled"), setDockTaskView, readBool);
      apply(getVal("fern-dock-show-desktop-corner"), setDockShowDesktop, readBool);
      apply(getVal("fern-panel-control-center-enabled"), setPanelControlCenter, (v) => v !== "false");
      apply(getVal("fern-panel-shelf-enabled"), setPanelShelf, (v) => v !== "false");
      apply(getVal("fern-panel-order"), setPanelOrder, (v) => parsePanelOrder(v));
      apply(getVal("fern-cc-layout"), setCcLayout, (v) => parseCcLayout(v));
      apply(getVal("fern-dock-recycle-enabled"), setDockRecycle, readBool);
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

    isEnabled()
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
    "fern-dock-drives": (v) => setDockDrives(parseDrives(v)),
    "fern-dock-icon-only": setDockIconOnly,
    "fern-start-icon": setStartIcon,
    "fern-dock-adaptive": setDockAdaptive,
    "fern-dock-win-number-enabled": setDockWinNumberEnabled,
    "fern-dock-preview-enabled": setDockPreviewEnabled,
    "fern-dock-search-enabled": setDockSearchEnabled,
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
    "fern-notch-remember-mode": setRememberMode,
    "fern-media-ambience-enabled": setMediaAmbienceEnabled,
    "fern-media-compact-glow-enabled": setMediaCompactGlowEnabled,
    "fern-media-layout": setMediaLayout,
    "fern-corners-enabled": setCornersEnabled,
    "fern-show-update-indicator": setShowUpdateIndicator,
    "fern-time-format-24h": setTimeFormat24h,
    "fern-legacy-mode": setLegacyMode,
    "fern-low-battery-threshold": setLowBatteryThreshold,
    "fern-scale": setScale,
    "fern-temp-unit": (v) => setTempUnitFahrenheit(v === "fahrenheit"),
    "fern-auto-update": setAutoUpdate,
    "fern-update-channel": (v) => setPrereleaseChannel(v === "prerelease"),
    "fern-volume-overlay-enabled": setVolumeOverlayEnabled,
    "fern-volume-edge-enabled": setVolumeEdgeEnabled,
    "fern-brightness-overlay-enabled": setBrightnessOverlayEnabled,
    "fern-brightness-edge-enabled": setBrightnessEdgeEnabled,
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
      const currentlyEnabled = await isEnabled();
      if (currentlyEnabled) {
        await disable();
        setAutostart(false);
      } else {
        await enable();
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

  const toggleAmbience = () => {
    const next = !mediaAmbienceEnabled;
    setMediaAmbienceEnabled(next);
    saveSetting("fern-media-ambience-enabled", String(next));
  };

  const toggleCompactGlow = () => {
    const next = !mediaCompactGlowEnabled;
    setMediaCompactGlowEnabled(next);
    saveSetting("fern-media-compact-glow-enabled", String(next));
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

  const toggleTimeFormat24h = () => {
    const next = !timeFormat24h;
    setTimeFormat24h(next);
    saveSetting("fern-time-format-24h", String(next));
  };

  const toggleLegacyMode = () => {
    const next = !legacyMode;
    setLegacyMode(next);
    saveSetting("fern-legacy-mode", String(next));
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
  };

  const setNotchModeValue = (newMode: string) => {
    setNotchMode(newMode);
    saveSetting("fern-notch-mode", newMode);
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
    legacyMode,
    toggleLegacyMode,
    showUpdateIndicator,
    toggleUpdateIndicator,
    scale,
    handleScaleChange,
    cornersEnabled,
    toggleCorners,

    // Theme
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
    rememberMode,
    toggleRememberMode,
    mediaLayout,
    toggleMediaLayout,
    mediaAmbienceEnabled,
    toggleAmbience,
    mediaCompactGlowEnabled,
    toggleCompactGlow,

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
    dockSearchEnabled,
    toggleDockSearch,
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
    brightnessOverlayEnabled,
    toggleBrightnessOverlay,
    brightnessEdgeEnabled,
    toggleBrightnessEdge,

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
