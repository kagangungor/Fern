import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { MouseEvent } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import { motion, useReducedMotion } from "framer-motion";
import {
  Activity,
  AlertTriangle,
  AppWindow,
  Calculator,
  Calendar,
  Copy,
  ExternalLink,
  File,
  FileText,
  Folder,
  Globe,
  Lock,
  Mic,
  Monitor,
  Moon,
  RotateCw,
  Search,
  Settings,
  Sparkles,
  Trash2,
  X,
} from "lucide-react";
import { useSettingsSync } from "../hooks/useSettingsSync";
import { localizeAppName, useT } from "../i18n";
import {
  appKey,
  fetchRunningApps,
  loadInstalledApps,
  readRecents,
  recordRecent,
} from "./engine/apps";
import { evaluateExpression } from "./engine/calculator";
import { buildQuickCommands, matchQuickCommands } from "./engine/commands";
import { fuzzyScore } from "./engine/fuzzy";
import { buildWebSearchUrl, detectDirectUrl } from "./engine/web";
import type {
  FileSearchResult,
  LauncherItem,
  QuickCommand,
  SearchApp,
  WebEngine,
} from "./types";

const WEB_ENGINE_KEY = "fern-launcher-web-engine";
const TEXT_EXTENSIONS = new Set([
  "txt",
  "md",
  "json",
  "ts",
  "tsx",
  "js",
  "jsx",
  "rs",
  "py",
  "toml",
  "yaml",
  "yml",
  "css",
  "html",
  "xml",
  "csv",
  "log",
  "ini",
  "cfg",
  "doc",
  "docx",
  "pdf",
]);

function readWebEngine(): WebEngine {
  try {
    return localStorage.getItem(WEB_ENGINE_KEY) === "duckduckgo"
      ? "duckduckgo"
      : "google";
  } catch {
    return "google";
  }
}

function normalizePath(p: string): string {
  return p.trim().toLowerCase().replace(/\//g, "\\");
}

function webEngineLabel(engine: WebEngine): string {
  return engine === "duckduckgo" ? "DuckDuckGo" : "Google";
}

async function copyToClipboard(text: string): Promise<void> {
  if (navigator.clipboard?.writeText) {
    try {
      await navigator.clipboard.writeText(text);
      return;
    } catch {
      // Fallback to execCommand below
    }
  }
  const textarea = document.createElement("textarea");
  textarea.value = text;
  textarea.setAttribute("readonly", "");
  textarea.style.position = "fixed";
  textarea.style.opacity = "0";
  document.body.appendChild(textarea);
  textarea.select();
  try {
    document.execCommand("copy");
  } finally {
    document.body.removeChild(textarea);
  }
}

function CommandIcon({ icon }: { icon: QuickCommand["icon"] }) {
  const props = { size: 16, strokeWidth: 2, "aria-hidden": true as const };
  switch (icon) {
    case "settings":
      return <Settings {...props} />;
    case "calendar":
      return <Calendar {...props} />;
    case "mic":
      return <Mic {...props} />;
    case "sparkles":
      return <Sparkles {...props} />;
    case "rotate":
      return <RotateCw {...props} />;
    case "activity":
      return <Activity {...props} />;
    case "monitor":
      return <Monitor {...props} />;
    case "lock":
      return <Lock {...props} />;
    case "moon":
      return <Moon {...props} />;
    case "trash":
      return <Trash2 {...props} />;
  }
}

function FileIcon({ file }: { file: FileSearchResult }) {
  if (file.is_dir) {
    return <Folder size={16} strokeWidth={2} aria-hidden="true" />;
  }
  const ext = file.extension?.toLowerCase() ?? "";
  if (TEXT_EXTENSIONS.has(ext)) {
    return <FileText size={16} strokeWidth={2} aria-hidden="true" />;
  }
  return <File size={16} strokeWidth={2} aria-hidden="true" />;
}

export function LauncherApp() {
  const t = useT();
  const reducedMotion = useReducedMotion();

  const [query, setQuery] = useState("");
  const [selectedIndex, setSelectedIndex] = useState(0);
  const [confirmId, setConfirmId] = useState<string | null>(null);
  const [installedApps, setInstalledApps] = useState<SearchApp[]>(
    () => loadInstalledApps().cached
  );
  const [runningApps, setRunningApps] = useState<SearchApp[]>([]);
  const [recents, setRecents] = useState<string[]>(() => readRecents());
  const [fileResults, setFileResults] = useState<FileSearchResult[]>([]);
  const [webEngine, setWebEngine] = useState<WebEngine>(readWebEngine);
  const [icons, setIcons] = useState<Record<string, string | null>>({});
  const [openSeq, setOpenSeq] = useState(0);
  const [langVersion, setLangVersion] = useState(0);

  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const requestIdRef = useRef(0);
  const requestedIconsRef = useRef<Set<string>>(new Set());
  const mountedRef = useRef(true);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  const commands = useMemo(() => buildQuickCommands(), []);

  // Sync live settings changes (web engine & language; theme is handled by initTheme)
  useSettingsSync({
    [WEB_ENGINE_KEY]: (value) => {
      const next: WebEngine = value === "duckduckgo" ? "duckduckgo" : "google";
      try {
        localStorage.setItem(WEB_ENGINE_KEY, next);
      } catch {
        // Ignore storage errors
      }
      setWebEngine(next);
    },
    "fern-language": (value) => {
      try {
        localStorage.setItem("fern-language", String(value));
      } catch {
        // Ignore storage errors
      }
      setLangVersion((v) => v + 1);
    },
  });

  // Initial load & readiness handshake
  useEffect(() => {
    let active = true;

    invoke("launcher_ready").catch(() => {});

    const { ready } = loadInstalledApps();
    ready.then((apps) => {
      if (active) setInstalledApps(apps);
    });

    fetchRunningApps().then((running) => {
      if (active) setRunningApps(running);
    });

    invoke<Record<string, unknown>>("load_settings")
      .then((settings) => {
        if (!active || !settings) return;
        const val = settings[WEB_ENGINE_KEY];
        if (val === "google" || val === "duckduckgo") {
          try {
            localStorage.setItem(WEB_ENGINE_KEY, val);
          } catch {
            // Ignore storage errors
          }
          setWebEngine(val);
        }
      })
      .catch(() => {});

    return () => {
      active = false;
    };
  }, []);

  // Listen to launcher-opened / launcher-closed lifecycle events from Rust
  useEffect(() => {
    const unlistenOpened = listen("launcher-opened", () => {
      requestIdRef.current += 1;
      setQuery("");
      setFileResults([]);
      setConfirmId(null);
      setSelectedIndex(0);
      setWebEngine(readWebEngine());
      setRecents(readRecents());
      setOpenSeq((seq) => seq + 1);
      fetchRunningApps().then((running) => setRunningApps(running));
      requestAnimationFrame(() => {
        inputRef.current?.focus();
        inputRef.current?.select();
      });
    });

    const unlistenClosed = listen("launcher-closed", () => {
      setConfirmId(null);
    });

    return () => {
      unlistenOpened.then((fn) => fn());
      unlistenClosed.then((fn) => fn());
    };
  }, []);

  // Debounced file search (100ms) guarded by monotonic requestId
  useEffect(() => {
    const trimmed = query.trim();
    const isExplicitWeb = trimmed.startsWith("? ");
    if (trimmed.length < 2 || isExplicitWeb) {
      requestIdRef.current += 1;
      setFileResults([]);
      return;
    }

    const reqId = ++requestIdRef.current;
    const timer = window.setTimeout(() => {
      invoke<FileSearchResult[]>("search_files", { query: trimmed, limit: 5 })
        .then((res) => {
          if (reqId === requestIdRef.current) {
            setFileResults(Array.isArray(res) ? res.slice(0, 5) : []);
          }
        })
        .catch(() => {
          if (reqId === requestIdRef.current) {
            setFileResults([]);
          }
        });
    }, 100);

    return () => window.clearTimeout(timer);
  }, [query]);

  // Merge installed apps with live running window state
  const mergedApps = useMemo(() => {
    const runningByKey = new Map<string, SearchApp>();
    const runningByPath = new Map<string, SearchApp>();
    for (const r of runningApps) {
      runningByKey.set(appKey(r), r);
      const norm = normalizePath(r.path);
      if (norm && !runningByPath.has(norm)) {
        runningByPath.set(norm, r);
      }
    }

    const seenKeys = new Set<string>();
    const merged: SearchApp[] = [];

    for (const app of installedApps) {
      const key = appKey(app);
      if (seenKeys.has(key)) continue;
      seenKeys.add(key);

      const runningMatch =
        runningByKey.get(key) ?? runningByPath.get(normalizePath(app.path));
      if (runningMatch) {
        seenKeys.add(appKey(runningMatch));
        merged.push({
          ...app,
          is_running: true,
          hwnd: runningMatch.hwnd,
          icon: app.icon ?? runningMatch.icon,
        });
      } else {
        merged.push(app);
      }
    }

    for (const r of runningApps) {
      const key = appKey(r);
      if (seenKeys.has(key)) continue;
      seenKeys.add(key);
      merged.push(r);
    }

    return merged;
  }, [installedApps, runningApps]);

  // Build unified LauncherItem list
  const { items, idleRecentCount } = useMemo(() => {
    const trimmed = query.trim();

    if (!trimmed) {
      const byKey = new Map<string, SearchApp>();
      for (const app of mergedApps) {
        byKey.set(appKey(app), app);
      }

      const list: LauncherItem[] = [];
      const seen = new Set<string>();
      const seenPaths = new Set<string>();

      let recentCount = 0;
      for (const key of recents) {
        if (recentCount >= 5) break;
        const app = byKey.get(key);
        if (!app || seen.has(key)) continue;
        const norm = normalizePath(app.path);
        if (norm && seenPaths.has(norm)) continue;
        seen.add(key);
        if (norm) seenPaths.add(norm);
        list.push({ kind: "app", id: `app:${key}`, app });
        recentCount++;
      }

      for (const r of runningApps) {
        if (list.length >= 8) break;
        const key = appKey(r);
        const norm = normalizePath(r.path);
        if (seen.has(key) || (norm && seenPaths.has(norm))) continue;
        seen.add(key);
        if (norm) seenPaths.add(norm);
        const app = byKey.get(key) ?? r;
        list.push({ kind: "app", id: `app:${key}`, app });
      }

      return { items: list, idleRecentCount: recentCount };
    }

    const list: LauncherItem[] = [];
    const isExplicitWeb = trimmed.startsWith("? ");
    const explicitWebQuery = isExplicitWeb ? trimmed.slice(2).trim() : "";

    if (isExplicitWeb && explicitWebQuery.length > 0) {
      list.push({
        kind: "web",
        id: `web:${webEngine}:${explicitWebQuery}`,
        query: explicitWebQuery,
        engine: webEngine,
        url: buildWebSearchUrl(explicitWebQuery, webEngine),
      });
    }

    const calcValue = evaluateExpression(trimmed);
    if (calcValue !== null) {
      list.push({
        kind: "calculator",
        id: "calc",
        expression: trimmed,
        value: calcValue,
      });
    }

    const directUrl = detectDirectUrl(trimmed);
    if (directUrl !== null) {
      list.push({
        kind: "url",
        id: `url:${directUrl}`,
        url: directUrl,
        display: trimmed,
      });
    }

    const recentSet = new Set(recents);
    const scoredApps: { app: SearchApp; score: number }[] = [];
    for (const app of mergedApps) {
      const localized = localizeAppName(app.name);
      const baseScore = Math.max(
        fuzzyScore(app.name, trimmed),
        localized !== app.name ? fuzzyScore(localized, trimmed) : -1
      );
      if (baseScore <= 0) continue;
      const bonus =
        (app.is_running ? 15 : 0) + (recentSet.has(appKey(app)) ? 10 : 0);
      scoredApps.push({ app, score: baseScore + bonus });
    }
    scoredApps.sort(
      (a, b) => b.score - a.score || a.app.name.localeCompare(b.app.name)
    );
    for (const entry of scoredApps.slice(0, 6)) {
      list.push({
        kind: "app",
        id: `app:${appKey(entry.app)}`,
        app: entry.app,
      });
    }

    const matchedCommands = matchQuickCommands(commands, trimmed, t).slice(
      0,
      3
    );
    for (const match of matchedCommands) {
      list.push({
        kind: "command",
        id: `cmd:${match.command.id}`,
        command: match.command,
        label: match.label,
      });
    }

    for (const file of fileResults.slice(0, 5)) {
      list.push({
        kind: "file",
        id: `file:${file.path}`,
        file,
      });
    }

    if (!isExplicitWeb && list.length < 4) {
      list.push({
        kind: "web",
        id: `web:${webEngine}:${trimmed}`,
        query: trimmed,
        engine: webEngine,
        url: buildWebSearchUrl(trimmed, webEngine),
      });
    }

    return { items: list, idleRecentCount: 0 };
  }, [
    query,
    mergedApps,
    runningApps,
    recents,
    commands,
    fileResults,
    webEngine,
    t,
    langVersion,
  ]);

  // Reset selection & confirmation when query changes
  useEffect(() => {
    setSelectedIndex(0);
    setConfirmId(null);
  }, [query]);

  // Clamp selection index if list shrinks
  useEffect(() => {
    setSelectedIndex((prev) => {
      if (items.length === 0) return 0;
      return prev >= items.length ? items.length - 1 : prev;
    });
  }, [items.length]);

  // Clear pending confirmation if selection moves away
  useEffect(() => {
    if (confirmId && items[selectedIndex]?.id !== confirmId) {
      setConfirmId(null);
    }
  }, [selectedIndex, items, confirmId]);

  // Scroll selected item into view
  useEffect(() => {
    const container = listRef.current;
    if (!container) return;
    const selectedEl = container.querySelector<HTMLElement>(
      '[data-selected="true"]'
    );
    selectedEl?.scrollIntoView({ block: "nearest" });
  }, [selectedIndex, items]);

  // Lazy-fetch app icons ONLY for currently rendered app rows
  useEffect(() => {
    for (const item of items) {
      if (item.kind !== "app") continue;
      const key = appKey(item.app);
      if (item.app.icon || requestedIconsRef.current.has(key)) continue;
      requestedIconsRef.current.add(key);

      invoke<string | null>("get_app_icon", {
        path: item.app.path,
        name: item.app.name,
        hwnd: item.app.hwnd ?? null,
      })
        .then((icon) => {
          if (mountedRef.current && icon) {
            setIcons((prev) => ({ ...prev, [key]: icon }));
          }
        })
        .catch(() => {
          // Leave fallback icon
        });
    }
  }, [items]);

  const hideLauncher = useCallback(() => {
    invoke("hide_launcher").catch(() => {});
  }, []);

  const activateItem = useCallback(
    async (
      item: LauncherItem,
      opts: { ctrl: boolean; shift: boolean } = { ctrl: false, shift: false }
    ) => {
      switch (item.kind) {
        case "calculator": {
          await copyToClipboard(item.value);
          hideLauncher();
          return;
        }
        case "url":
        case "web": {
          try {
            await openUrl(item.url);
          } catch {
            // Ignore opener error
          }
          hideLauncher();
          return;
        }
        case "app": {
          recordRecent(item.app);
          setRecents(readRecents());
          if (opts.ctrl || opts.shift) {
            await invoke("launch_new_instance", {
              appPath: item.app.path,
              appName: item.app.name,
              args: item.app.args ?? null,
            }).catch(() => {});
          } else if (item.app.is_running && typeof item.app.hwnd === "number") {
            await invoke("focus_window", { hwnd: item.app.hwnd }).catch(
              () => {}
            );
          } else {
            await invoke("open_app", {
              appName: item.app.path,
              args: item.app.args ?? null,
            }).catch(() => {});
          }
          hideLauncher();
          return;
        }
        case "command": {
          if (item.command.confirmRequired && confirmId !== item.id) {
            setConfirmId(item.id);
            return;
          }
          setConfirmId(null);
          hideLauncher();
          try {
            await item.command.run();
          } catch {
            // Ignore command error
          }
          return;
        }
        case "file": {
          await invoke("open_file_path", {
            path: item.file.path,
            revealInFolder: opts.ctrl,
          }).catch(() => {});
          hideLauncher();
          return;
        }
      }
    },
    [confirmId, hideLauncher]
  );

  useEffect(() => {
    const onWindowKeyDown = (e: globalThis.KeyboardEvent) => {
      if (e.isComposing) return;

      if (e.key === "ArrowDown" || (e.key === "Tab" && !e.shiftKey)) {
        e.preventDefault();
        if (items.length > 0) {
          setSelectedIndex((prev) => (prev + 1) % items.length);
        }
        return;
      }

      if (e.key === "ArrowUp" || (e.key === "Tab" && e.shiftKey)) {
        e.preventDefault();
        if (items.length > 0) {
          setSelectedIndex((prev) => (prev - 1 + items.length) % items.length);
        }
        return;
      }

      if (e.key === "Escape") {
        e.preventDefault();
        if (confirmId !== null) {
          setConfirmId(null);
        } else {
          hideLauncher();
        }
        return;
      }

      if (e.key === "Enter") {
        e.preventDefault();
        const selected = items[selectedIndex];
        if (!selected) return;
        void activateItem(selected, {
          ctrl: e.ctrlKey || e.metaKey,
          shift: e.shiftKey,
        });
      }
    };

    window.addEventListener("keydown", onWindowKeyDown);
    return () => window.removeEventListener("keydown", onWindowKeyDown);
  }, [items, selectedIndex, confirmId, hideLauncher, activateItem]);

  const handleRootMouseDown = useCallback(
    (e: MouseEvent<HTMLDivElement>) => {
      if (e.target === e.currentTarget) {
        hideLauncher();
      }
    },
    [hideLauncher]
  );

  const selectedItem = items[selectedIndex] ?? null;
  const isIdle = query.trim().length === 0;

  return (
    <div
      className="launcher-root"
      onMouseDown={handleRootMouseDown}
    >
      <motion.div
        key={openSeq}
        className="launcher-card"
        role="dialog"
        aria-label="Fern Quick Launcher"
        onMouseDown={(e) => {
          if (e.target !== inputRef.current) e.preventDefault();
        }}
        initial={
          reducedMotion ? false : { opacity: 0, y: -8, scale: 0.985 }
        }
        animate={{ opacity: 1, y: 0, scale: 1 }}
        transition={
          reducedMotion
            ? { duration: 0 }
            : { duration: 0.15, ease: [0.16, 1, 0.3, 1] }
        }
      >
        <div className="launcher-search-row">
          <Search
            className="launcher-search-icon"
            size={18}
            strokeWidth={2.2}
            aria-hidden="true"
          />
          <input
            ref={inputRef}
            className="launcher-input"
            type="text"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder={t("launcher.placeholder")}
            autoComplete="off"
            autoCorrect="off"
            autoCapitalize="off"
            spellCheck={false}
            aria-autocomplete="list"
            aria-controls="launcher-results-list"
            aria-activedescendant={
              selectedItem ? `launcher-option-${selectedIndex}` : undefined
            }
          />
          {query.length > 0 && (
            <button
              type="button"
              className="launcher-clear-btn"
              onClick={() => {
                setQuery("");
                inputRef.current?.focus();
              }}
              aria-label={t("launcher.clearSearch")}
            >
              <X size={14} strokeWidth={2.2} aria-hidden="true" />
            </button>
          )}
          <kbd className="launcher-esc-pill">ESC</kbd>
        </div>

        <div
          id="launcher-results-list"
          ref={listRef}
          className="launcher-list"
          role="listbox"
        >
          {items.length === 0 ? (
            <div className="launcher-empty">
              {isIdle ? t("launcher.emptyIdle") : t("launcher.emptyState")}
            </div>
          ) : (
            items.map((item, idx) => {
              const isSelected = idx === selectedIndex;
              const isConfirming =
                item.kind === "command" && confirmId === item.id;

              const showRecentsHeader =
                isIdle && idleRecentCount > 0 && idx === 0;
              const showRunningHeader =
                isIdle &&
                idx === idleRecentCount &&
                items.length > idleRecentCount;

              return (
                <div key={item.id} className="launcher-row-group">
                  {showRecentsHeader && (
                    <div className="launcher-section-label">
                      {t("launcher.section.recents")}
                    </div>
                  )}
                  {showRunningHeader && (
                    <div className="launcher-section-label">
                      {t("launcher.section.running")}
                    </div>
                  )}

                  {item.kind === "calculator" ? (
                    <div
                      id={`launcher-option-${idx}`}
                      role="option"
                      aria-selected={isSelected}
                      data-selected={isSelected ? "true" : undefined}
                      className={`launcher-row launcher-calc-row${
                        isSelected ? " is-selected" : ""
                      }`}
                      onMouseEnter={() => setSelectedIndex((prev) => (prev === idx ? prev : idx))}
                      onClick={() => void activateItem(item)}
                    >
                      <div className="launcher-icon-box launcher-icon-calc">
                        <Calculator size={16} strokeWidth={2} aria-hidden="true" />
                      </div>
                      <div className="launcher-row-text launcher-calc-text">
                        <span className="launcher-calc-value">
                          = {item.value}
                        </span>
                        <span className="launcher-row-sub">
                          {item.expression}
                        </span>
                      </div>
                      <span className="launcher-badge launcher-badge-accent">
                        <Copy size={11} strokeWidth={2.2} aria-hidden="true" />
                        {t("launcher.badge.copy")}
                      </span>
                    </div>
                  ) : item.kind === "app" ? (
                    (() => {
                      const key = appKey(item.app);
                      const iconSrc = item.app.icon ?? icons[key] ?? null;
                      const displayName = localizeAppName(item.app.name);
                      return (
                        <div
                          id={`launcher-option-${idx}`}
                          role="option"
                          aria-selected={isSelected}
                          data-selected={isSelected ? "true" : undefined}
                          className={`launcher-row${
                            isSelected ? " is-selected" : ""
                          }`}
                          onMouseEnter={() => setSelectedIndex((prev) => (prev === idx ? prev : idx))}
                          onClick={(e) =>
                            void activateItem(item, {
                              ctrl: e.ctrlKey || e.metaKey,
                              shift: e.shiftKey,
                            })
                          }
                        >
                          <div className="launcher-icon-box">
                            {iconSrc ? (
                              <img
                                className="launcher-app-icon"
                                src={iconSrc}
                                alt=""
                                draggable={false}
                              />
                            ) : (
                              <AppWindow
                                size={16}
                                strokeWidth={2}
                                aria-hidden="true"
                              />
                            )}
                          </div>
                          <div className="launcher-row-text">
                            <span className="launcher-row-title">
                              {displayName}
                            </span>
                          </div>
                          <span
                            className={`launcher-badge${
                              item.app.is_running ? " is-running" : ""
                            }`}
                          >
                            {item.app.is_running && (
                              <span
                                className="launcher-running-dot"
                                aria-hidden="true"
                              />
                            )}
                            {item.app.is_running
                              ? t("launcher.badge.running")
                              : t("launcher.badge.app")}
                          </span>
                        </div>
                      );
                    })()
                  ) : item.kind === "command" ? (
                    <div
                      id={`launcher-option-${idx}`}
                      role="option"
                      aria-selected={isSelected}
                      data-selected={isSelected ? "true" : undefined}
                      className={`launcher-row${
                        isSelected ? " is-selected" : ""
                      }${isConfirming ? " is-confirm" : ""}`}
                      onMouseEnter={() => setSelectedIndex((prev) => (prev === idx ? prev : idx))}
                      onClick={() => void activateItem(item)}
                    >
                      <div
                        className={`launcher-icon-box${
                          isConfirming ? " launcher-icon-confirm" : ""
                        }`}
                      >
                        {isConfirming ? (
                          <AlertTriangle
                            size={16}
                            strokeWidth={2}
                            aria-hidden="true"
                          />
                        ) : (
                          <CommandIcon icon={item.command.icon} />
                        )}
                      </div>
                      <div className="launcher-row-text">
                        <span className="launcher-row-title">{item.label}</span>
                        {isConfirming && (
                          <span className="launcher-row-sub launcher-confirm-sub">
                            {t("launcher.confirm.prompt")}
                          </span>
                        )}
                      </div>
                      <span
                        className={`launcher-badge${
                          isConfirming ? " is-confirm" : ""
                        }`}
                      >
                        {isConfirming
                          ? t("launcher.hint.confirm")
                          : t("launcher.badge.command")}
                      </span>
                    </div>
                  ) : item.kind === "file" ? (
                    <div
                      id={`launcher-option-${idx}`}
                      role="option"
                      aria-selected={isSelected}
                      data-selected={isSelected ? "true" : undefined}
                      className={`launcher-row${
                        isSelected ? " is-selected" : ""
                      }`}
                      onMouseEnter={() => setSelectedIndex((prev) => (prev === idx ? prev : idx))}
                      onClick={(e) =>
                        void activateItem(item, {
                          ctrl: e.ctrlKey || e.metaKey,
                          shift: e.shiftKey,
                        })
                      }
                    >
                      <div className="launcher-icon-box">
                        <FileIcon file={item.file} />
                      </div>
                      <div className="launcher-row-text">
                        <span className="launcher-row-title">
                          {item.file.name}
                        </span>
                        <span className="launcher-row-sub">
                          {item.file.parent_dir}
                        </span>
                      </div>
                      <span className="launcher-badge">
                        {item.file.is_dir
                          ? t("launcher.badge.folder")
                          : t("launcher.badge.file")}
                      </span>
                    </div>
                  ) : item.kind === "url" ? (
                    <div
                      id={`launcher-option-${idx}`}
                      role="option"
                      aria-selected={isSelected}
                      data-selected={isSelected ? "true" : undefined}
                      className={`launcher-row${
                        isSelected ? " is-selected" : ""
                      }`}
                      onMouseEnter={() => setSelectedIndex((prev) => (prev === idx ? prev : idx))}
                      onClick={() => void activateItem(item)}
                    >
                      <div className="launcher-icon-box">
                        <ExternalLink
                          size={16}
                          strokeWidth={2}
                          aria-hidden="true"
                        />
                      </div>
                      <div className="launcher-row-text">
                        <span className="launcher-row-title">
                          {item.display}
                        </span>
                        <span className="launcher-row-sub">
                          {t("launcher.url.open")}
                        </span>
                      </div>
                      <span className="launcher-badge">
                        {t("launcher.badge.url")}
                      </span>
                    </div>
                  ) : (
                    <div
                      id={`launcher-option-${idx}`}
                      role="option"
                      aria-selected={isSelected}
                      data-selected={isSelected ? "true" : undefined}
                      className={`launcher-row${
                        isSelected ? " is-selected" : ""
                      }`}
                      onMouseEnter={() => setSelectedIndex((prev) => (prev === idx ? prev : idx))}
                      onClick={() => void activateItem(item)}
                    >
                      <div className="launcher-icon-box">
                        <Globe size={16} strokeWidth={2} aria-hidden="true" />
                      </div>
                      <div className="launcher-row-text">
                        <span className="launcher-row-title">
                          {t("launcher.web.searchWith", {
                            engine: webEngineLabel(item.engine),
                            query: item.query,
                          })}
                        </span>
                      </div>
                      <span className="launcher-badge">
                        {t("launcher.badge.web")}
                      </span>
                    </div>
                  )}
                </div>
              );
            })
          )}
        </div>

        <div className="launcher-footer">
          <div className="launcher-footer-group">
            <span className="launcher-hint">
              <kbd>↑↓</kbd>
              <span>{t("launcher.hint.navigate")}</span>
            </span>
          </div>

          <div className="launcher-footer-group">
            {selectedItem && confirmId === selectedItem.id ? (
              <>
                <span className="launcher-hint">
                  <kbd>↵</kbd>
                  <span>{t("launcher.hint.confirm")}</span>
                </span>
                <span className="launcher-hint">
                  <kbd>Esc</kbd>
                  <span>{t("launcher.hint.cancel")}</span>
                </span>
              </>
            ) : selectedItem?.kind === "calculator" ? (
              <>
                <span className="launcher-hint">
                  <kbd>↵</kbd>
                  <span>{t("launcher.hint.copy")}</span>
                </span>
                <span className="launcher-hint">
                  <kbd>Esc</kbd>
                  <span>{t("launcher.hint.close")}</span>
                </span>
              </>
            ) : selectedItem?.kind === "file" ? (
              <>
                <span className="launcher-hint">
                  <kbd>↵</kbd>
                  <span>{t("launcher.hint.open")}</span>
                </span>
                <span className="launcher-hint">
                  <kbd>Ctrl+↵</kbd>
                  <span>{t("launcher.hint.showInFolder")}</span>
                </span>
                <span className="launcher-hint">
                  <kbd>Esc</kbd>
                  <span>{t("launcher.hint.close")}</span>
                </span>
              </>
            ) : selectedItem?.kind === "app" ? (
              <>
                <span className="launcher-hint">
                  <kbd>↵</kbd>
                  <span>
                    {selectedItem.app.is_running
                      ? t("launcher.hint.focus")
                      : t("launcher.hint.open")}
                  </span>
                </span>
                <span className="launcher-hint">
                  <kbd>Ctrl+↵</kbd>
                  <span>{t("launcher.hint.newWindow")}</span>
                </span>
                <span className="launcher-hint">
                  <kbd>Esc</kbd>
                  <span>{t("launcher.hint.close")}</span>
                </span>
              </>
            ) : selectedItem?.kind === "command" ? (
              <>
                <span className="launcher-hint">
                  <kbd>↵</kbd>
                  <span>{t("launcher.hint.run")}</span>
                </span>
                <span className="launcher-hint">
                  <kbd>Esc</kbd>
                  <span>{t("launcher.hint.close")}</span>
                </span>
              </>
            ) : selectedItem?.kind === "url" || selectedItem?.kind === "web" ? (
              <>
                <span className="launcher-hint">
                  <kbd>↵</kbd>
                  <span>{t("launcher.hint.openBrowser")}</span>
                </span>
                <span className="launcher-hint">
                  <kbd>Esc</kbd>
                  <span>{t("launcher.hint.close")}</span>
                </span>
              </>
            ) : (
              <span className="launcher-hint">
                <kbd>Esc</kbd>
                <span>{t("launcher.hint.close")}</span>
              </span>
            )}
          </div>
        </div>
      </motion.div>
    </div>
  );
}
