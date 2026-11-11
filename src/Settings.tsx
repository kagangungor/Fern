import { StrictMode, useState, useEffect } from "react";
import { createRoot } from "react-dom/client";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { Effect, EffectState } from "@tauri-apps/api/window";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Settings, Palette, PanelTop, Monitor, Layers, Activity, Info } from "lucide-react";
import { WindowControls } from "./components/WindowControls";
import {
  useSettings,
  GeneralTab,
  AppearanceTab,
  NotchTab,
  CalendarSyncPage,
  DockTab,
  OverlaysTab,
  DiagnosticsTab,
  AboutTab,
} from "./settings/index";
import { SettingsSearch } from "./settings/SettingsSearch";
import type { SettingsTab } from "./settings/index";
import { initTheme } from "./theme";
import { useT } from "./i18n";
import "./Settings.css";

const appWindow = getCurrentWebviewWindow();

const TABS: { id: SettingsTab; labelKey: string; icon: typeof Settings }[] = [
  { id: "general", labelKey: "tabs.general", icon: Settings },
  { id: "appearance", labelKey: "tabs.appearance", icon: Palette },
  { id: "notch", labelKey: "tabs.notch", icon: PanelTop },
  { id: "dock", labelKey: "tabs.dock", icon: Monitor },
  { id: "overlays", labelKey: "tabs.overlays", icon: Layers },
  { id: "diagnostics", labelKey: "tabs.diagnostics", icon: Activity },
  { id: "about", labelKey: "tabs.about", icon: Info },
];

function smoothScrollTo(container: HTMLElement, target: HTMLElement, duration = 320) {
  const containerRect = container.getBoundingClientRect();
  const targetRect = target.getBoundingClientRect();
  const relativeTop = targetRect.top - containerRect.top;
  const startTop = container.scrollTop;
  const targetTop = Math.max(0, startTop + relativeTop - 12);

  if (Math.abs(targetTop - startTop) < 2) {
    container.scrollTop = targetTop;
    return;
  }

  const startTime = performance.now();
  const step = (now: number) => {
    const elapsed = now - startTime;
    const progress = Math.min(elapsed / duration, 1);
    const ease = 1 - Math.pow(1 - progress, 3);
    container.scrollTop = startTop + (targetTop - startTop) * ease;
    if (progress < 1) {
      requestAnimationFrame(step);
    }
  };
  requestAnimationFrame(step);
}

function SettingsApp() {
  const t = useT();
  const [activeTab, setActiveTab] = useState<SettingsTab>("general");
  const settings = useSettings();

  const handleNavigate = (tab: SettingsTab, sectionId?: string) => {
    setActiveTab(tab);
    if (sectionId) {
      window.dispatchEvent(new CustomEvent("fern-expand-section", { detail: sectionId }));
      setTimeout(() => {
        const el = document.getElementById(sectionId);
        const container = document.querySelector<HTMLElement>(".settings-content");
        if (el && container) {
          smoothScrollTo(container, el);
          el.classList.remove("section-highlight");
          void el.offsetWidth;
          el.classList.add("section-highlight");
          setTimeout(() => el.classList.remove("section-highlight"), 2000);
        }
      }, 160);
    }
  };

  useEffect(() => {
    return initTheme({ isSettings: true });
  }, []);

  useEffect(() => {
    const navigate = (page: string | null) => {
      if (page === "calendar-sync") setActiveTab("calendar-sync");
    };
    const unlisten = listen<string>("settings-navigate", (e) => navigate(e.payload));
    // This window is created on demand, so the page it was opened for was requested before the listener existed.
    unlisten.then(() => invoke<string | null>("take_settings_page")).then(navigate).catch(() => {});
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  useEffect(() => {
    const preventContext = (e: MouseEvent) => e.preventDefault();
    document.addEventListener("contextmenu", preventContext as any);

    appWindow.setEffects({ effects: [Effect.Acrylic], state: EffectState.Active, color: [10, 10, 15, 20] }).catch(() => {})
      .finally(() => {
        requestAnimationFrame(() => requestAnimationFrame(() => invoke("on_demand_window_ready").catch(() => {})));
      });

    return () => {
      document.removeEventListener("contextmenu", preventContext as any);
    };
  }, []);

  useEffect(() => {
    invoke("resize_settings_window", {
      width: 620 * settings.scale,
      height: 480 * settings.scale,
    }).catch(console.error);
  }, [settings.scale]);

  return (
    <div className="settings-container" style={{ zoom: settings.scale }}>
      <div className="title-bar" data-tauri-drag-region>
        <span className="title-text" data-tauri-drag-region>
          {t("settings.title")}
        </span>
        <WindowControls onClose={() => invoke("close_settings_window").catch(() => {})} />
      </div>

      <div className="settings-body">
        <div className="settings-sidebar">
          <SettingsSearch onNavigate={handleNavigate} />
          {TABS.map(({ id, labelKey, icon: Icon }) => (
            <button
              key={id}
              className={`sidebar-tab ${activeTab === id || (id === "notch" && activeTab === "calendar-sync") ? "active" : ""}`}
              onClick={() => setActiveTab(id)}
            >
              <div className="sidebar-tab-icon">
                <Icon size={14} strokeWidth={1.5} />
              </div>
              <span>{t(labelKey)}</span>
            </button>
          ))}
        </div>

        <div className="settings-content">
          {activeTab === "general" && (
            <GeneralTab
              autostart={settings.autostart}
              weekStart={settings.weekStart}
              changeWeekStart={settings.changeWeekStart}
              toggleAutostart={settings.toggleAutostart}
              timeFormat24h={settings.timeFormat24h}
              toggleTimeFormat24h={settings.toggleTimeFormat24h}
              memorySaver={settings.memorySaver}
              toggleMemorySaver={settings.toggleMemorySaver}
              showUpdateIndicator={settings.showUpdateIndicator}
              toggleUpdateIndicator={settings.toggleUpdateIndicator}
              lowBatteryThreshold={settings.lowBatteryThreshold}
              hasBattery={settings.hasBattery}
              handleThresholdChange={settings.handleThresholdChange}
              restartFern={settings.restartFern}
              quitFern={settings.quitFern}
            />
          )}
          {activeTab === "appearance" && (
            <AppearanceTab
              themeStyle={settings.themeStyle}
              handleThemeStyleChange={settings.handleThemeStyleChange}
              themeMode={settings.themeMode}
              handleThemeModeChange={settings.handleThemeModeChange}
              themeColor={settings.themeColor}
              handleThemeColorChange={settings.handleThemeColorChange}
              themeOpacity={settings.themeOpacity}
              handleOpacityChange={settings.handleOpacityChange}
              themeSaturation={settings.themeSaturation}
              handleSaturationChange={settings.handleSaturationChange}
              themeBrightness={settings.themeBrightness}
              handleBrightnessChange={settings.handleBrightnessChange}
              cornersEnabled={settings.cornersEnabled}
              toggleCorners={settings.toggleCorners}
              scale={settings.scale}
              handleScaleChange={settings.handleScaleChange}
              glassWallpaperEnabled={settings.glassWallpaperEnabled}
              toggleGlassWallpaper={settings.toggleGlassWallpaper}
            />
          )}
          {activeTab === "calendar-sync" && <CalendarSyncPage onBack={() => setActiveTab("notch")} />}
          {activeTab === "notch" && (
            <NotchTab
              onOpenCalendarSync={() => setActiveTab("calendar-sync")}
              notchMode={settings.notchMode}
              setNotchModeValue={settings.setNotchModeValue}
              monitors={settings.monitors}
              notchMonitorMode={settings.notchMonitorMode}
              setNotchMonitorModeValue={settings.setNotchMonitorModeValue}
              notchMonitorId={settings.notchMonitorId}
              setNotchMonitorIdValue={settings.setNotchMonitorIdValue}
              notchModeByMonitor={settings.notchModeByMonitor}
              setNotchModeForMonitor={settings.setNotchModeForMonitor}
              replaceNotchModes={settings.replaceNotchModesByMonitor}
              calendarEnabled={settings.calendarEnabled}
              toggleCalendar={settings.toggleCalendar}
              timerEnabled={settings.timerEnabled}
              toggleTimerEnabled={settings.toggleTimerEnabled}
              timerSoundEnabled={settings.timerSoundEnabled}
              toggleTimerSound={settings.toggleTimerSound}
              notchTimerProgress={settings.notchTimerProgress}
              toggleNotchTimerProgress={settings.toggleNotchTimerProgress}
              stopwatchEnabled={settings.stopwatchEnabled}
              toggleStopwatchEnabled={settings.toggleStopwatchEnabled}
              dualClockEnabled={settings.dualClockEnabled}
              toggleDualClock={settings.toggleDualClock}
              dualClockZone={settings.dualClockZone}
              dualClockLabel={settings.dualClockLabel}
              selectDualClockZone={settings.selectDualClockZone}
              musicModeEnabled={settings.musicModeEnabled}
              toggleMusicMode={settings.toggleMusicMode}
              musicCompactNotch={settings.musicCompactNotch}
              toggleMusicCompactNotch={settings.toggleMusicCompactNotch}
              mediaLayout={settings.mediaLayout}
              toggleMediaLayout={settings.toggleMediaLayout}
              mediaAmbienceEnabled={settings.mediaAmbienceEnabled}
              artworkThemeEnabled={settings.artworkThemeEnabled}
              toggleArtworkTheme={settings.toggleArtworkTheme}
              toggleAmbience={settings.toggleAmbience}
              mediaCompactGlowEnabled={settings.mediaCompactGlowEnabled}
              toggleCompactGlow={settings.toggleCompactGlow}
              lyricsEnabled={settings.lyricsEnabled}
              toggleLyrics={settings.toggleLyrics}
              lyricsSeekEnabled={settings.lyricsSeekEnabled}
              toggleLyricsSeek={settings.toggleLyricsSeek}
              notchScrollVolume={settings.notchScrollVolume}
              toggleNotchScrollVolume={settings.toggleNotchScrollVolume}
              shelfPersist={settings.shelfPersist}
              toggleShelfPersist={settings.toggleShelfPersist}
              shelfKeepOpen={settings.shelfKeepOpen}
              toggleShelfKeepOpen={settings.toggleShelfKeepOpen}
              rememberMode={settings.rememberMode}
              toggleRememberMode={settings.toggleRememberMode}
              panelControlCenter={settings.panelControlCenter}
              togglePanelControlCenter={settings.togglePanelControlCenter}
              panelShelf={settings.panelShelf}
              togglePanelShelf={settings.togglePanelShelf}
              panelStatus={settings.panelStatus}
              togglePanelStatus={settings.togglePanelStatus}
              panelScrollDirection={settings.panelScrollDirection}
              changePanelScrollDirection={settings.changePanelScrollDirection}
              panelOrder={settings.panelOrder}
              reorderPanels={settings.reorderPanels}
              resetPanelOrder={settings.resetPanelOrder}
              ccLayout={settings.ccLayout}
              changeCcLayout={settings.changeCcLayout}
              resetCcLayout={settings.resetCcLayout}
              weatherEnabled={settings.weatherEnabled}
              toggleWeather={settings.toggleWeather}
              tempUnitFahrenheit={settings.tempUnitFahrenheit}
              toggleTempUnit={settings.toggleTempUnit}
              cityName={settings.cityName}
              cityQuery={settings.cityQuery}
              setCityQuery={settings.setCityQuery}
              citySearchResults={settings.citySearchResults}
              showCityDropdown={settings.showCityDropdown}
              setShowCityDropdown={settings.setShowCityDropdown}
              selectCity={settings.selectCity}
              handleCityClear={settings.handleCityClear}
              statusWidgets={settings.statusWidgets}
              handleWidgetsChange={settings.handleWidgetsChange}
              hasBattery={settings.hasBattery}
              agentApiEnabled={settings.agentApiEnabled}
              toggleAgentApi={settings.toggleAgentApi}
              agentSoundsEnabled={settings.agentSoundsEnabled}
              toggleAgentSounds={settings.toggleAgentSounds}
              notchEdgeDelay={settings.notchEdgeDelay}
              handleNotchEdgeDelayChange={settings.handleNotchEdgeDelayChange}
            />
          )}
          {activeTab === "dock" && (
            <DockTab
              dockEnabled={settings.dockEnabled}
              toggleDock={settings.toggleDock}
              dockMode={settings.dockMode}
              setDockModeValue={settings.setDockModeValue}
              dockPreviewEnabled={settings.dockPreviewEnabled}
              toggleDockPreview={settings.toggleDockPreview}
              dockConsoleWindows={settings.dockConsoleWindows}
              toggleDockConsoleWindows={settings.toggleDockConsoleWindows}
              dockAppVolumeScroll={settings.dockAppVolumeScroll}
              toggleDockAppVolumeScroll={settings.toggleDockAppVolumeScroll}
              dockSearchEnabled={settings.dockSearchEnabled}
              toggleDockSearch={settings.toggleDockSearch}
              dockSearchLauncher={settings.dockSearchLauncher}
              setDockSearchLauncherValue={settings.setDockSearchLauncherValue}
              dockCalendarEnabled={settings.dockCalendarEnabled}
              toggleDockCalendar={settings.toggleDockCalendar}
              dockTray={settings.dockTray}
              toggleDockTray={settings.toggleDockTray}
              dockTaskView={settings.dockTaskView}
              toggleDockTaskView={settings.toggleDockTaskView}
              dockShowDesktop={settings.dockShowDesktop}
              toggleDockShowDesktop={settings.toggleDockShowDesktop}
              dockRecycle={settings.dockRecycle}
              toggleDockRecycle={settings.toggleDockRecycle}
              dockDrives={settings.dockDrives}
              toggleDockDrive={settings.toggleDockDrive}
              dockIconOnly={settings.dockIconOnly}
              toggleDockIconOnly={settings.toggleDockIconOnly}
              dockAdaptive={settings.dockAdaptive}
              toggleDockAdaptive={settings.toggleDockAdaptive}
              startIcon={settings.startIcon}
              handleStartIconChange={settings.handleStartIconChange}
              dockWinNumberEnabled={settings.dockWinNumberEnabled}
              toggleDockWinNumber={settings.toggleDockWinNumber}
              dockMixedReorder={settings.dockMixedReorder}
              toggleDockMixedReorder={settings.toggleDockMixedReorder}
              dockAttentionEnabled={settings.dockAttentionEnabled}
              toggleDockAttention={settings.toggleDockAttention}
              dockHideDelay={settings.dockHideDelay}
              handleDockHideDelayChange={settings.handleDockHideDelayChange}
              monitors={settings.monitors}
              dockMonitorMode={settings.dockMonitorMode}
              setDockMonitorModeValue={settings.setDockMonitorModeValue}
              dockMonitorId={settings.dockMonitorId}
              setDockMonitorIdValue={settings.setDockMonitorIdValue}
              dockModeByMonitor={settings.dockModeByMonitor}
              setDockModeForMonitor={settings.setDockModeForMonitor}
              replaceDockModes={settings.replaceDockModesByMonitor}
              dockIndicator={settings.dockIndicator}
              changeDockIndicator={settings.changeDockIndicator}
              dockShowEndTask={settings.dockShowEndTask}
              toggleDockShowEndTask={settings.toggleDockShowEndTask}
              dockMenuItems={settings.dockMenuItems}
              toggleDockMenuItem={settings.toggleDockMenuItem}
            />
          )}
          {activeTab === "overlays" && (
            <OverlaysTab
              volumeOverlayEnabled={settings.volumeOverlayEnabled}
              toggleVolumeOverlay={settings.toggleVolumeOverlay}
              volumeEdgeEnabled={settings.volumeEdgeEnabled}
              toggleVolumeEdge={settings.toggleVolumeEdge}
              volumeSide={settings.volumeSide}
              handleVolumeSideChange={settings.handleVolumeSideChange}
              brightnessOverlayEnabled={settings.brightnessOverlayEnabled}
              toggleBrightnessOverlay={settings.toggleBrightnessOverlay}
              brightnessEdgeEnabled={settings.brightnessEdgeEnabled}
              toggleBrightnessEdge={settings.toggleBrightnessEdge}
              brightnessSide={settings.brightnessSide}
              handleBrightnessSideChange={settings.handleBrightnessSideChange}
              edgeDelay={settings.edgeDelay}
              handleEdgeDelayChange={settings.handleEdgeDelayChange}
            />
          )}
          {activeTab === "diagnostics" && <DiagnosticsTab />}
          {activeTab === "about" && (
            <AboutTab
              appVersion={settings.appVersion}
              autoUpdate={settings.autoUpdate}
              toggleAutoUpdate={settings.toggleAutoUpdate}
              prereleaseChannel={settings.prereleaseChannel}
              togglePrereleaseChannel={settings.togglePrereleaseChannel}
              updateStatus={settings.updateStatus}
              updateVersion={settings.updateVersion}
              checkForUpdates={settings.checkForUpdates}
              installUpdate={settings.installUpdate}
              exportStatus={settings.exportStatus}
              importStatus={settings.importStatus}
              handleExportSettings={settings.handleExportSettings}
              handleImportSettings={settings.handleImportSettings}
              resetStatus={settings.resetStatus}
              handleResetSettings={settings.handleResetSettings}
              uninstallStatus={settings.uninstallStatus}
              uninstallError={settings.uninstallError}
              handleUninstallFern={settings.handleUninstallFern}
            />
          )}
        </div>
      </div>
    </div>
  );
}

createRoot(document.getElementById("root") as HTMLElement).render(
  <StrictMode>
    <SettingsApp />
  </StrictMode>
);
