import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  AppWindow,
  Calculator,
  Clock,
  FolderSearch,
  Globe,
  HardDrive,
  LayoutList,
  Search,
  Terminal,
  Trash2,
} from "lucide-react";
import { SettingRow } from "./SettingRow";
import { SettingSection } from "./SettingSection";
import { Dropdown } from "./Dropdown";
import { OrderList } from "./OrderList";
import { LauncherHotkey } from "./LauncherHotkey";
import { useSettingsSync } from "../hooks/useSettingsSync";
import { useT } from "../i18n";
import {
  IDLE_SECTIONS,
  LAUNCHER_BROWSER_KEY,
  LAUNCHER_IDLE_DISABLED_KEY,
  LAUNCHER_IDLE_ORDER_KEY,
  LAUNCHER_SEARCH_DISABLED_KEY,
  LAUNCHER_SEARCH_ORDER_KEY,
  LAUNCHER_WEB_ENGINE_KEY,
  SEARCH_SECTIONS,
  normalizeDisabled,
  normalizeOrder,
  readBrowserPath,
  readIdleDisabled,
  readIdleOrder,
  readSearchDisabled,
  readSearchOrder,
  readWebEngine,
  type IdleSectionId,
  type SearchSectionId,
} from "../launcher/config";
import { RECENTS_KEY } from "../launcher/engine/apps";

interface BrowserInfo {
  name: string;
  path: string;
}

function saveJsonSetting(key: string, value: string[]) {
  const raw = JSON.stringify(value);
  try {
    localStorage.setItem(key, raw);
  } catch {
    /* ignore */
  }
  invoke("save_setting", { key, value: raw }).catch(() => {});
}

function saveStrSetting(key: string, value: string) {
  try {
    localStorage.setItem(key, value);
  } catch {
    /* ignore */
  }
  invoke("save_setting", { key, value }).catch(() => {});
}

export function LauncherTab() {
  const t = useT();
  const [webEngine, setWebEngine] = useState<string>(readWebEngine);
  const [browserPath, setBrowserPath] = useState<string>(readBrowserPath);
  const [browsers, setBrowsers] = useState<BrowserInfo[]>([]);
  const [idleOrder, setIdleOrder] = useState<IdleSectionId[]>(readIdleOrder);
  const [idleDisabled, setIdleDisabled] = useState<IdleSectionId[]>(readIdleDisabled);
  const [searchOrder, setSearchOrder] = useState<SearchSectionId[]>(readSearchOrder);
  const [searchDisabled, setSearchDisabled] = useState<SearchSectionId[]>(readSearchDisabled);
  const [fileProvider, setFileProvider] = useState<string | null | undefined>(undefined);
  const [clearedRecents, setClearedRecents] = useState(false);

  useEffect(() => {
    let alive = true;
    invoke<BrowserInfo[]>("get_installed_browsers")
      .then((list) => {
        if (alive && Array.isArray(list)) setBrowsers(list);
      })
      .catch(() => {});
    invoke<string | null>("get_file_search_status")
      .then((provider) => {
        if (alive) setFileProvider(provider ?? null);
      })
      .catch(() => {
        if (alive) setFileProvider(null);
      });
    invoke<Record<string, unknown>>("load_settings")
      .then((s) => {
        if (!alive || !s) return;
        if (s[LAUNCHER_WEB_ENGINE_KEY] === "duckduckgo" || s[LAUNCHER_WEB_ENGINE_KEY] === "google") {
          setWebEngine(String(s[LAUNCHER_WEB_ENGINE_KEY]));
        }
        if (typeof s[LAUNCHER_BROWSER_KEY] === "string") {
          setBrowserPath(String(s[LAUNCHER_BROWSER_KEY]));
        }
        if (s[LAUNCHER_IDLE_ORDER_KEY] !== undefined) {
          setIdleOrder(normalizeOrder(s[LAUNCHER_IDLE_ORDER_KEY], IDLE_SECTIONS));
        }
        if (s[LAUNCHER_IDLE_DISABLED_KEY] !== undefined) {
          setIdleDisabled(normalizeDisabled(s[LAUNCHER_IDLE_DISABLED_KEY], IDLE_SECTIONS));
        }
        if (s[LAUNCHER_SEARCH_ORDER_KEY] !== undefined) {
          setSearchOrder(normalizeOrder(s[LAUNCHER_SEARCH_ORDER_KEY], SEARCH_SECTIONS));
        }
        if (s[LAUNCHER_SEARCH_DISABLED_KEY] !== undefined) {
          setSearchDisabled(normalizeDisabled(s[LAUNCHER_SEARCH_DISABLED_KEY], SEARCH_SECTIONS));
        }
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, []);

  useSettingsSync({
    [LAUNCHER_WEB_ENGINE_KEY]: (v) => setWebEngine(v === "duckduckgo" ? "duckduckgo" : "google"),
    [LAUNCHER_BROWSER_KEY]: (v) => setBrowserPath(typeof v === "string" ? v : ""),
    [LAUNCHER_IDLE_ORDER_KEY]: (v) => setIdleOrder(normalizeOrder(v, IDLE_SECTIONS)),
    [LAUNCHER_IDLE_DISABLED_KEY]: (v) => setIdleDisabled(normalizeDisabled(v, IDLE_SECTIONS)),
    [LAUNCHER_SEARCH_ORDER_KEY]: (v) => setSearchOrder(normalizeOrder(v, SEARCH_SECTIONS)),
    [LAUNCHER_SEARCH_DISABLED_KEY]: (v) => setSearchDisabled(normalizeDisabled(v, SEARCH_SECTIONS)),
  });

  const changeWebEngine = (val: string) => {
    const next = val === "duckduckgo" ? "duckduckgo" : "google";
    setWebEngine(next);
    saveStrSetting(LAUNCHER_WEB_ENGINE_KEY, next);
  };

  const changeBrowser = (val: string) => {
    setBrowserPath(val);
    saveStrSetting(LAUNCHER_BROWSER_KEY, val);
  };

  const toggleIdle = (id: string) => {
    const sid = id as IdleSectionId;
    const next = idleDisabled.includes(sid)
      ? idleDisabled.filter((x) => x !== sid)
      : [...idleDisabled, sid];
    setIdleDisabled(next);
    saveJsonSetting(LAUNCHER_IDLE_DISABLED_KEY, next);
  };

  const reorderIdle = (next: string[]) => {
    const norm = normalizeOrder(next, IDLE_SECTIONS);
    setIdleOrder(norm);
    saveJsonSetting(LAUNCHER_IDLE_ORDER_KEY, norm);
  };

  const resetIdle = () => {
    const order = [...IDLE_SECTIONS];
    setIdleOrder(order);
    setIdleDisabled([]);
    saveJsonSetting(LAUNCHER_IDLE_ORDER_KEY, order);
    saveJsonSetting(LAUNCHER_IDLE_DISABLED_KEY, []);
  };

  const toggleSearch = (id: string) => {
    const sid = id as SearchSectionId;
    const next = searchDisabled.includes(sid)
      ? searchDisabled.filter((x) => x !== sid)
      : [...searchDisabled, sid];
    setSearchDisabled(next);
    saveJsonSetting(LAUNCHER_SEARCH_DISABLED_KEY, next);
  };

  const reorderSearch = (next: string[]) => {
    const norm = normalizeOrder(next, SEARCH_SECTIONS);
    setSearchOrder(norm);
    saveJsonSetting(LAUNCHER_SEARCH_ORDER_KEY, norm);
  };

  const resetSearch = () => {
    const order = [...SEARCH_SECTIONS];
    setSearchOrder(order);
    setSearchDisabled([]);
    saveJsonSetting(LAUNCHER_SEARCH_ORDER_KEY, order);
    saveJsonSetting(LAUNCHER_SEARCH_DISABLED_KEY, []);
  };

  const clearRecents = () => {
    try {
      localStorage.setItem(RECENTS_KEY, "[]");
    } catch {
      /* ignore */
    }
    invoke("save_setting", { key: RECENTS_KEY, value: "[]" }).catch(() => {});
    setClearedRecents(true);
    setTimeout(() => setClearedRecents(false), 1800);
  };

  const idleIcon = (id: string) =>
    id === "recents" ? <Clock size={14} strokeWidth={1.9} /> : <AppWindow size={14} strokeWidth={1.9} />;

  const searchIcon = (id: string) => {
    switch (id) {
      case "calculator":
        return <Calculator size={14} strokeWidth={1.9} />;
      case "apps":
        return <AppWindow size={14} strokeWidth={1.9} />;
      case "commands":
        return <Terminal size={14} strokeWidth={1.9} />;
      case "files":
        return <FolderSearch size={14} strokeWidth={1.9} />;
      default:
        return <Globe size={14} strokeWidth={1.9} />;
    }
  };

  const browserOptions = [
    { value: "", label: t("launcherSettings.browser.default") },
    ...browsers.map((b) => ({ value: b.path, label: b.name })),
  ];
  if (browserPath && !browserOptions.some((o) => o.value === browserPath)) {
    const fallbackName = browserPath.split(/[/\\]/).pop() || browserPath;
    browserOptions.push({ value: browserPath, label: fallbackName });
  }

  return (
    <>
      <SettingSection
        id="launcher-general"
        title={t("launcherSettings.section.general")}
        defaultOpen
      >
        <LauncherHotkey />

        <SettingRow
          icon={Search}
          label={t("dock.launcherWebEngine")}
          desc={t("dock.launcherWebEngine.desc")}
        >
          <Dropdown
            value={webEngine}
            onChange={changeWebEngine}
            options={[
              { value: "google", label: "Google" },
              { value: "duckduckgo", label: "DuckDuckGo" },
            ]}
          />
        </SettingRow>

        <SettingRow
          icon={Globe}
          label={t("launcherSettings.browser")}
          desc={t("launcherSettings.browser.desc")}
          divider={false}
        >
          <Dropdown
            value={browserPath}
            onChange={changeBrowser}
            options={browserOptions}
          />
        </SettingRow>
      </SettingSection>

      <SettingSection
        id="launcher-idle"
        title={t("launcherSettings.section.idle")}
      >
        <SettingRow
          icon={LayoutList}
          label={t("launcherSettings.section.idle")}
          desc={t("launcherSettings.section.idle.summary")}
          stacked
        >
          <OrderList
            ids={idleOrder}
            label={(id) => t(`launcherSettings.idle.${id}`)}
            icon={idleIcon}
            enabled={(id) => !idleDisabled.includes(id as IdleSectionId)}
            onToggle={toggleIdle}
            onReorder={reorderIdle}
            onReset={resetIdle}
            resetLabel={t("launcherSettings.resetOrder")}
          />
        </SettingRow>

        <SettingRow
          icon={Trash2}
          label={t("launcherSettings.clearRecents")}
          desc={t("launcherSettings.clearRecents.desc")}
          action
          divider={false}
          onClick={clearRecents}
        >
          <span
            style={{
              fontSize: "12px",
              fontWeight: 500,
              color: clearedRecents
                ? "#34d399"
                : "var(--fern-text-muted, rgba(255, 255, 255, 0.65))",
            }}
          >
            {clearedRecents
              ? t("launcherSettings.clearRecents.done")
              : t("launcherSettings.clearRecents.btn")}
          </span>
        </SettingRow>
      </SettingSection>

      <SettingSection
        id="launcher-modules"
        title={t("launcherSettings.section.modules")}
      >
        <SettingRow
          icon={LayoutList}
          label={t("launcherSettings.section.modules")}
          desc={t("launcherSettings.section.modules.summary")}
          stacked
        >
          <OrderList
            ids={searchOrder}
            label={(id) => t(`launcherSettings.module.${id}`)}
            icon={searchIcon}
            enabled={(id) => !searchDisabled.includes(id as SearchSectionId)}
            onToggle={toggleSearch}
            onReorder={reorderSearch}
            onReset={resetSearch}
            resetLabel={t("launcherSettings.resetOrder")}
          />
        </SettingRow>

        <SettingRow
          icon={HardDrive}
          label={t("launcherSettings.everythingStatus")}
          desc={
            fileProvider
              ? t("launcherSettings.everythingStatus.connected")
              : t("launcherSettings.everythingStatus.missing")
          }
          divider={false}
        >
          <span
            style={{
              fontSize: "12px",
              fontWeight: 500,
              padding: "3px 9px",
              borderRadius: "999px",
              background: fileProvider
                ? "rgba(52, 211, 153, 0.16)"
                : "var(--fern-group-bg, rgba(255,255,255,0.08))",
              color: fileProvider
                ? "#34d399"
                : "var(--fern-text-muted, rgba(255,255,255,0.65))",
              border: fileProvider
                ? "1px solid rgba(52, 211, 153, 0.35)"
                : "1px solid var(--fern-border, rgba(255,255,255,0.1))",
            }}
          >
            {fileProvider === undefined
              ? "..."
              : fileProvider
                ? t("launcherSettings.everythingStatus.badgeOn")
                : t("launcherSettings.everythingStatus.badgeOff")}
          </span>
        </SettingRow>
      </SettingSection>
    </>
  );
}
