import { NotchHotkey } from "./NotchHotkey";
import { MicMuteHotkey } from "./MicMuteHotkey";
import { PrivacyAuraSetting } from "./PrivacyAuraSetting";
import { NetWidgetMode } from "./NetWidgetMode";
import {
  PanelTop,
  Calendar,
  Music,
  Music2,
  Minimize2,
  LayoutList,
  Sparkles,
  Palette,
  Circle,
  CloudSun,
  X,
  Monitor,
  MonitorCheck,
  CloudCog,
  ChevronRight,
  Hourglass,
  Timer,
  BellRing,
  Globe,
  Volume2,
  Clock,
  Bot,
  Layers,
  Archive,
} from "lucide-react";
import { DEFAULT_PANEL_ORDER, type CcLayout, type PanelId } from "../panelOrder";
import { ControlCenterConfig } from "../components/ControlCenterConfig";
import { OrderList } from "./OrderList";
import { SettingRow } from "./SettingRow";
import { SettingSection } from "./SettingSection";
import { Dropdown } from "./Dropdown";
import { useCaldav } from "../hooks/useCaldav";
import { StatusWidgetConfig } from "../components/StatusWidgetConfig";
import { DualClockPicker } from "../components/DualClockPicker";
import { zoneOffsetLabel } from "../dualClock";
import type { WidgetConfig, MonitorInfo, MonitorMode } from "./types";
import { usePerMonitor } from "./usePerMonitor";
import { useT } from "../i18n";
import { AgentSounds } from "./AgentSounds";

interface NotchTabProps {
  notchMode: string;
  setNotchModeValue: (mode: string) => void;
  monitors: MonitorInfo[];
  notchMonitorMode: MonitorMode;
  setNotchMonitorModeValue: (mode: MonitorMode) => void;
  notchMonitorId: string;
  setNotchMonitorIdValue: (id: string) => void;
  notchModeByMonitor: Record<string, string>;
  setNotchModeForMonitor: (monitorId: string, mode: string | null) => void;
  replaceNotchModes: (next: Record<string, string>) => void;
  calendarEnabled: boolean;
  toggleCalendar: () => void;
  timerEnabled: boolean;
  toggleTimerEnabled: () => void;
  timerSoundEnabled: boolean;
  toggleTimerSound: () => void;
  notchTimerProgress: boolean;
  toggleNotchTimerProgress: () => void;
  stopwatchEnabled: boolean;
  toggleStopwatchEnabled: () => void;
  dualClockEnabled: boolean;
  toggleDualClock: () => void;
  dualClockZone: string;
  selectDualClockZone: (entry: { label: string; zone: string }) => void;
  dualClockLabel: string;
  musicModeEnabled: boolean;
  toggleMusicMode: () => void;
  musicCompactNotch: boolean;
  toggleMusicCompactNotch: () => void;
  mediaLayout: "classic" | "compact";
  toggleMediaLayout: (layout: "classic" | "compact") => void;
  artworkThemeEnabled: boolean;
  toggleArtworkTheme: () => void;
  mediaAmbienceEnabled: boolean;
  toggleAmbience: () => void;
  mediaCompactGlowEnabled: boolean;
  toggleCompactGlow: () => void;
  lyricsEnabled?: boolean;
  toggleLyrics?: () => void;
  lyricsSeekEnabled?: boolean;
  toggleLyricsSeek?: () => void;
  panelControlCenter: boolean;
  togglePanelControlCenter: () => void;
  panelShelf: boolean;
  togglePanelShelf: () => void;
  shelfPersist: boolean;
  toggleShelfPersist: () => void;
  shelfKeepOpen: boolean;
  toggleShelfKeepOpen: () => void;
  rememberMode: boolean;
  toggleRememberMode: () => void;
  panelOrder: PanelId[] | null;
  reorderPanels: (order: PanelId[]) => void;
  resetPanelOrder: () => void;
  ccLayout: CcLayout;
  changeCcLayout: (next: CcLayout) => void;
  resetCcLayout: () => void;
  notchScrollVolume: boolean;
  toggleNotchScrollVolume: () => void;
  panelStatus: boolean;
  togglePanelStatus: () => void;
  panelScrollDirection: string;
  changePanelScrollDirection: (direction: string) => void;
  weatherEnabled: boolean;
  toggleWeather: () => void;
  tempUnitFahrenheit: boolean;
  toggleTempUnit: () => void;
  cityName: string;
  cityQuery: string;
  setCityQuery: (q: string) => void;
  citySearchResults: { name: string; country: string; latitude: number; longitude: number }[];
  selectCity: (c: { name: string; country: string; latitude: number; longitude: number }) => void;
  handleCityClear: () => void;
  showCityDropdown: boolean;
  setShowCityDropdown: (show: boolean) => void;
  statusWidgets: WidgetConfig;
  handleWidgetsChange: (widgets: WidgetConfig) => void;
  hasBattery: boolean;
  onOpenCalendarSync: () => void;
  agentApiEnabled: boolean;
  toggleAgentApi: () => void;
  agentSoundsEnabled: boolean;
  toggleAgentSounds: () => void;
  notchEdgeDelay: string;
  handleNotchEdgeDelayChange: (delay: string) => void;
}

export function NotchTab({
  notchMode,
  setNotchModeValue,
  monitors,
  notchMonitorMode,
  setNotchMonitorModeValue,
  notchMonitorId,
  setNotchMonitorIdValue,
  notchModeByMonitor,
  setNotchModeForMonitor,
  replaceNotchModes,
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
  selectDualClockZone,
  dualClockLabel,
  musicModeEnabled,
  toggleMusicMode,
  musicCompactNotch,
  toggleMusicCompactNotch,
  mediaLayout,
  toggleMediaLayout,
  artworkThemeEnabled,
  toggleArtworkTheme,
  mediaAmbienceEnabled,
  toggleAmbience,
  mediaCompactGlowEnabled,
  toggleCompactGlow,
  lyricsEnabled = true,
  toggleLyrics,
  lyricsSeekEnabled = true,
  toggleLyricsSeek,
  panelControlCenter,
  togglePanelControlCenter,
  panelShelf,
  togglePanelShelf,
  shelfPersist,
  toggleShelfPersist,
  shelfKeepOpen,
  toggleShelfKeepOpen,
  rememberMode,
  toggleRememberMode,
  panelOrder,
  reorderPanels,
  resetPanelOrder,
  ccLayout,
  changeCcLayout,
  resetCcLayout,
  notchScrollVolume,
  toggleNotchScrollVolume,
  panelStatus,
  togglePanelStatus,
  panelScrollDirection,
  changePanelScrollDirection,
  weatherEnabled,
  toggleWeather,
  tempUnitFahrenheit,
  toggleTempUnit,
  cityName,
  cityQuery,
  setCityQuery,
  citySearchResults,
  selectCity,
  handleCityClear,
  showCityDropdown,
  setShowCityDropdown,
  statusWidgets,
  handleWidgetsChange,
  hasBattery,
  onOpenCalendarSync,
  agentApiEnabled,
  toggleAgentApi,
  agentSoundsEnabled,
  toggleAgentSounds,
  notchEdgeDelay,
  handleNotchEdgeDelayChange,
}: NotchTabProps) {
  const t = useT();
  const caldav = useCaldav();
  const sources = (caldav.account ? 1 : 0) + caldav.links.length;
  const perMonitor = usePerMonitor(notchMode, notchModeByMonitor, replaceNotchModes, monitors);
  const multiMonitor = notchMonitorMode === "all" && monitors.length > 1;
  const showPerMonitor = multiMonitor && perMonitor.enabled;

  return (
    <>
      {/* ─── 1. GÖRÜNÜM VE GİZLİLİK (DISPLAY & PRIVACY) ─── */}
      <SettingSection id="notch-display" title={t("notch.section.display")} defaultOpen>
        {!showPerMonitor && (
          <>
            <SettingRow icon={PanelTop} label={t("notch.behavior")} desc={t("notch.behavior.desc")}>
              <Dropdown
                value={notchMode}
                onChange={setNotchModeValue}
                options={[
                  { value: "fixed", label: t("common.fixed") },
                  { value: "smart", label: t("common.smart") },
                  { value: "peek", label: t("common.peek") },
                ]}
              />
            </SettingRow>
            {(notchMode === "smart" || notchMode === "peek") && (
              <SettingRow icon={Clock} label={t("notch.edgeDelay")} desc={t("notch.edgeDelay.desc")}>
                <Dropdown
                  value={notchEdgeDelay}
                  onChange={handleNotchEdgeDelayChange}
                  options={[
                    { value: "0", label: "0 ms" },
                    { value: "150", label: "150 ms" },
                    { value: "300", label: "300 ms" },
                    { value: "500", label: "500 ms" },
                  ]}
                />
              </SettingRow>
            )}
          </>
        )}

        <SettingRow icon={MonitorCheck} label={t("common.showOn")} desc={t("notch.showOn.desc")}>
          <Dropdown
            value={notchMonitorMode}
            onChange={(v) => setNotchMonitorModeValue(v as MonitorMode)}
            options={[
              { value: "primary", label: t("common.monitors.primary") },
              { value: "all", label: t("common.monitors.all") },
              { value: "specific", label: t("common.monitors.specific") },
            ]}
          />
        </SettingRow>

        {notchMonitorMode === "specific" && (
          <SettingRow icon={Monitor} label={t("common.monitor")} desc={t("notch.monitor.desc")}>
            <Dropdown
              value={notchMonitorId}
              onChange={setNotchMonitorIdValue}
              placeholder={monitors.length === 0 ? t("common.loading") : undefined}
              options={monitors.map((m) => ({
                value: m.id,
                label: `${m.label}${m.is_primary ? ` (${t("common.primary")})` : ""}`,
              }))}
            />
          </SettingRow>
        )}

        {multiMonitor && (
          <SettingRow icon={Monitor} label={t("common.perMonitor")} desc={t("common.perMonitor.desc")}>
            <label className="toggle-switch">
              <input type="checkbox" checked={perMonitor.enabled} onChange={perMonitor.toggle} />
              <span className="slider"></span>
            </label>
          </SettingRow>
        )}

        {showPerMonitor &&
          monitors.map((m) => (
            <SettingRow
              key={m.id}
              icon={Monitor}
              label={t("common.monitorBehavior", { name: m.label })}
              desc={m.is_primary ? t("common.primaryMonitor") : undefined}
            >
              <Dropdown
                value={notchModeByMonitor[m.id] || notchMode}
                onChange={(v) => setNotchModeForMonitor(m.id, v)}
                options={[
                  { value: "fixed", label: t("common.fixed") },
                  { value: "smart", label: t("common.smart") },
                  { value: "peek", label: t("common.peek") },
                ]}
              />
            </SettingRow>
          ))}

        <PrivacyAuraSetting />
      </SettingSection>

      {/* ─── 2. PANELLER VE DENETİM MERKEZİ (PANELS & CONTROL CENTER) ─── */}
      <SettingSection id="notch-panels" title={t("notch.section.panels")}>
        <SettingRow icon={Layers} label={t("notch.panelControlCenter")} desc={t("notch.panelControlCenter.desc")}>
          <label className="toggle-switch">
            <input type="checkbox" checked={panelControlCenter} onChange={togglePanelControlCenter} />
            <span className="slider"></span>
          </label>
        </SettingRow>

        <SettingRow icon={Archive} label={t("notch.panelShelf")} desc={t("notch.panelShelf.desc")}>
          <label className="toggle-switch">
            <input type="checkbox" checked={panelShelf} onChange={togglePanelShelf} />
            <span className="slider"></span>
          </label>
        </SettingRow>

        <SettingRow icon={Circle} label={t("notch.panelStatus")} desc={t("notch.panelStatus.desc")}>
          <label className="toggle-switch">
            <input type="checkbox" checked={panelStatus} onChange={togglePanelStatus} />
            <span className="slider"></span>
          </label>
        </SettingRow>

        <SettingRow icon={Circle} label={t("notch.scrollDirection")} desc={t("notch.scrollDirection.desc")}>
          <Dropdown
            value={panelScrollDirection}
            onChange={changePanelScrollDirection}
            options={[
              { value: "natural", label: t("notch.scrollDirection.natural") },
              { value: "inverted", label: t("notch.scrollDirection.inverted") },
            ]}
          />
        </SettingRow>

        {panelShelf && (
          <>
            <SettingRow icon={Circle} label={t("notch.shelfPersist")} desc={t("notch.shelfPersist.desc")}>
              <label className="toggle-switch">
                <input type="checkbox" checked={shelfPersist} onChange={toggleShelfPersist} />
                <span className="slider"></span>
              </label>
            </SettingRow>

            <SettingRow icon={Circle} label={t("notch.shelfKeepOpen")} desc={t("notch.shelfKeepOpen.desc")}>
              <label className="toggle-switch">
                <input type="checkbox" checked={shelfKeepOpen} onChange={toggleShelfKeepOpen} />
                <span className="slider"></span>
              </label>
            </SettingRow>
          </>
        )}

        <SettingRow icon={Circle} label={t("notch.rememberMode")} desc={t("notch.rememberMode.desc")}>
          <label className="toggle-switch">
            <input type="checkbox" checked={rememberMode} onChange={toggleRememberMode} />
            <span className="slider"></span>
          </label>
        </SettingRow>

        <SettingRow icon={LayoutList} label={t("notch.panelOrder")} desc={t("notch.panelOrder.desc")} stacked>
          <OrderList
            ids={panelOrder ?? DEFAULT_PANEL_ORDER}
            label={(id) => t(`panel.${id}`)}
            onReorder={(order) => reorderPanels(order as PanelId[])}
            onReset={panelOrder ? resetPanelOrder : undefined}
            resetLabel={t("notch.panelOrder.reset")}
          />
        </SettingRow>

        {panelControlCenter && (
          <SettingRow icon={Circle} label={t("notch.section.cc")} desc="" stacked divider={false}>
            <ControlCenterConfig
              value={ccLayout}
              onChange={changeCcLayout}
              onReset={resetCcLayout}
              customized={Boolean(ccLayout.tiles || ccLayout.buttons || ccLayout.sliders)}
            />
          </SettingRow>
        )}
      </SettingSection>

      {/* ─── 3. MÜZİK VE MEDYA (MUSIC & MEDIA) ─── */}
      <SettingSection id="notch-music" title={t("notch.section.music")}>
        <SettingRow icon={Music} label={t("notch.musicMode")} desc={t("notch.musicMode.desc")}>
          <label className="toggle-switch">
            <input type="checkbox" checked={musicModeEnabled} onChange={toggleMusicMode} />
            <span className="slider"></span>
          </label>
        </SettingRow>

        {musicModeEnabled && (
          <>
            <SettingRow icon={Minimize2} label={t("notch.compactMode")} desc={t("notch.compactMode.desc")}>
              <label className="toggle-switch">
                <input type="checkbox" checked={musicCompactNotch} onChange={toggleMusicCompactNotch} />
                <span className="slider"></span>
              </label>
            </SettingRow>

            <SettingRow icon={LayoutList} label={t("notch.mediaLayout")} desc={t("notch.mediaLayout.desc")}>
              <div className="unit-toggle-minimal wide">
                <span className={mediaLayout === "classic" ? "active" : ""} onClick={() => toggleMediaLayout("classic")}>
                  {t("notch.mediaLayout.classic")}
                </span>
                <span className={mediaLayout === "compact" ? "active" : ""} onClick={() => toggleMediaLayout("compact")}>
                  {t("notch.mediaLayout.compact")}
                </span>
              </div>
            </SettingRow>

            <SettingRow icon={Palette} label={t("notch.artworkTheme")} desc={t("notch.artworkTheme.desc")}>
              <label className="toggle-switch">
                <input type="checkbox" checked={artworkThemeEnabled} onChange={toggleArtworkTheme} />
                <span className="slider"></span>
              </label>
            </SettingRow>

            {!artworkThemeEnabled && (
              <SettingRow icon={Sparkles} label={t("notch.ambientGlow")} desc={t("notch.ambientGlow.desc")}>
                <label className="toggle-switch">
                  <input type="checkbox" checked={mediaAmbienceEnabled} onChange={toggleAmbience} />
                  <span className="slider"></span>
                </label>
              </SettingRow>
            )}

            <SettingRow icon={Circle} label={t("notch.compactGlow")} desc={t("notch.compactGlow.desc")}>
              <label className="toggle-switch">
                <input type="checkbox" checked={mediaCompactGlowEnabled} onChange={toggleCompactGlow} />
                <span className="slider"></span>
              </label>
            </SettingRow>

            {toggleLyrics && (
              <SettingRow icon={Music2} label={t("notch.lyrics")} desc={t("notch.lyrics.desc")}>
                <label className="toggle-switch">
                  <input type="checkbox" checked={lyricsEnabled} onChange={toggleLyrics} />
                  <span className="slider"></span>
                </label>
              </SettingRow>
            )}

            {lyricsEnabled && toggleLyricsSeek && (
              <SettingRow icon={Sparkles} label={t("notch.lyricsSeek")} desc={t("notch.lyricsSeek.desc")}>
                <label className="toggle-switch">
                  <input type="checkbox" checked={lyricsSeekEnabled} onChange={toggleLyricsSeek} />
                  <span className="slider"></span>
                </label>
              </SettingRow>
            )}

            <SettingRow icon={Volume2} label={t("notch.scrollVolume")} desc={t("notch.scrollVolume.desc")} divider={false}>
              <label className="toggle-switch">
                <input type="checkbox" checked={notchScrollVolume} onChange={toggleNotchScrollVolume} />
                <span className="slider"></span>
              </label>
            </SettingRow>
          </>
        )}
      </SettingSection>

      {/* ─── 4. DURUM ARAÇLARI VE HAVA DURUMU (WIDGETS & WEATHER) ─── */}
      <SettingSection id="notch-widgets" title={t("notch.widgets.group")}>
        <StatusWidgetConfig value={statusWidgets} onChange={handleWidgetsChange} hasBattery={hasBattery} />
        <NetWidgetMode />

        <SettingRow icon={CloudSun} label={t("notch.weather")} desc={cityName || t("notch.weather.auto")} divider={false}>
          <div className="weather-controls">
            <div className="unit-toggle-minimal" onClick={toggleTempUnit}>
              <span className={!tempUnitFahrenheit ? "active" : ""}>C</span>
              <span className={tempUnitFahrenheit ? "active" : ""}>F</span>
            </div>
            <label className="toggle-switch">
              <input type="checkbox" checked={weatherEnabled} onChange={toggleWeather} />
              <span className="slider"></span>
            </label>
          </div>
        </SettingRow>

        {weatherEnabled && (
          <div className="manual-city-input">
            <div className="city-input-row">
              <input
                type="text"
                placeholder={t("notch.weather.search")}
                value={cityQuery}
                onChange={(e) => setCityQuery(e.target.value)}
                onFocus={() => citySearchResults.length > 0 && setShowCityDropdown(true)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" && citySearchResults.length > 0) {
                    e.preventDefault();
                    selectCity(citySearchResults[0]);
                  }
                  if (e.key === "Escape") {
                    setShowCityDropdown(false);
                  }
                }}
                onBlur={() =>
                  setTimeout(() => {
                    setShowCityDropdown(false);
                    if (cityQuery !== cityName && citySearchResults.length > 0) selectCity(citySearchResults[0]);
                  }, 150)
                }
              />
              {(cityName || cityQuery) && (
                <button
                  className="city-clear-btn"
                  onMouseDown={(e) => {
                    e.preventDefault();
                    handleCityClear();
                  }}
                  title={t("notch.weather.clear")}
                >
                  <X size={10} strokeWidth={2.5} />
                </button>
              )}
            </div>
            {showCityDropdown && citySearchResults.length > 0 && (
              <div className="city-dropdown">
                {citySearchResults.map((city) => (
                  <button
                    key={`${city.name}-${city.latitude}`}
                    className="city-dropdown-item"
                    onMouseDown={(e) => {
                      e.preventDefault();
                      selectCity(city);
                    }}
                  >
                    <span className="city-dropdown-name">{city.name}</span>
                    <span className="city-dropdown-country">{city.country}</span>
                  </button>
                ))}
              </div>
            )}
          </div>
        )}
      </SettingSection>

      {/* ─── 5. ZAMAN VE TAKVİM (TIME & PRODUCTIVITY) ─── */}
      <SettingSection id="notch-calendar" title={t("notch.section.calendar")}>
        <SettingRow icon={Calendar} label={t("notch.calendar")} desc={t("notch.calendar.desc")}>
          <label className="toggle-switch">
            <input type="checkbox" checked={calendarEnabled} onChange={toggleCalendar} />
            <span className="slider"></span>
          </label>
        </SettingRow>

        {calendarEnabled && (
          <SettingRow
            icon={CloudCog}
            label={t("notch.calendarSync")}
            desc={
              sources
                ? t(sources === 1 ? "notch.calendarSync.connectedOne" : "notch.calendarSync.connectedMany", { count: sources })
                : t("notch.calendarSync.desc")
            }
            action
            onClick={onOpenCalendarSync}
          >
            <ChevronRight size={14} strokeWidth={1.5} />
          </SettingRow>
        )}

        <SettingRow
          icon={Globe}
          label={t("notch.dualClock")}
          desc={dualClockEnabled ? [dualClockLabel, zoneOffsetLabel(dualClockZone)].filter(Boolean).join(" · ") : t("notch.dualClock.desc")}
        >
          <label className="toggle-switch">
            <input type="checkbox" checked={dualClockEnabled} onChange={toggleDualClock} />
            <span className="slider"></span>
          </label>
        </SettingRow>

        {dualClockEnabled && (
          <>
            <DualClockPicker onSelect={selectDualClockZone} />
            <div className="setting-divider" />
          </>
        )}

        <SettingRow icon={Hourglass} label={t("notch.timer")} desc={t("notch.timer.desc")}>
          <label className="toggle-switch">
            <input type="checkbox" checked={timerEnabled} onChange={toggleTimerEnabled} />
            <span className="slider"></span>
          </label>
        </SettingRow>

        {timerEnabled && (
          <>
            <SettingRow icon={BellRing} label={t("notch.timerSound")} desc={t("notch.timerSound.desc")}>
              <label className="toggle-switch">
                <input type="checkbox" checked={timerSoundEnabled} onChange={toggleTimerSound} />
                <span className="slider"></span>
              </label>
            </SettingRow>

            <SettingRow icon={Timer} label={t("notch.timerProgress")} desc={t("notch.timerProgress.desc")}>
              <label className="toggle-switch">
                <input type="checkbox" checked={notchTimerProgress} onChange={toggleNotchTimerProgress} />
                <span className="slider"></span>
              </label>
            </SettingRow>
          </>
        )}

        <SettingRow icon={Timer} label={t("notch.stopwatch")} desc={t("notch.stopwatch.desc")} divider={false}>
          <label className="toggle-switch">
            <input type="checkbox" checked={stopwatchEnabled} onChange={toggleStopwatchEnabled} />
            <span className="slider"></span>
          </label>
        </SettingRow>
      </SettingSection>

      {/* ─── 6. YAPAY ZEKÂ VE FERNIE (AI AGENT & COMPANION) ─── */}
      <SettingSection id="notch-agent" title={t("notch.agent.group")}>
        <SettingRow icon={Bot} label={t("notch.agent.enable")} desc={t("notch.agent.enable.desc")}>
          <label className="toggle-switch">
            <input type="checkbox" checked={agentApiEnabled} onChange={toggleAgentApi} />
            <span className="slider"></span>
          </label>
        </SettingRow>

        {agentApiEnabled && (
          <>
            <SettingRow icon={Volume2} label={t("notch.agent.sounds")} desc={t("notch.agent.sounds.desc")}>
              <label className="toggle-switch">
                <input type="checkbox" checked={agentSoundsEnabled} onChange={toggleAgentSounds} />
                <span className="slider"></span>
              </label>
            </SettingRow>

            {agentSoundsEnabled && <AgentSounds />}
          </>
        )}
      </SettingSection>

      {/* ─── 7. KLAVYE KISAYOLLARI (KEYBOARD SHORTCUTS) ─── */}
      <SettingSection id="notch-hotkeys" title={t("notch.hotkey.group")}>
        <NotchHotkey />
        <MicMuteHotkey />
      </SettingSection>
    </>
  );
}
