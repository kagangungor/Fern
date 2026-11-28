import type React from 'react';
import { useState, useEffect, useMemo, useRef } from 'react';
import { motion } from 'framer-motion';
import { invoke } from '@tauri-apps/api/core';
import { useT, localizeAppName } from '../i18n';
import type { AppInfo } from './types';
import { itemKey, isSameApp } from './identity';

let cachedInstalledApps: AppInfo[] | null = null;

export function AddAppPopup({
  onClose,
  onAdd,
  containerRef,
  scale,
  runningApps,
  pinned,
  bottom,
  isPickingProgramRef,
}: {
  onClose: () => void;
  onAdd: (app: AppInfo) => void;
  containerRef: React.RefObject<HTMLDivElement | null>;
  scale: number;
  runningApps: AppInfo[];
  pinned: AppInfo[];
  bottom: number;
  isPickingProgramRef?: React.MutableRefObject<boolean>;
}) {
  const t = useT();
  const [apps, setApps] = useState<AppInfo[]>(() => cachedInstalledApps ?? []);

  const handleBrowse = async () => {
    if (isPickingProgramRef) isPickingProgramRef.current = true;
    try {
      const picked = await invoke<[string, string] | null>("pick_program");
      if (picked) {
        onAdd({ name: picked[0], path: picked[1], icon: null, is_running: false });
      }
    } catch (e) {
      console.error("Failed to pick program:", e);
    } finally {
      setTimeout(() => {
        if (isPickingProgramRef) isPickingProgramRef.current = false;
      }, 500);
    }
  };
  const [search, setSearch] = useState("");
  const [debouncedSearch, setDebouncedSearch] = useState("");
  const [loading, setLoading] = useState(() => cachedInstalledApps === null);
  const [listIcons, setListIcons] = useState<Record<string, string>>({});
  const listIconsRef = useRef(listIcons);
  listIconsRef.current = listIcons;
  const attemptedPathsRef = useRef<Set<string>>(new Set());
  const popupAliveRef = useRef(true);
  useEffect(() => {
    popupAliveRef.current = true;
    return () => {
      popupAliveRef.current = false;
    };
  }, []);
  const [selectedIndex, setSelectedIndex] = useState(0);
  const selectionSourceRef = useRef<"keyboard" | "mouse">("keyboard");
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const mountedRef = useRef(false);

  useEffect(() => {
    const timer = setTimeout(() => setDebouncedSearch(search), 150);
    return () => clearTimeout(timer);
  }, [search]);

  useEffect(() => {
    invoke('set_window_keyboard_focus', { enabled: true })
      .catch(() => {})
      .finally(() => inputRef.current?.focus());
    return () => {
      invoke('set_window_keyboard_focus', { enabled: false }).catch(() => {});
    };
  }, []);

  // Reset selection when search changes
  useEffect(() => {
    setSelectedIndex(0);
  }, [debouncedSearch]);

  // Running apps that aren't pinned: already in memory, so the default view is instant.
  // Matching is fuzzy, a pinned .lnk and its running .exe are the same app.
  const runningSuggestions = useMemo(
    () => runningApps.filter((a) => a.path !== "start" && !pinned.some((p) => isSameApp(p, a))),
    [runningApps, pinned]
  );

  const sections = useMemo(() => {
    const s = debouncedSearch.trim().toLowerCase();
    // No query: running apps, then everything installed, minus what's pinned.
    // The backend already drops helper entries.
    if (!s) {
      const runningIds = new Set(runningSuggestions.map((a) => itemKey(a)));
      const rest = apps
        .filter(
          (a) =>
            a.path !== "start" &&
            !pinned.some((p) => isSameApp(p, a)) &&
            !runningIds.has(itemKey(a))
        )
        .slice(0, 60);
      const out: { title: string | null; items: AppInfo[] }[] = [];
      if (runningSuggestions.length > 0)
        out.push({ title: t("dockmenu.running"), items: runningSuggestions });
      if (rest.length > 0) out.push({ title: t("dockmenu.allApps"), items: rest });
      return out;
    }
    const starts: AppInfo[] = [];
    const contains: AppInfo[] = [];
    const candidates: AppInfo[] = [...apps];
    for (const r of runningApps) {
      if (r.path === "start") continue;
      if (!candidates.some((a) => isSameApp(a, r))) {
        candidates.push(r);
      }
    }
    for (const a of candidates) {
      // Search covers pinned apps too.
      if (a.path === "start") continue;
      const n = a.name.toLowerCase();
      const loc = localizeAppName(a.name).toLowerCase();
      if (n.startsWith(s) || loc.startsWith(s)) starts.push(a);
      else if (n.includes(s) || loc.includes(s)) contains.push(a);
    }
    const byName = (x: AppInfo, y: AppInfo) => x.name.localeCompare(y.name);
    starts.sort(byName);
    contains.sort(byName);
    return [{ title: null, items: [...starts, ...contains].slice(0, 50) }];
  }, [apps, debouncedSearch, pinned, runningApps, runningSuggestions]);

  // Flat selectable list; section headers are not selectable.
  const flat = useMemo(() => sections.flatMap((s) => s.items), [sections]);
  const flatIndex = useMemo(() => new Map(flat.map((a, i) => [a, i])), [flat]);

  // Keep selection in range as results narrow.
  useEffect(() => {
    setSelectedIndex((i) => Math.min(i, Math.max(0, flat.length - 1)));
  }, [flat.length]);

  // Scroll on keyboard nav only, doing it on mount or mouse hover yanks the list.
  useEffect(() => {
    if (!mountedRef.current) {
      mountedRef.current = true;
      return;
    }
    if (selectionSourceRef.current !== "keyboard") return;
    if (!listRef.current) return;
    const row = listRef.current.querySelector(
      `[data-idx="${selectedIndex}"]`
    ) as HTMLElement | null;
    if (row) row.scrollIntoView({ block: "nearest" });
  }, [selectedIndex]);

  useEffect(() => {
    const load = async () => {
      try {
        const res = await invoke<AppInfo[]>("get_installed_apps");
        const sorted = res.sort((a, b) => a.name.localeCompare(b.name));
        cachedInstalledApps = sorted;
        setApps(sorted);
      } finally {
        setLoading(false);
      }
    };
    load();
  }, []);

  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        onClose();
        return;
      }
      if (e.key === "ArrowDown") {
        e.preventDefault();
        selectionSourceRef.current = "keyboard";
        setSelectedIndex((i) => Math.min(i + 1, flat.length - 1));
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        selectionSourceRef.current = "keyboard";
        setSelectedIndex((i) => Math.max(i - 1, 0));
      } else if (e.key === "Enter") {
        e.preventDefault();
        const target = flat[selectedIndex];
        if (!target) return;
        // Already on the dock: just dismiss, never pin twice.
        if (pinned.some((p) => isSameApp(p, target))) onClose();
        else onAdd(target);
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => {
      window.removeEventListener("keydown", handleKeyDown);
    };
  }, [onClose, flat, selectedIndex, onAdd, pinned]);

  useEffect(() => {
    const handleMouseDown = (e: MouseEvent) => {
      const popup = containerRef.current;
      if (popup && !popup.contains(e.target as Node)) {
        onClose();
      }
    };
    const handleBlur = () => {
      if (!isPickingProgramRef?.current) onClose();
    };
    window.addEventListener("blur", handleBlur);
    document.addEventListener("mousedown", handleMouseDown, true);
    return () => {
      window.removeEventListener("blur", handleBlur);
      document.removeEventListener("mousedown", handleMouseDown, true);
    };
  }, [onClose, containerRef, isPickingProgramRef]);

  // Lazy-load icons as rows enter (or approach) the visible scroll viewport.
  useEffect(() => {
    const root = listRef.current;
    if (!root || loading || flat.length === 0) return;

    const observer = new IntersectionObserver(
      (entries) => {
        const targets: AppInfo[] = [];
        for (const entry of entries) {
          if (!entry.isIntersecting) continue;
          const idxStr = (entry.target as HTMLElement).dataset.idx;
          if (idxStr === undefined) continue;
          const app = flat[Number(idxStr)];
          observer.unobserve(entry.target);
          if (!app || app.icon || listIconsRef.current[app.path] || attemptedPathsRef.current.has(app.path)) {
            continue;
          }
          attemptedPathsRef.current.add(app.path);
          targets.push(app);
        }
        if (targets.length === 0) return;

        Promise.all(
          targets.map(async (app) => {
            try {
              const icon = await invoke<string | null>("get_app_icon", { path: app.path, name: app.name });
              return [app.path, icon] as const;
            } catch (err) {
              console.error(err);
              return [app.path, null] as const;
            }
          })
        ).then((pairs) => {
          if (!popupAliveRef.current) return;
          const batch: Record<string, string> = {};
          for (const [path, icon] of pairs) if (icon) batch[path] = icon;
          if (Object.keys(batch).length > 0) setListIcons((prev) => ({ ...prev, ...batch }));
        });
      },
      { root, rootMargin: "200px 0px" }
    );

    const rows = root.querySelectorAll<HTMLElement>("[data-idx]");
    rows.forEach((row) => observer.observe(row));

    return () => observer.disconnect();
  }, [flat, loading]);

  return (
    <div className="add-popup-anchor" style={{ zoom: scale, bottom }}>
      <motion.div
        ref={containerRef}
        className="add-app-popup"
        style={{ transformOrigin: "bottom center" }}
        initial={{ opacity: 0, scaleY: 0 }}
        animate={{ opacity: 1, scaleY: 1 }}
        exit={{ opacity: 0, scaleY: 0, transition: { duration: 0.16, ease: "easeIn" } }}
        transition={{
          opacity: { duration: 0.15 },
          scaleY: { type: "spring", stiffness: 300, damping: 28, mass: 0.9 }
        }}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="popup-search-row">
          <svg
            className="popup-search-icon"
            width="14"
            height="14"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
            strokeLinecap="round"
            strokeLinejoin="round"
          >
            <circle cx="11" cy="11" r="8" />
            <line x1="21" y1="21" x2="16.65" y2="16.65" />
          </svg>
          <input
            ref={inputRef}
            type="text"
            className="popup-search-input"
            placeholder={t("dockmenu.searchApps")}
            value={search}
            onChange={(e) => setSearch(e.target.value)}
          />
        </div>
        <div className="popup-apps-scroll" ref={listRef}>
          {loading ? (
            <div aria-hidden>
              {Array.from({ length: 8 }, (_, i) => (
                <div className="popup-app-row popup-skeleton-row" key={i}>
                  <div className="popup-app-icon popup-skeleton-box" />
                  <div className="popup-skeleton-line" />
                </div>
              ))}
            </div>
          ) : flat.length > 0 ? (
            sections.map((sec) => (
              <div key={sec.title ?? "all"}>
                {sec.title && <div className="popup-section-label">{sec.title}</div>}
                {sec.items.map((app) => {
                  const gi = flatIndex.get(app) ?? 0;
                  const icon = app.icon || listIcons[app.path];
                  const alreadyPinned = pinned.some((p) => isSameApp(p, app));
                  return (
                    <div
                      key={`${app.path}::${app.name}`}
                      data-idx={gi}
                      className={`popup-app-row${gi === selectedIndex ? " selected" : ""}${alreadyPinned ? " is-pinned" : ""}`}
                      onClick={() => (alreadyPinned ? onClose() : onAdd(app))}
                      onMouseEnter={() => {
                        selectionSourceRef.current = "mouse";
                        setSelectedIndex(gi);
                      }}
                    >
                      <div className="popup-app-icon">
                        {icon ? (
                          <img
                            key={icon}
                            src={icon}
                            alt=""
                            draggable={false}
                            style={{ opacity: 0 }}
                            onLoad={(e) => {
                              e.currentTarget.style.opacity = "1";
                            }}
                          />
                        ) : (
                          <span className="popup-app-initial">{app.name[0]}</span>
                        )}
                      </div>
                      <span className="popup-app-name">{localizeAppName(app.name)}</span>
                      <span className="popup-app-pin">{alreadyPinned ? "✓" : "+"}</span>
                    </div>
                  );
                })}
              </div>
            ))
          ) : (
            <div className="popup-empty">
              {debouncedSearch.trim()
                ? t("dockmenu.noResults")
                : t("dockmenu.emptyHint")}
            </div>
          )}
        </div>
        <div className="popup-browse-row" onClick={handleBrowse}>
          {t("dockmenu.browseProgram")}
        </div>
      </motion.div>
    </div>
  );
}
