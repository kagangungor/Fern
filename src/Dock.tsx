import { Volume2, Volume1, VolumeX } from 'lucide-react';
import { useState, useEffect, useLayoutEffect, useMemo, useRef, memo, useCallback } from 'react';
import { motion, AnimatePresence, Reorder, useIsPresent } from 'framer-motion';
import { invoke } from '@tauri-apps/api/core';
import { emit, listen } from '@tauri-apps/api/event';
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
import './Dock.css';
import './surfaces.css';
import { SurfaceShape } from './components/SurfaceShape';
import { initTheme } from './theme';
import { useSettingsSync } from './hooks/useSettingsSync';
import { useT, localizeAppName } from './i18n';
import { PlayIcon, PauseIcon, SkipBackIcon, SkipForwardIcon } from './icons';
import { createRegionSender, dockBand, fullWindow } from './windowRegion';
import { restingInterval } from './restingInterval';
import { useRestState } from './hooks/useRestState';
import { appAttention, appUnreadCount } from './attention';

// Room around the widest bar for its width animation, and above the tallest for the icons' lift.
const DOCK_REGION_WIDTH_PAD = 120;
const DOCK_REGION_LIFT = 96;

interface AppInfo {
  name: string;
  path: string;
  icon: string | null;
  is_running: boolean;
  is_pinned?: boolean;
  hwnd?: number;
  executable?: string;
  all_hwnds?: [number, string][];
  args?: string;
}

// Browser hosts (Edge/Chrome/Brave/ApplicationFrameHost) run every PWA/UWP window, so the
// title is part of the identity or two PWAs would merge into one item.
const HOST_PROCESSES = ['msedge.exe', 'chrome.exe', 'brave.exe', 'applicationframehost.exe'];
export function isBrowserHost(path: string) {
  const p = path.toLowerCase();
  return HOST_PROCESSES.some(host => p.includes(host));
}

// Stable identity for a dock item.
export function appIdentity(p: string, executable?: string, name?: string) {
  if (!p) return "";
  const normalized = p.toLowerCase().replace(/\\/g, '/');
  // Shell application ids (AUMIDs) and bare names are unique on their own.
  if (!normalized.includes('/')) return normalized;
  if (name && isBrowserHost(normalized)) {
    return `${normalized}:${name.toLowerCase()}`;
  }
  if (executable) return `${normalized}:${executable.toLowerCase()}`;
  return normalized;
}

const itemKey = (app: AppInfo) => appIdentity(app.path, app.executable, app.name);

// Fuzzy identity: a running window and its installed or pinned entry can have different
// paths (exe, .lnk, AUMID), so exact keys miss.
export function isSameApp(a: AppInfo, b: AppInfo): boolean {
  if (!a.path || !b.path) return false;
  if (itemKey(a) === itemKey(b)) return true;
  // Browser hosts run many apps under one exe, only the full identity can match those.
  if (isBrowserHost(a.path) || isBrowserHost(b.path)) return false;
  const aId = isIdentifier(a.path);
  const bId = isIdentifier(b.path);
  // Two opaque shell ids match only exactly (checked above).
  if (aId && bId) return false;
  if (aId !== bId) {
    // One side is an AUMID, the other a file path. AUMIDs contain the product name, so match
    // on overlapping display names plus the exe stem inside the id.
    const fileSide = aId ? b : a;
    const idLower = (aId ? a : b).path.toLowerCase();
    const na = a.name.toLowerCase();
    const nb = b.name.toLowerCase();
    if (na !== nb && !na.includes(nb) && !nb.includes(na)) return false;
    const stem = (fileSide.executable?.toLowerCase() || fileOf(fileSide.path)).replace(
      /\.(exe|lnk)$/,
      ""
    );
    return stem.length >= 3 && idLower.includes(stem);
  }
  if (appIdentity(a.path) === appIdentity(b.path)) return true;
  const normExe = (s: string) => (s.endsWith(".exe") ? s : `${s}.exe`);
  const aExe = a.executable?.toLowerCase() || fileOf(a.path);
  const bExe = b.executable?.toLowerCase() || fileOf(b.path);
  if (!aExe || !bExe) return false;
  return normExe(aExe) === normExe(bExe);
}

const fileOf = (p: string) =>
  (p.split("/").pop()?.split("\\").pop()?.toLowerCase() || "").replace(/\.lnk$/, "");

// AUMIDs identify one app. Two PWAs in the same browser must never match on the shared exe name.
const isIdentifier = (p: string) => !p.includes('/') && !p.includes('\\');

const isFernWindow = (app: { path: string }) => /(^|[\\/])fern\.exe$/i.test(app.path);

function parseDrives(val: string | null): string[] {
  try {
    const parsed = JSON.parse(val ?? "[]");
    return Array.isArray(parsed) ? parsed.filter((d): d is string => typeof d === "string") : [];
  } catch {
    return [];
  }
}

const solidIconCache = new Map<string, boolean>();

function hasOpaqueCorners(img: HTMLImageElement): boolean {
  try {
    const canvas = document.createElement('canvas');
    canvas.width = canvas.height = 16;
    const ctx = canvas.getContext('2d');
    if (!ctx) return false;
    ctx.drawImage(img, 0, 0, 16, 16);
    const data = ctx.getImageData(0, 0, 16, 16).data;
    return [[0, 0], [15, 0], [0, 15], [15, 15]].every(([x, y]) => data[(y * 16 + x) * 4 + 3] > 235);
  } catch {
    return false;
  }
}

function DockAppIcon({ src, alt, fern }: { src: string | undefined; alt: string; fern: boolean }) {
  const [solid, setSolid] = useState(() => (src ? solidIconCache.get(src) ?? false : false));
  return (
    <img
      src={src}
      alt={alt}
      className={fern ? 'fern-icon-img' : solid ? 'solid-icon' : ''}
      draggable={false}
      onLoad={(e) => {
        if (fern || !src) return;
        if (!solidIconCache.has(src)) solidIconCache.set(src, hasOpaqueCorners(e.currentTarget));
        setSolid(solidIconCache.get(src) ?? false);
      }}
    />
  );
}

// Start button icon: a public asset, or the data URI of an uploaded icon ("custom:" prefix).
function resolveStartIcon(startIcon: string): string {
  if (startIcon === "windows") return "/windows.png";
  if (startIcon.startsWith("custom:")) return startIcon.slice("custom:".length);
  return "/fern.png";
}

// Stable module-level constants so object references never change between renders,
// preventing Framer Motion from re-triggering animations on every re-render.
const DOCK_SPRING = { type: 'spring' as const, stiffness: 350, damping: 30 };
const ITEM_ENTRY_TRANSITION = {
  opacity: { duration: 0.15 },
  scale: DOCK_SPRING
};
const ITEM_INITIAL = { opacity: 0, scale: 0.8 };
const ITEM_ANIMATE = { opacity: 1, scale: 1 };
const ITEM_EXIT = { opacity: 0, scale: 0.8, transition: { duration: 0.15 } };

function SearchIcon() {
  return (
    <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
      <circle cx="11" cy="11" r="7" />
      <line x1="21" y1="21" x2="16.65" y2="16.65" />
    </svg>
  );
}

function CalendarIcon() {
  return (
    <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round">
      <rect x="3" y="4.5" width="18" height="16" rx="3" />
      <line x1="3" y1="10" x2="21" y2="10" />
      <line x1="8" y1="2.5" x2="8" y2="6.5" />
      <line x1="16" y1="2.5" x2="16" y2="6.5" />
    </svg>
  );
}

function ChevronUpIcon() {
  return (
    <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.6" strokeLinecap="round" strokeLinejoin="round">
      <polyline points="6 15 12 9 18 15" />
    </svg>
  );
}

function TaskViewIcon() {
  return (
    <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round">
      <rect x="3" y="5" width="14" height="11" rx="2.5" />
      <path d="M7 19.5h12a2.5 2.5 0 0 0 2.5-2.5V9.5" />
    </svg>
  );
}

function RecycleIcon({ isEmpty = true }: { isEmpty?: boolean }) {
  return (
    <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M4 7h16" />
      <path d="M9 7V4.5h6V7" />
      <path d="M6 7l1 13h10l1-13" />
      {isEmpty ? (
        <>
          <line x1="10" y1="11" x2="10" y2="16" />
          <line x1="14" y1="11" x2="14" y2="16" />
        </>
      ) : (
        <>
          <path d="M9 11l2 4 2-3 2 4" strokeWidth="1.8" />
          <line x1="8.5" y1="9" x2="15.5" y2="9" strokeWidth="1.8" />
        </>
      )}
    </svg>
  );
}

function DriveIcon({ letter }: { letter: string }) {
  return (
    <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round">
      <rect x="2.5" y="7" width="19" height="10" rx="3" />
      <text x="12" y="14.6" textAnchor="middle" fontSize="7.5" fontWeight="700" fill="currentColor" stroke="none">{letter}</text>
    </svg>
  );
}

// One dot per open window, capped at three.
interface AppMediaSession {
  app_id: string;
  title: string;
  artist: string;
  is_playing: boolean;
}

interface RecycleBinStatus {
  is_empty: boolean;
  item_count: number;
  size_bytes: number;
}

interface DockDrive {
  letter: string;
  label: string;
  removable: boolean;
}

function sessionForApp(app: AppInfo, sessions: AppMediaSession[]): AppMediaSession | undefined {
  const exe = (app.executable || app.path.split(/[\\/]/).pop() || "").toLowerCase().replace(/\.exe$/, "");
  if (!exe) return undefined;
  return sessions.find((s) => s.app_id.toLowerCase().replace(/\.exe$/, "").includes(exe));
}

const FERN_MENU_IDS = ["calendar", "settings", "taskmgr", "restart", "clearicons", "quit"];

function parseMenuItems(val: unknown): string[] {
  try {
    const parsed = typeof val === "string" ? JSON.parse(val) : val;
    return Array.isArray(parsed) ? parsed.filter((id): id is string => FERN_MENU_IDS.includes(id)) : FERN_MENU_IDS;
  } catch {
    return FERN_MENU_IDS;
  }
}

function WindowDots({ count, mode }: { count: number; mode: string }) {
  if (mode === "dots") {
    return (
      <div className="window-dots dots">
        {Array.from({ length: Math.min(Math.max(count, 1), 4) }, (_, i) => (
          <div key={i} className="window-dot" />
        ))}
      </div>
    );
  }
  if (mode === "bar") {
    return (
      <div className="window-dots bar">
        <div className="window-bar" />
      </div>
    );
  }
  return (
    <div className="window-dots dot">
      <div className="active-indicator" />
    </div>
  );
}

function AttentionBadge({ count, isFlashing }: { count: number | null; isFlashing: boolean }) {
  if (count === null && !isFlashing) return null;
  const countStr = count !== null ? (count > 99 ? "99+" : String(count)) : null;
  return (
    <div className={`attention-badge ${countStr ? "has-count" : ""} ${isFlashing ? "flashing" : ""}`}>
      {countStr}
    </div>
  );
}

let liveThumbnailGeneration = 0;

interface DockPreviewTooltipProps {
  app: AppInfo;
  previews: { hwnd: number; title: string; image: string }[];
  onClosePreview: (e: React.MouseEvent, hwnd: number) => void;
  renderMedia: (app: AppInfo) => React.ReactNode;
  renderPreviewMedia?: (app: AppInfo) => React.ReactNode;
  onMouseEnter: () => void;
  onMouseLeave: () => void;
}

function DockPreviewTooltip({
  app,
  previews,
  onClosePreview,
  renderMedia,
  renderPreviewMedia,
  onMouseEnter,
  onMouseLeave,
}: DockPreviewTooltipProps) {
  const t = useT();
  const isPresent = useIsPresent();
  const [liveHwnds, setLiveHwnds] = useState<Set<number>>(() => new Set());

  useEffect(() => {
    if (isPresent) {
      invoke('set_live_thumbnails_opacity', { opacity: 255 }).catch(() => {});
    } else {
      invoke('set_live_thumbnails_opacity', { opacity: 0 }).catch(() => {});
    }
  }, [isPresent]);
  const surfaceRefs = useRef<Map<number, HTMLDivElement>>(new Map());

  useLayoutEffect(() => {
    const generation = ++liveThumbnailGeneration;
    let cancelled = false;
    let rafId: number | null = null;
    let t1: any = null;
    let t2: any = null;

    const syncThumbnails = async () => {
      if (cancelled) return;
      const dpr = window.devicePixelRatio || 1;
      const targets: { hwnd: number; x: number; y: number; width: number; height: number }[] = [];

      for (const p of previews) {
        const el = surfaceRefs.current.get(p.hwnd);
        if (el) {
          const r = el.getBoundingClientRect();
          if (r.width > 0 && r.height > 0) {
            targets.push({
              hwnd: p.hwnd,
              x: Math.round(r.left * dpr),
              y: Math.round(r.top * dpr),
              width: Math.round(r.width * dpr),
              height: Math.round(r.height * dpr),
            });
          }
        }
      }

      if (targets.length > 0) {
        try {
          const active = await invoke<number[]>('set_live_thumbnails', { targets, clientAreaOnly: false });
          if (!cancelled && active) {
            setLiveHwnds(new Set(active));
          }
        } catch {}
      }
    };

    rafId = requestAnimationFrame(() => {
      syncThumbnails();
      t1 = setTimeout(syncThumbnails, 80);
      t2 = setTimeout(syncThumbnails, 220);
    });

    return () => {
      cancelled = true;
      if (rafId) cancelAnimationFrame(rafId);
      if (t1) clearTimeout(t1);
      if (t2) clearTimeout(t2);
      if (generation === liveThumbnailGeneration) invoke('clear_live_thumbnails').catch(() => {});
    };
  }, [previews]);

  return (
    <motion.div
      className={`preview-tooltip ${previews.length > 1 ? 'multi' : ''}`}
      initial={{ opacity: 0, y: 10, scale: 0.95 }}
      animate={{ opacity: 1, y: 0, scale: 1 }}
      exit={{ opacity: 0, scale: 0.95 }}
      transition={{ duration: 0.15 }}
      onMouseEnter={onMouseEnter}
      onMouseLeave={onMouseLeave}
    >
      {renderPreviewMedia && renderPreviewMedia(app)}
      <div className="preview-items">
        {previews.map((prev, idx) => {
          const isLive = liveHwnds.has(prev.hwnd);
          return (
            <div
              key={prev.hwnd}
              className="preview-item"
              onClick={() => invoke('focus_window', { hwnd: prev.hwnd })}
              onDragOver={(e) => {
                e.preventDefault();
                e.stopPropagation();
                e.dataTransfer.dropEffect = 'copy';
                invoke('bring_hwnd_to_front', { hwnd: prev.hwnd }).catch(() => {});
              }}
            >
              <div className="preview-header">
                <div className="preview-label">{localizeAppName(prev.title || app.name)}</div>
                <button
                  className="preview-close-btn"
                  onClick={(e) => {
                    e.stopPropagation();
                    onClosePreview(e, prev.hwnd);
                  }}
                  data-tooltip={t("dockmenu.closeWindow")}
                >
                  <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="3" strokeLinecap="round">
                    <line x1="18" y1="6" x2="6" y2="18"></line>
                    <line x1="6" y1="6" x2="18" y2="18"></line>
                  </svg>
                </button>
              </div>
              <div
                className={`preview-surface${isLive ? ' has-live' : ''}`}
                ref={(el) => {
                  if (el) surfaceRefs.current.set(prev.hwnd, el);
                  else surfaceRefs.current.delete(prev.hwnd);
                }}
              >
                <div className="preview-live-mount" />
                {prev.image ? (
                  <img
                    src={prev.image}
                    alt={`Preview ${idx}`}
                    className={`preview-thumb-img${isLive ? ' is-live' : ''}`}
                  />
                ) : (
                  <div className="preview-item-icon-fallback">{renderMedia(app)}</div>
                )}
              </div>
            </div>
          );
        })}
      </div>
    </motion.div>
  );
}

const tourDockAction = () => { emit('tour-dock-action').catch(() => {}); };

const Dock = memo(function Dock() {
  const rest = useRestState();
  const rectPollMs = restingInterval(500, rest);
  const windowsPollMs = restingInterval(10000, rest);
  const t = useT();
  useEffect(() => {
    return initTheme({ backdrop: true });
  }, []);

  const [pinnedApps, setPinnedApps] = useState<AppInfo[]>([]);
  const [activeApps, setActiveApps] = useState<AppInfo[]>([]);
  const iconsRef = useRef<Record<string, string>>({});
  const [, setIconsTick] = useState(0); 
  const [dockMode, setDockMode] = useState(() => {
    const raw = localStorage.getItem("fern-dock-mode") || "smart";
    if (raw === "auto-hide") return "smart";
    return raw;
  });
  const [dockPreviewEnabled, setDockPreviewEnabled] = useState(() => localStorage.getItem("fern-dock-preview-enabled") !== "false");
  const [previewDelay, setPreviewDelay] = useState(() => { const v = localStorage.getItem("fern-dock-preview-delay"); return v === null ? 300 : Number(v) || 0; });
  const [dockSearchEnabled, setDockSearchEnabled] = useState(() => localStorage.getItem("fern-dock-search-enabled") !== "false");
  const [dockCalendarEnabled, setDockCalendarEnabled] = useState(() => localStorage.getItem("fern-dock-calendar-enabled") !== "false");
  const [dockTray, setDockTray] = useState(() => localStorage.getItem("fern-dock-tray-enabled") !== "false");
  const [dockTaskView, setDockTaskView] = useState(() => localStorage.getItem("fern-dock-taskview-enabled") === "true");
  const [dockShowDesktop, setDockShowDesktop] = useState(() => localStorage.getItem("fern-dock-show-desktop-corner") === "true");
  const [dockRecycle, setDockRecycle] = useState(() => localStorage.getItem("fern-dock-recycle-enabled") === "true");
  const [dockDrives, setDockDrives] = useState<string[]>(() => parseDrives(localStorage.getItem("fern-dock-drives")));
  const [dockIconOnly, setDockIconOnly] = useState(() => localStorage.getItem("fern-dock-icon-only") === "true");
  const [hiddenApps, setHiddenApps] = useState<string[]>(() => {
    try {
      const raw = localStorage.getItem("fern-dock-hidden-apps");
      return raw ? JSON.parse(raw) : [];
    } catch {
      return [];
    }
  });

  useEffect(() => {
    const handler = () => {
      try {
        const raw = localStorage.getItem("fern-dock-hidden-apps");
        setHiddenApps(raw ? JSON.parse(raw) : []);
      } catch {}
    };
    window.addEventListener("storage", handler);
    const unlisten = listen("refresh-hidden-apps", handler);
    return () => {
      window.removeEventListener("storage", handler);
      unlisten.then((u) => u());
    };
  }, []);

  const handleHideApp = (app: AppInfo) => {
    const identifier = app.executable || app.path || app.name;
    if (!identifier) return;
    const updated = Array.from(new Set([...hiddenApps, identifier]));
    setHiddenApps(updated);
    localStorage.setItem("fern-dock-hidden-apps", JSON.stringify(updated));
    invoke("save_setting", { key: "fern-dock-hidden-apps", value: JSON.stringify(updated) }).catch(() => {});
    emit("refresh-hidden-apps").catch(() => {});
  };
    const [dockAppVolumeScroll, setDockAppVolumeScroll] = useState(() => localStorage.getItem("fern-dock-app-volume-scroll") !== "false");
  const [volumeHuds, setVolumeHuds] = useState<{ [appKey: string]: { currentVol: number; targetVol: number; muted: boolean; visible: boolean } }>({});
  const [activeVolumeKey, setActiveVolumeKey] = useState<string | null>(null);
  const volumeAnimRefs = useRef<Map<string, {
    rafId: number;
    currentVol: number;
    targetVol: number;
    muted: boolean;
    lastNativeSend: number;
    hideTimer: any;
  }>>(new Map());

  const handleAppWheel = useCallback((e: React.WheelEvent, app: AppInfo) => {
    if (!dockAppVolumeScroll || !app.is_running || app.path === 'start') return;

    e.preventDefault();
    e.stopPropagation();

    const key = itemKey(app);
    const targetExe = app.executable || app.path || app.name;

    // Immediately dismiss any open preview and live thumbnails
    if (previewTimerRef.current) clearTimeout(previewTimerRef.current);
    isPreviewHoveredRef.current = false;
    setPreviewData(null);
    invoke('clear_live_thumbnails').catch(() => {});
    setActiveVolumeKey(key);

    // Immediately close all other apps' volume HUDs and cancel their anim loops
    for (const [otherKey, otherAnim] of volumeAnimRefs.current.entries()) {
      if (otherKey !== key) {
        if (otherAnim.rafId) {
          cancelAnimationFrame(otherAnim.rafId);
          otherAnim.rafId = 0;
        }
        if (otherAnim.hideTimer) {
          clearTimeout(otherAnim.hideTimer);
          otherAnim.hideTimer = null;
        }
      }
    }

    const delta = -Math.sign(e.deltaY) * Math.min(Math.max(Math.abs(e.deltaY) * 0.0004, 0.015), 0.08);

    let anim = volumeAnimRefs.current.get(key);
    if (!anim) {
      const initialVol = volumeHuds[key]?.currentVol ?? 1.0;
      anim = {
        rafId: 0,
        currentVol: initialVol,
        targetVol: initialVol,
        muted: false,
        lastNativeSend: 0,
        hideTimer: null,
      };
      volumeAnimRefs.current.set(key, anim);

      invoke<{ volume: number; muted: boolean } | null>('get_target_app_volume', { target: targetExe })
        .then((res) => {
          if (res && anim) {
            anim.currentVol = res.volume;
            anim.targetVol = res.volume;
            anim.muted = res.muted;
          }
        })
        .catch(() => {});
    }

    anim.targetVol = Math.max(0, Math.min(1, anim.targetVol + delta));
    if (anim.targetVol > 0) anim.muted = false;
    else if (anim.targetVol === 0) anim.muted = true;

    if (anim.hideTimer) clearTimeout(anim.hideTimer);

    if (!anim.rafId) {
      const loop = () => {
        const a = volumeAnimRefs.current.get(key);
        if (!a) return;

        const diff = a.targetVol - a.currentVol;
        if (Math.abs(diff) > 0.001) {
          a.currentVol += diff * 0.22;
          const now = performance.now();
          if (now - a.lastNativeSend >= 20) {
            a.lastNativeSend = now;
            invoke('set_target_app_volume', { target: targetExe, volume: a.currentVol }).catch(() => {});
          }
          setVolumeHuds((prev) => {
            const next: typeof prev = {};
            next[key] = { currentVol: a.currentVol, targetVol: a.targetVol, muted: a.muted, visible: true };
            for (const k of Object.keys(prev)) {
              if (k !== key && prev[k]?.visible) {
                next[k] = { ...prev[k], visible: false };
              }
            }
            return next;
          });
          a.rafId = requestAnimationFrame(loop);
        } else {
          a.currentVol = a.targetVol;
          a.rafId = 0;
          invoke('set_target_app_volume', { target: targetExe, volume: a.currentVol }).catch(() => {});
          setVolumeHuds((prev) => {
            const next: typeof prev = {};
            next[key] = { currentVol: a.currentVol, targetVol: a.targetVol, muted: a.muted, visible: true };
            for (const k of Object.keys(prev)) {
              if (k !== key && prev[k]?.visible) {
                next[k] = { ...prev[k], visible: false };
              }
            }
            return next;
          });
        }
      };
      anim.rafId = requestAnimationFrame(loop);
    }

    anim.hideTimer = setTimeout(() => {
      setActiveVolumeKey((current) => (current === key ? null : current));
      setVolumeHuds((prev) => {
        if (!prev[key]) return prev;
        return { ...prev, [key]: { ...prev[key], visible: false } };
      });
    }, 1200);
  }, [dockAppVolumeScroll, volumeHuds]);

  const [dockAdaptive, setDockAdaptive] = useState(() => localStorage.getItem("fern-dock-adaptive") === "true");
  const [dockAttentionEnabled, setDockAttentionEnabled] = useState(() => localStorage.getItem("fern-dock-attention-enabled") !== "false");
  const [dockHideDelay, setDockHideDelay] = useState(() => parseInt(localStorage.getItem("fern-dock-hide-delay") || "800", 10));
  const [attentionHwnds, setAttentionHwnds] = useState<number[]>([]);
  const [startIcon, setStartIcon] = useState(() => localStorage.getItem("fern-start-icon") || "default");
  const [isMaximized, setIsMaximized] = useState(false);
  const [dockMixedReorder, setDockMixedReorder] = useState(() => localStorage.getItem("fern-dock-mixed-reorder") === "true");
  const [windowIndicator, setWindowIndicator] = useState(() => localStorage.getItem("fern-dock-indicator") ?? "dot");
  const [showEndTask, setShowEndTask] = useState(() => localStorage.getItem("fern-dock-show-end-task") !== "false");
  const [menuItems, setMenuItems] = useState<string[]>(() => parseMenuItems(localStorage.getItem("fern-dock-menu-items")));
  const isPickingProgramRef = useRef(false);
  const [mixedOrder, setMixedOrder] = useState<string[]>([]);
  const [previewData, setPreviewData] = useState<{ id: string, previews: { hwnd: number, title: string, image: string }[] } | null>(null);
  const [isDockHovered, setIsDockHovered] = useState(false);
  const [isEdgeHovered, setIsEdgeHovered] = useState(false);
  const [isOverlapped, setIsOverlapped] = useState(false);
  const [isFullscreen, setIsFullscreen] = useState(false);
  const [showAddPopup, setShowAddPopup] = useState(false);
  const [contextMenu, setContextMenu] = useState<{
    x: number;
    y: number;
    app: AppInfo | null;
    extraKind?: 'recycle' | 'drive';
    driveLetter?: string;
    isRemovable?: boolean;
  } | null>(null);
  const [recycleStatus, setRecycleStatus] = useState<RecycleBinStatus | null>(null);
  const [driveDetails, setDriveDetails] = useState<Record<string, DockDrive>>({});

  const refreshRecycleStatus = useCallback(() => {
    if (!dockRecycle) return;
    invoke<RecycleBinStatus>('get_recycle_bin_status')
      .then(setRecycleStatus)
      .catch(() => {});
  }, [dockRecycle]);

  const refreshDriveDetails = useCallback(() => {
    invoke<DockDrive[]>('get_drives')
      .then((drives) => {
        const map: Record<string, DockDrive> = {};
        drives.forEach((d) => { map[d.letter] = d; });
        setDriveDetails(map);
      })
      .catch(() => {});
  }, []);

  useEffect(() => {
    if (dockRecycle) {
      refreshRecycleStatus();
      const interval = setInterval(refreshRecycleStatus, 8000);
      return () => clearInterval(interval);
    }
  }, [dockRecycle, refreshRecycleStatus]);

  useEffect(() => {
    refreshDriveDetails();
  }, [refreshDriveDetails]);
  const [contextMenuHeight, setContextMenuHeight] = useState(0);
  const [contextMenuWidth, setContextMenuWidth] = useState(0);
  const [activeSubmenu, setActiveSubmenu] = useState<string | null>(null);
  const [activeOrder, setActiveOrder] = useState<string[]>([]);
  const [isDragging, setIsDragging] = useState(false);
  const [hoveredApp, setHoveredApp] = useState<string | null>(null);
  const [dragHoveringApp, setDragHoveringApp] = useState<string | null>(null);
  const dragHoverTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const dragHoverAppKeyRef = useRef<string | null>(null);
  const [trayOpen, setTrayOpen] = useState(false);
  const [clickedShortcut, setClickedShortcut] = useState<string | null>(null);
  const [pressedApp, setPressedApp] = useState<string | null>(null);
  const [isReady, setIsReady] = useState(false);
  const [isImpacted, setIsImpacted] = useState(false);
  const [isExpanded, setIsExpanded] = useState(false);
  const [startupAnimating, setStartupAnimating] = useState(false);
  const [customIcons, setCustomIcons] = useState<Record<string, string>>({});
  const [toast, setToast] = useState<string | null>(null);
  const iconPickerTargetRef = useRef<string | null>(null);
  const toastTimerRef = useRef<any>(null);
  const dockRef = useRef<HTMLDivElement>(null);
  const [popupBottom, setPopupBottom] = useState(56);
  const [scale, setScale] = useState(() => parseFloat(localStorage.getItem("fern-scale") || "1.0"));
  const [viewportWidth, setViewportWidth] = useState(() => window.innerWidth);
  const pinnedItemsRef = useRef<AppInfo[]>([]);
  const handleAppClickRef = useRef<(app: AppInfo) => void>(() => {});

  useEffect(() => {
    const onResize = () => setViewportWidth(window.innerWidth);
    window.addEventListener('resize', onResize);
    return () => window.removeEventListener('resize', onResize);
  }, []);



  // Measure the dock pill when the popup opens. The card tucks ~14px behind it,
  // so the seam stays fused even if the measurement is slightly off.
  useEffect(() => {
    if (showAddPopup) {
      const h = dockRef.current?.getBoundingClientRect().height ?? 0;
      if (h > 0) setPopupBottom((h / (scale || 1)) + 12);
    }
  }, [showAddPopup, scale]);

  const isCurrentlyHovered = isDockHovered || isEdgeHovered;
  const [interactionState, setInteractionState] = useState<'active' | 'grace' | 'none'>('none');
  const isAnyInteraction = isCurrentlyHovered || !!contextMenu || showAddPopup;
  
  const previewTimerRef = useRef<any>(null);
  const isPreviewHoveredRef = useRef(false);
  const [mediaSessions, setMediaSessions] = useState<AppMediaSession[]>([]);
  const hoveredAppRef = useRef<string | null>(null);

  useEffect(() => {
    if (isAnyInteraction) {
      setInteractionState('active');
    } else if (interactionState !== 'none') {
      setInteractionState('grace');
      const timer = setTimeout(() => setInteractionState('none'), dockHideDelay);
      return () => clearTimeout(timer);
    }
  }, [isAnyInteraction, dockHideDelay]);

  const isHidden = isFullscreen ||
    (!startupAnimating && (
      (dockMode === 'smart' && isOverlapped && interactionState === 'none') ||
      (dockMode === 'peek' && interactionState === 'none')
    ));

  // Adaptive mode (fixed dock): stretch to a full-width taskbar while a window is maximized.
  const isAdaptive = dockAdaptive && dockMode === 'fixed' && isMaximized && isExpanded && !isHidden;
  // Nearly full width, 24px margin per side. Width is pre-scale, so (viewport - 48*scale) / scale.
  const adaptiveWidth = (viewportWidth - 48 * scale) / scale;

  useEffect(() => {
    let cleared = false;

    const checkVisibility = async (): Promise<boolean> => {
      try {
        const { getCurrentWebviewWindow } = await import('@tauri-apps/api/webviewWindow');
        const visible = await getCurrentWebviewWindow().isVisible();
        if (visible) {
          setStartupAnimating(true);
          setIsReady(true);
          setTimeout(() => setIsImpacted(true), 280);
          setTimeout(() => setIsExpanded(true), 350);
          setTimeout(() => setStartupAnimating(false), 1500);
          return true;
        }
      } catch (_) {}
      return false;
    };

    // Keep polling until visible — no time cap, since the dock can be
    // enabled at runtime from settings after any delay.
    const interval = setInterval(async () => {
      if (cleared) return;
      if (await checkVisibility()) {
        clearInterval(interval);
        cleared = true;
      }
    }, 200);

    // Also attempt immediately
    checkVisibility().then(ok => { if (ok) { clearInterval(interval); cleared = true; } });

    return () => { clearInterval(interval); cleared = true; };
  }, []);

  // The widest and tallest the expanded bar has been, set by the region code further down.
  const dockBarExtentRef = useRef({ width: 0, height: 0 });
  const updateDockRegionRef = useRef<() => void>(() => {});

  useEffect(() => {
    let lastSent = "";
    const updateRect = () => {
      if (dockRef.current) {
        const rect = dockRef.current.getBoundingClientRect();
        const hasPreview = !!previewData;
        const next = {
          x: Math.round(rect.x) - (hasPreview ? 500 : 0),
          y: Math.round(rect.y) - (hasPreview ? 320 : 0),
          width: Math.round(rect.width) + (hasPreview ? 1000 : 0),
          height: Math.round(rect.height) + (hasPreview ? 320 : 0)
        };
        const key = `${next.x},${next.y},${next.width},${next.height}`;
        if (key === lastSent) return;
        lastSent = key;
        invoke('update_dock_rect', { rect: next }).catch(() => { lastSent = ""; });
      }
      // The bar's size animates without re-rendering, this is what sees it grow.
      updateDockRegionRef.current();
    };

    updateRect();
    window.addEventListener('resize', updateRect);
    const observer = new ResizeObserver(updateRect);
    if (dockRef.current) observer.observe(dockRef.current);

    let rafId: number;
    const chaseStart = performance.now();
    const chase = () => {
      updateRect();
      if (performance.now() - chaseStart < 700) {
        rafId = requestAnimationFrame(chase);
      }
    };
    rafId = requestAnimationFrame(chase);

    const poll = rectPollMs === null ? undefined : setInterval(updateRect, rectPollMs);

    return () => {
      window.removeEventListener('resize', updateRect);
      observer.disconnect();
      clearInterval(poll);
      cancelAnimationFrame(rafId);
    };
  }, [pinnedApps, activeApps, isHidden, previewData, scale, rectPollMs]);

  const refreshDockMode = () => {
    invoke<string>('get_dock_mode_for_window').then(setDockMode).catch(() => {});
  };

  useEffect(() => {
    const init = async () => {
      const settings: any = await invoke('load_settings').catch(() => ({}));
      const getVal = (key: string, fallback: string | null = null) => {
        const val = settings[key];
        if (val !== undefined && val !== null) return String(val);
        const local = localStorage.getItem(key);
        if (local !== null) return local;
        return fallback;
      };

      refreshDockMode();

      const preview = getVal("fern-dock-preview-enabled", "true");
      setDockPreviewEnabled(preview === "true");

      const searchEnabled = getVal("fern-dock-search-enabled", "true");
      setDockSearchEnabled(searchEnabled === "true");

      const calendarEnabled = getVal("fern-dock-calendar-enabled", "true");
      setDockCalendarEnabled(calendarEnabled === "true");

      setDockTray(getVal("fern-dock-tray-enabled", "true") !== "false");
      setDockTaskView(getVal("fern-dock-taskview-enabled", "false") === "true");
      setDockShowDesktop(getVal("fern-dock-show-desktop-corner", "false") === "true");
      setDockRecycle(getVal("fern-dock-recycle-enabled", "false") === "true");
      setDockDrives(parseDrives(getVal("fern-dock-drives", "[]")));

      const iconOnly = getVal("fern-dock-icon-only", "false");
      setDockIconOnly(iconOnly === "true");

      const adaptive = getVal("fern-dock-adaptive", "false");
      setDockAdaptive(adaptive === "true");

      const startIconVal = getVal("fern-start-icon", "default") || "default";
      setStartIcon(startIconVal);

      const mixedReorder = getVal("fern-dock-mixed-reorder", "false");
      setDockMixedReorder(mixedReorder === "true");

      const indicator = getVal("fern-dock-indicator", "dot");
      setWindowIndicator(indicator || "dot");

      const endTaskVal = getVal("fern-dock-show-end-task", "true");
      setShowEndTask(endTaskVal !== "false");

      const menuItemsVal = getVal("fern-dock-menu-items");
      setMenuItems(parseMenuItems(menuItemsVal));

      const scaleVal = getVal("fern-scale");
      if (scaleVal !== null) setScale(parseFloat(scaleVal));

      const pinned = await invoke<AppInfo[]>('load_pinned_apps');
      setPinnedApps(pinned.map(a => ({ ...a, is_pinned: true })));
      pinned.forEach(app => fetchIcon(app.path));

      // Warm the installed apps cache so the add-app popup opens with data.
      invoke<AppInfo[]>('get_installed_apps').catch(() => {});

      // Load custom icons
      try {
        const icons = await invoke<Record<string, string>>('get_custom_icons');
        setCustomIcons(icons);
      } catch (_) {}
    };
    init();

    // Sent to one dock by label, a global listen() would get them for every dock.
    const thisWindow = getCurrentWebviewWindow();

    const unlistenOverlap = thisWindow.listen<boolean>("dock-overlap", (event) => {
      setIsOverlapped(event.payload);
    });

    const unlistenEdgeHover = thisWindow.listen<boolean>("dock-edge-hover", (event) => {
      setIsEdgeHovered(event.payload);
    });

    // A click-through window gets no mouse events, so its mouseleave never comes. The backend says so instead.
    const unlistenCursorLeft = thisWindow.listen("dock-cursor-left", () => {
      isPreviewHoveredRef.current = false;
      setIsDockHovered(false);
      setHoveredApp(null);
      setPressedApp(null);
    });

    const unlistenFullscreen = thisWindow.listen<boolean>("dock-fullscreen", (event) => {
      setIsFullscreen(event.payload);
      if (event.payload) {
        setIsEdgeHovered(false);
        setIsDockHovered(false);
      }
    });

    const unlistenMonitors = listen("monitors-changed", refreshDockMode);
    const unlistenTray = listen<boolean>("tray-flyout-state", (event) => {
      setTrayOpen(event.payload);
      if (!event.payload) {
        setClickedShortcut(null);
      }
    });

    const unlistenMaximized = thisWindow.listen<boolean>("dock-maximized", (event) => {
      setIsMaximized(event.payload);
    });

    return () => {
      unlistenMaximized.then(f => f());
      unlistenOverlap.then(f => f());
      unlistenEdgeHover.then(f => f());
      unlistenCursorLeft.then(f => f());
      unlistenFullscreen.then(f => f());
      unlistenMonitors.then(f => f());
      unlistenTray.then(f => f());
    };
  }, []);

  useSettingsSync(
    {
      "fern-dock-mode": refreshDockMode,
      "fern-dock-mode-by-monitor": refreshDockMode,
      "fern-dock-monitor-mode": refreshDockMode,
      "fern-dock-monitor-id": refreshDockMode,
      "fern-dock-preview-enabled": setDockPreviewEnabled,
      "fern-dock-app-volume-scroll": (v) => setDockAppVolumeScroll(v !== false && v !== "false"),
      "fern-dock-preview-delay": (v) => setPreviewDelay(Number(v) || 0),
      "fern-dock-search-enabled": setDockSearchEnabled,
      "fern-dock-calendar-enabled": setDockCalendarEnabled,
      "fern-dock-tray-enabled": (v) => setDockTray(v !== false && v !== "false"),
      "fern-dock-taskview-enabled": setDockTaskView,
      "fern-dock-show-desktop-corner": setDockShowDesktop,
      "fern-dock-recycle-enabled": setDockRecycle,
      "fern-dock-drives": (v) => setDockDrives(parseDrives(v)),
      "fern-dock-icon-only": setDockIconOnly,
      "fern-dock-adaptive": setDockAdaptive,
      "fern-dock-attention-enabled": (v) => setDockAttentionEnabled(v !== false && v !== "false"),
      "fern-dock-hide-delay": (v: string) => setDockHideDelay(parseInt(v || "800", 10)),
      "fern-start-icon": setStartIcon,
      "fern-dock-mixed-reorder": setDockMixedReorder,
      "fern-dock-indicator": (v) => setWindowIndicator(v || "dot"),
      "fern-dock-show-end-task": (v) => setShowEndTask(v !== false && v !== "false"),
      "fern-dock-menu-items": (v) => setMenuItems(parseMenuItems(v)),
      "fern-scale": setScale,
    }
  );

  useEffect(() => {
    let pollSeq = 0;

    const poll = async () => {
      if (isDragging) return;
      const seq = ++pollSeq;
      const running = await invoke<AppInfo[]>('get_active_windows');
      // Drop out-of-order responses, an old poll must not bring back closed apps.
      if (seq !== pollSeq) return;
      setActiveApps(running);

      setActiveOrder(prev => {
        const newPaths = running.map(r => appIdentity(r.path, r.executable, r.name));
        const existingPaths = prev.filter(p => newPaths.includes(p));
        const addedPaths = newPaths.filter(p => !prev.includes(p));
        return [...existingPaths, ...addedPaths];
      });

      running.forEach(app => fetchIcon(app.path, app.name, app.hwnd));
    };

    poll();
    invoke<number[]>("get_attention_windows").then(setAttentionHwnds).catch(() => {});

    const unlistenAttention = listen<number[]>("app-attention", (event) => {
      setAttentionHwnds(event.payload);
    });

    const unlistenWindowChange = listen("windows-changed", () => {
      poll();
    });

    const unlistenSettingsReset = listen("settings-reset", () => {
      localStorage.clear();
      window.location.reload();
    });

    const unlistenIconCache = listen("icon-cache-cleared", () => {
      window.location.reload();
    });

    // Safety net: even if a window event is missed, converge on the real state
    const interval = windowsPollMs === null ? undefined : setInterval(poll, windowsPollMs);

    return () => {
      clearInterval(interval);
      unlistenAttention.then(f => f());
      unlistenWindowChange.then(f => f());
      unlistenSettingsReset.then(f => f());
      unlistenIconCache.then(f => f());
    };
  }, [isDragging, windowsPollMs]);

  const fetchIcon = async (path: string, name?: string, hwnd?: number, retryCount = 0) => {
    const isHost = isBrowserHost(path);
    const cacheKey = isHost && name ? `${path}:${name.toLowerCase()}` : (hwnd ? `${path}-${hwnd}` : path);
    
    if (iconsRef.current[cacheKey]) return;
    try {
      const icon = await invoke<string | null>('get_app_icon', { path, name: name || null, hwnd: hwnd || null });
      if (icon) {
        iconsRef.current[cacheKey] = icon;
        if (!isHost) iconsRef.current[path] = icon;
        setIconsTick(t => t + 1);
      } else if (retryCount < 3 && !hwnd) {
        setTimeout(() => fetchIcon(path, name, undefined, retryCount + 1), 3000 * (retryCount + 1));
      }
    } catch (e) {
      console.error(`Failed to fetch icon for ${path}:`, e);
      if (retryCount < 3 && !hwnd) {
        setTimeout(() => fetchIcon(path, name, undefined, retryCount + 1), 3000 * (retryCount + 1));
      }
    }
  };

  const handleClearIconCache = async () => {
    try {
      await invoke('clear_icon_cache');
      iconsRef.current = {};
      setIconsTick(t => t + 1);
      pinnedApps.forEach(app => fetchIcon(app.path));
    } catch (e) {
      console.error("Failed to clear icon cache:", e);
    }
  };

  const handleIconFileSelect = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    const target = iconPickerTargetRef.current;
    if (!file || !target) return;

    const reader = new FileReader();
    reader.onload = async () => {
      const dataUri = reader.result as string;
      try {
        const newIcon = await invoke<string>('set_custom_icon', {
          cacheKey: target,
          iconData: dataUri
        });
        setCustomIcons(prev => ({ ...prev, [target]: newIcon }));
      } catch (err) {
        const msg = typeof err === 'string' ? err : t('dockmenu.iconError');
        if (toastTimerRef.current) clearTimeout(toastTimerRef.current);
        setToast(msg);
        toastTimerRef.current = setTimeout(() => setToast(null), 4000);
      }
    };
    reader.readAsDataURL(file);
    e.target.value = '';
  };

  const handleRemoveCustomIcon = async (app: AppInfo) => {
    try {
      await invoke('remove_custom_icon', { path: app.path, name: app.name || null });
      const isHost = isBrowserHost(app.path);
      const ck = isHost && app.name ? `${app.path}:${app.name.toLowerCase()}` : (app.hwnd ? `${app.path}-${app.hwnd}` : app.path);
      setCustomIcons(prev => {
        const next = { ...prev };
        delete next[ck];
        delete next[app.path];
        return next;
      });
      // Invalidate only this app's cached icon and re-fetch only this app
      delete iconsRef.current[ck];
      delete iconsRef.current[app.path];
      if (app.hwnd) delete iconsRef.current[`${app.path}-${app.hwnd}`];
      setIconsTick(t => t + 1);
      fetchIcon(app.path, app.name, app.hwnd);
    } catch (err) {
      console.error("Failed to remove custom icon:", err);
    }
  };

  const handleClosePreview = async (e: React.MouseEvent, hwnd: number) => {
    e.stopPropagation();
    try {
      const closed = await invoke<boolean>('close_window', { hwnd });
      if (!closed) {
        // Still open, so it stays in the preview. The backend has brought it to the front.
        if (toastTimerRef.current) clearTimeout(toastTimerRef.current);
        setToast(t('dockmenu.closeElevated'));
        toastTimerRef.current = setTimeout(() => setToast(null), 4000);
        return;
      }
      setPreviewData(prev => {
        if (!prev) return null;
        const remaining = prev.previews.filter(p => p.hwnd !== hwnd);
        if (remaining.length === 0) {
          setHoveredApp(null);
          return null;
        }
        return { ...prev, previews: remaining };
      });
    } catch (err) {
      console.error("Failed to close window:", err);
    }
  };

  const handleAppClick = async (app: AppInfo) => {
    tourDockAction();
    try {
      if (app.path === 'start') {
        await invoke('open_app', { appName: 'start' });
      } else if (app.all_hwnds && app.all_hwnds.length > 1) {
        // Several windows: bring the most recent forward, then cycle.
        await invoke('focus_app_windows', { hwnds: app.all_hwnds.map(([hwnd]) => hwnd) });
      } else if (app.hwnd) {
        await invoke('focus_window', { hwnd: app.hwnd });
      } else {
        await invoke('open_app', { appName: app.path, args: app.args ?? null });
      }
    } catch (e) {
      console.error(`Failed to interact with ${app.name}:`, e);
    }
  };

  const handleNewInstance = async (app: AppInfo) => {
    if (!app || app.path === 'start') return;
    try {
      await invoke('launch_new_instance', { appPath: app.path, appName: app.name, args: app.args ?? null });
    } catch (e) {
      console.error(`Failed to launch a new instance of ${app.name}:`, e);
    }
  };

  // Middle-click opens a new instance like the taskbar. The mousedown preventDefault
  // stops the autoscroll cursor.
  const handleMiddleClick = (e: React.MouseEvent, app: AppInfo) => {
    if (e.button !== 1) return;
    e.preventDefault();
    e.stopPropagation();
    tourDockAction();
    handleNewInstance(app);
  };

  const togglePin = async (app: AppInfo) => {
    let newPinned;
    if (app.is_pinned) {
      newPinned = pinnedApps.filter(a => itemKey(a) !== itemKey(app));
    } else {
      // Same app pinned under another entry shape (running window vs shortcut) must not duplicate.
      if (pinnedApps.some(a => isSameApp(a, app))) return;
      newPinned = [...pinnedApps, { ...app, is_pinned: true, is_running: false, hwnd: undefined }];
      fetchIcon(app.path, app.name); 
    }
    setPinnedApps(newPinned);
    await invoke('save_pinned_apps', { apps: newPinned });
    closeMenu();
  };

  const menuRef = useRef<HTMLDivElement>(null);
  const popupRef = useRef<HTMLDivElement>(null);



  const handleContextMenu = (e: React.MouseEvent, app: AppInfo | null) => {
    e.stopPropagation();
    e.preventDefault();
    tourDockAction();
    if (previewTimerRef.current) clearTimeout(previewTimerRef.current);
    isPreviewHoveredRef.current = false;
    setPreviewData(null);
    setHoveredApp(null);
    invoke('clear_live_thumbnails').catch(() => {});
    setContextMenuHeight(0);
    setContextMenuWidth(0);
    const dockRect = dockRef.current?.getBoundingClientRect();
    const target = (e.currentTarget as HTMLElement).getBoundingClientRect();
    setContextMenu({
      x: app ? target.left + target.width / 2 : (dockRect ? dockRect.left + dockRect.width / 2 : e.clientX),
      y: dockRect ? dockRect.top : e.clientY,
      app,
    });
  };

  const closeMenu = () => {
    setContextMenu(null);
    setActiveSubmenu(null);
    invoke('set_menu_open', { open: false, rect: null }).catch(() => {});
  };

  const fernOptionsMenu = () => (
    <div 
      className="menu-item has-submenu"
      onMouseEnter={() => setActiveSubmenu('fern')}
      onMouseLeave={() => setActiveSubmenu(null)}
    >
      {t("dockmenu.options")}
      <span className="submenu-arrow">▶</span>
      <div className="submenu">
        {menuItems.includes("calendar") && (
          <div className="menu-item" onClick={() => { invoke('open_calendar_window'); closeMenu(); }}>{t("dockmenu.openCalendar")}</div>
        )}
        {menuItems.includes("settings") && (
          <div className="menu-item" onClick={() => { invoke('open_settings_window'); closeMenu(); }}>{t("dockmenu.openSettings")}</div>
        )}
        {menuItems.includes("taskmgr") && (
          <div className="menu-item" onClick={() => { invoke('open_app', { appName: 'taskmgr' }); closeMenu(); }}>{t("dockmenu.taskManager")}</div>
        )}
        {menuItems.includes("restart") && (
          <div className="menu-item" onClick={() => invoke('restart_fern')}>{t("general.restart")}</div>
        )}
        {menuItems.includes("clearicons") && (
          <div className="menu-item" onClick={() => { handleClearIconCache(); closeMenu(); }}>{t("dockmenu.clearIconCache")}</div>
        )}
        {menuItems.includes("quit") && (
          <>
            <div className="menu-divider" />
            <div className="menu-item quit" onClick={() => invoke('quit_fern')}>{t("general.quit")}</div>
          </>
        )}
      </div>
    </div>
  );

  const closePopup = () => {
    setShowAddPopup(false);
    invoke('set_menu_open', { open: false, rect: null }).catch(() => {});
  };

  useEffect(() => {
    if (contextMenu) {
      if (previewTimerRef.current) clearTimeout(previewTimerRef.current);
      isPreviewHoveredRef.current = false;
      setPreviewData(null);
      setHoveredApp(null);
      invoke('clear_live_thumbnails').catch(() => {});
    }
  }, [contextMenu]);

  useLayoutEffect(() => {
    if (contextMenu && menuRef.current) {
      const measured = menuRef.current.offsetHeight;
      if (measured > 0 && measured !== contextMenuHeight) {
        setContextMenuHeight(measured);
      }
      const measuredWidth = menuRef.current.getBoundingClientRect().width;
      if (measuredWidth > 0 && Math.abs(measuredWidth - contextMenuWidth) > 0.5) {
        setContextMenuWidth(measuredWidth);
      }
    } else if (!contextMenu && (contextMenuHeight !== 0 || contextMenuWidth !== 0)) {
      setContextMenuHeight(0);
      setContextMenuWidth(0);
    }
  }, [contextMenu, contextMenuHeight, contextMenuWidth]);

  useEffect(() => {
    let open = false;
    let rect = null;

    if (contextMenu && menuRef.current) {
      const r = menuRef.current.getBoundingClientRect();
      rect = { 
        x: Math.round(r.x), 
        y: Math.round(r.y), 
        width: Math.round(r.width + (activeSubmenu ? 160 : 0)), 
        height: Math.round(r.height) 
      };
      open = true;
    } else if (showAddPopup && popupRef.current) {
      const r = popupRef.current.getBoundingClientRect();
      rect = { 
        x: Math.round(r.x), 
        y: Math.round(r.y), 
        width: Math.round(r.width), 
        height: Math.round(r.height) 
      };
      open = true;
    }

    invoke('set_menu_open', { open, rect }).catch(() => {});

  }, [contextMenu, showAddPopup, pinnedApps, activeApps, activeSubmenu, scale]);

  const dockItems = useMemo(() => {
    const runningMap = new Map();
    activeApps.forEach(a => {
      const id = appIdentity(a.path, a.executable, a.name);
      if (!runningMap.has(id)) runningMap.set(id, a);
    });
    
    const matchedRunningKeys = new Set<string>();

    const findRunningApp = (p: AppInfo) => {
      // 1. Try exact match by identity
      const id = appIdentity(p.path, p.executable, p.name);
      let running = runningMap.get(id);
      if (running) {
        matchedRunningKeys.add(appIdentity(running.path, running.executable, running.name));
        return running;
      }

      // 2. Try match by path (without executable)
      const pathId = appIdentity(p.path);
      running = runningMap.get(pathId);
      if (running) {
        matchedRunningKeys.add(appIdentity(running.path, running.executable, running.name));
        return running;
      }

      // 3. Try fallback match by executable name if defined
      if (p.executable) {
        const targetExe = p.executable.toLowerCase();
        const found = activeApps.find(a => !isIdentifier(a.path) && a.executable?.toLowerCase() === targetExe);
        if (found) {
          matchedRunningKeys.add(appIdentity(found.path, found.executable, found.name));
          return found;
        }
      }

      // 4. Try fallback match by path's file name (e.g., if path is "msedge" and running app's executable is "msedge.exe")
      const pinFilename = p.path.split('/').pop()?.split('\\').pop()?.toLowerCase() || "";
      if (pinFilename) {
        const found = activeApps.find(a => {
          if (isIdentifier(a.path)) return false;
          const runExe = a.executable?.toLowerCase() || a.path.split('/').pop()?.split('\\').pop()?.toLowerCase() || "";
          return runExe === pinFilename || runExe === `${pinFilename}.exe` || `${runExe}.exe` === pinFilename;
        });
        if (found) {
          matchedRunningKeys.add(appIdentity(found.path, found.executable, found.name));
          return found;
        }
      }

      return undefined;
    };

    const pinned: AppInfo[] = [
      { name: 'Start', path: 'start', icon: null, is_running: false, is_pinned: true, hwnd: undefined, all_hwnds: undefined, executable: undefined },
      ...pinnedApps.map(p => {
        const running = findRunningApp(p);
        return { ...p, is_running: !!running, hwnd: running?.hwnd, all_hwnds: running?.all_hwnds };
      })
    ];

    const isAppHidden = (a: AppInfo) => {
      const key1 = a.path?.toLowerCase();
      const key2 = a.executable?.toLowerCase();
      const key3 = a.name?.toLowerCase();
      return hiddenApps.some((h) => {
        const hl = h.toLowerCase();
        return hl === key1 || hl === key2 || hl === key3;
      });
    };

    const unpinned = activeOrder
      .map(id => activeApps.find(a => appIdentity(a.path, a.executable, a.name) === id))
      .filter((a): a is AppInfo => !!a && !matchedRunningKeys.has(appIdentity(a.path, a.executable, a.name)) && !isAppHidden(a));

    return [...pinned, ...unpinned];
  }, [pinnedApps, activeApps, activeOrder, hiddenApps]);

  const startItem = useMemo(() => dockItems.find(i => i.path === 'start') as AppInfo, [dockItems]);
  const pinnedItems = useMemo(() => dockItems.filter(i => i.path !== 'start' && i.is_pinned), [dockItems]);
  const unpinnedItems = useMemo(() => dockItems.filter(i => !i.is_pinned), [dockItems]);

  // Latest state for the Win+Number listener, which subscribes only once.
  useEffect(() => {
    pinnedItemsRef.current = pinnedItems;
    handleAppClickRef.current = handleAppClick;
  });

  useEffect(() => {
    // Backend claims Win+1-9 while the taskbar is hidden and tells us the slot, same as clicking that icon.
    const unlisten = getCurrentWebviewWindow().listen<number>("dock-win-number", (event) => {
      const app = pinnedItemsRef.current[event.payload];
      if (app) handleAppClickRef.current(app);
    });
    return () => { unlisten.then(f => f()); };
  }, []);

  const combinableItems = useMemo(() => dockItems.filter(i => i.path !== 'start'), [dockItems]);
  const mixedItems = useMemo(() => {
    if (!dockMixedReorder) return combinableItems;
    const known = combinableItems.filter(i => mixedOrder.includes(itemKey(i)));
    const unknown = combinableItems.filter(i => !mixedOrder.includes(itemKey(i)));
    known.sort((a, b) => mixedOrder.indexOf(itemKey(a)) - mixedOrder.indexOf(itemKey(b)));
    return [...known, ...unknown];
  }, [combinableItems, dockMixedReorder, mixedOrder]);

  // Keyed by identity, not path: web apps in one browser share an exe path.
  const handleReorder = (newKeys: string[]) => {
    const oldKeys = pinnedApps.map(itemKey);
    if (JSON.stringify(newKeys) !== JSON.stringify(oldKeys)) {
      const reordered = newKeys
        .map(key => pinnedApps.find(p => itemKey(p) === key))
        .filter((p): p is AppInfo => !!p);
      setPinnedApps(reordered);
    }
  };

  const handleDragEnd = () => {
    setIsDragging(false);
    setPressedApp(null);
    invoke('save_pinned_apps', { apps: pinnedApps }).catch(console.error);
  };

  const handleUnpinnedReorder = (newPaths: string[]) => {
    setActiveOrder(newPaths);
  };

  const handleUnpinnedDragEnd = () => {
    setIsDragging(false);
    setPressedApp(null);
  };

  const handleMixedReorder = (newKeys: string[]) => {
    setMixedOrder(newKeys);
    const pinnedKeys = new Set(pinnedApps.map(itemKey));
    const newPinnedOrder = newKeys.filter(k => pinnedKeys.has(k));
    const oldPinnedOrder = pinnedApps.map(itemKey);
    if (JSON.stringify(newPinnedOrder) !== JSON.stringify(oldPinnedOrder)) {
      const reordered = newPinnedOrder
        .map(key => pinnedApps.find(p => itemKey(p) === key))
        .filter((p): p is AppInfo => !!p);
      setPinnedApps(reordered);
    }
  };

  const handleAppDragOver = useCallback((e: React.DragEvent, app: AppInfo) => {
    if (isDragging) return;
    if (!app.is_running) return;
    const targetHwnd = app.all_hwnds && app.all_hwnds.length > 0 ? app.all_hwnds[0][0] : app.hwnd;
    if (!targetHwnd) return;

    e.preventDefault();
    e.stopPropagation();
    e.dataTransfer.dropEffect = 'copy';

    const key = itemKey(app);
    if (dragHoverAppKeyRef.current === key) {
      return;
    }

    if (dragHoverTimerRef.current) {
      clearTimeout(dragHoverTimerRef.current);
      dragHoverTimerRef.current = null;
    }

    dragHoverAppKeyRef.current = key;
    setDragHoveringApp(key);

    dragHoverTimerRef.current = setTimeout(async () => {
      try {
        await invoke('bring_hwnd_to_front', { hwnd: targetHwnd });
      } catch (err) {
        console.error('Failed to bring window to front on drag hover:', err);
      }
    }, 450);
  }, [isDragging]);

  const handleAppDragLeave = useCallback((_e: React.DragEvent, app: AppInfo) => {
    if (isDragging) return;
    const key = itemKey(app);
    if (dragHoverAppKeyRef.current === key) {
      if (dragHoverTimerRef.current) {
        clearTimeout(dragHoverTimerRef.current);
        dragHoverTimerRef.current = null;
      }
      dragHoverAppKeyRef.current = null;
      setDragHoveringApp(null);
    }
  }, [isDragging]);

  const handleAppDrop = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
    if (dragHoverTimerRef.current) {
      clearTimeout(dragHoverTimerRef.current);
      dragHoverTimerRef.current = null;
    }
    dragHoverAppKeyRef.current = null;
    setDragHoveringApp(null);
  }, []);

  useEffect(() => {
    return () => {
      if (dragHoverTimerRef.current) {
        clearTimeout(dragHoverTimerRef.current);
      }
    };
  }, []);

  const handleMixedDragEnd = (app: AppInfo) => {
    setIsDragging(false);
    setPressedApp(null);
    if (app.is_pinned) {
      invoke('save_pinned_apps', { apps: pinnedApps }).catch(console.error);
    }
  };

  useEffect(() => {
    // Only report true dock-window hover, not the edge-hover from Rust,
    // to avoid a feedback loop that keeps the dock open.
    invoke('set_dock_hovered', { hovered: isDockHovered }).catch(() => {});
  }, [isDockHovered]);

  useEffect(() => {
    invoke('set_dock_dragging', { dragging: isDragging }).catch(() => {});
  }, [isDragging]);

  useEffect(() => {
    const handleBlur = () => {
      closeMenu();
      closePopup();
    };
    window.addEventListener('blur', handleBlur);
    return () => window.removeEventListener('blur', handleBlur);
  }, []);

  useEffect(() => {
    if (contextMenu) {
      if (previewTimerRef.current) clearTimeout(previewTimerRef.current);
      isPreviewHoveredRef.current = false;
      setPreviewData(null);
      invoke('clear_live_thumbnails').catch(() => {});
      return;
    }

    if (previewTimerRef.current) clearTimeout(previewTimerRef.current);
    isPreviewHoveredRef.current = false;
    hoveredAppRef.current = hoveredApp;

    if (!hoveredApp) {
      const timer = setTimeout(() => setPreviewData(null), 100);
      return () => clearTimeout(timer);
    }

    if (hoveredApp && !isDragging) {
      if (activeVolumeKey) {
        setPreviewData(null);
        return;
      }
      const app = dockItems.find(a => itemKey(a) === hoveredApp);
      if (app && app.is_running) {
        const hwndsToCapture = app.all_hwnds || (app.hwnd ? [[app.hwnd, app.name]] : []);

        previewTimerRef.current = setTimeout(async () => {
          try {
            const results = await Promise.all(
              hwndsToCapture.map(async ([hwnd, title]) => {
                try {
                  const res = await invoke<[string, number] | null>("capture_window_thumbnail", { hwnd, maxWidth: 320, maxHeight: 200 });
                  if (res) {
                    const [image, lastFocused] = res;
                    return { hwnd, title, image, lastFocused };
                  }
                } catch {}
                return { hwnd, title, image: "", lastFocused: 0 };
              })
            );

            const captured = results
              .sort((a, b) => b.lastFocused - a.lastFocused)
              .map(({ hwnd, title, image }) => ({ hwnd, title, image }));

            const currentHovered = hoveredAppRef.current;
            if (!contextMenu && captured.length > 0 && currentHovered === itemKey(app)) {
              setPreviewData({ id: itemKey(app), previews: captured });
            } else if (currentHovered === itemKey(app)) {
              setPreviewData(null);
            }
          } catch (e) {
            console.error("Failed to capture thumbnails:", e);
            setPreviewData(null);
          }
        }, previewDelay);
      } else {
        setPreviewData(null);
      }
    }

    return () => {
      if (previewTimerRef.current) clearTimeout(previewTimerRef.current);
    };
  }, [hoveredApp, isDragging, dockItems, previewDelay, contextMenu]);

  const iconVariants = {
    idle: { y: 0, scale: 1 },
    hover: { y: -5, scale: 1.1 },
    drag: { y: -10, scale: 1.1, opacity: 0.8 },
    tap: { scale: 0.95 }
  };

  const previewAppId = dockPreviewEnabled ? previewData?.id : undefined;
  useEffect(() => {
    if (!previewAppId) {
      setMediaSessions([]);
      return;
    }
    const refresh = () => invoke<AppMediaSession[]>('get_app_media_sessions').then(setMediaSessions).catch(() => {});
    refresh();
    const timer = setInterval(refresh, 2000);
    return () => clearInterval(timer);
  }, [previewAppId]);

  const controlMedia = (e: React.MouseEvent, session: AppMediaSession, action: 'previous' | 'play_pause' | 'next') => {
    e.stopPropagation();
    invoke('app_media_control', { appId: session.app_id, action })
      .then(() => setTimeout(() => invoke<AppMediaSession[]>('get_app_media_sessions').then(setMediaSessions).catch(() => {}), 300))
      .catch(() => {});
  };

  const renderAppIcon = (app: AppInfo) => {
    const isHost = isBrowserHost(app.path);
    const cacheKey = isHost ? `${app.path}:${app.name.toLowerCase()}` : (app.hwnd ? `${app.path}-${app.hwnd}` : app.path);
    const allowPathFallback = !isHost || !app.is_running;
    const icon = customIcons[cacheKey] || (allowPathFallback && customIcons[app.path]) || iconsRef.current[cacheKey] || (allowPathFallback && iconsRef.current[app.path]) || app.icon;
    const isFernOrSettings = isFernWindow(app);
    return (icon || isFernOrSettings) ? (
      <DockAppIcon src={isFernOrSettings ? "/fern.png" : (icon ?? undefined)} alt={localizeAppName(app.name)} fern={isFernOrSettings} />
    ) : (
      <div className="fallback-icon">{app.name[0]}</div>
    );
  };

  const renderPreviewMedia = (app: AppInfo) => {
    const session = sessionForApp(app, mediaSessions);
    if (!session || !session.title) return null;
    return (
      <div className="preview-media" onClick={(e) => e.stopPropagation()}>
        <div className="preview-media-info">
          <span className="preview-media-title">{session.title}</span>
          {session.artist && <span className="preview-media-artist">{session.artist}</span>}
        </div>
        <div className="preview-media-controls">
          <button aria-label={t("media.previous")} onClick={(e) => controlMedia(e, session, 'previous')}><SkipBackIcon size={14} /></button>
          <button
            aria-label={session.is_playing ? t("media.pause") : t("media.play")}
            onClick={(e) => controlMedia(e, session, 'play_pause')}
          >
            {session.is_playing ? <PauseIcon size={16} /> : <PlayIcon size={16} />}
          </button>
          <button aria-label={t("media.next")} onClick={(e) => controlMedia(e, session, 'next')}><SkipForwardIcon size={14} /></button>
        </div>
      </div>
    );
  };

  const renderShortcut = (
    id: string,
    label: string,
    delay: number,
    onClick: (rect: DOMRect, e: React.MouseEvent) => void,
    icon: React.ReactNode,
    onContextMenu?: (e: React.MouseEvent) => void
  ) => {
    const isTray = id === 'tray';
    const hideTooltip = clickedShortcut === id || (isTray && trayOpen);
    return (
      <motion.div
        key={id}
        layout="position"
        initial={{ opacity: 0, scale: 0.8 }}
        animate={{ opacity: 1, scale: 1 }}
        transition={{
          opacity: { duration: 0.15, delay },
          scale: { type: "spring", stiffness: 400, damping: 25, delay },
          layout: isDragging ? { duration: 0 } : DOCK_SPRING
        }}
        className={`dock-icon-wrapper ${hideTooltip ? 'shortcut-active' : ''}`}
        onContextMenu={onContextMenu}
        onMouseEnter={() => { if (!contextMenu) setHoveredApp(id); }}
        onMouseLeave={() => { setHoveredApp(null); setPressedApp(null); setClickedShortcut(null); }}
      >
        {!contextMenu && hoveredApp === id && !hideTooltip && <div className="tooltip">{label}</div>}
        <motion.div
          className="dock-icon"
          variants={iconVariants}
          animate={pressedApp === id ? "tap" : (hoveredApp === id ? "hover" : "idle")}
          onPointerDown={() => setPressedApp(id)}
          onPointerUp={() => setPressedApp(null)}
          onPointerCancel={() => setPressedApp(null)}
          onClick={(e) => {
            e.stopPropagation();
            setHoveredApp(null);
            setPressedApp(null);
            setClickedShortcut(id);
            onClick(e.currentTarget.getBoundingClientRect(), e);
          }}
        >
          {icon}
        </motion.div>
      </motion.div>
    );
  };

  // Clip the dock window to what it draws (see window_region.rs), so its empty parts never take clicks.
  // A menu closes on a click anywhere and a dragged icon can go anywhere, so those get the whole window,
  // as does the fly-in on startup and the toast, which has its own position.
  const sendDockRegion = useMemo(
    () => createRegionSender((rects) => { invoke("set_window_regions", { rects }).catch(() => {}); }),
    []
  );
  updateDockRegionRef.current = () => {
    const w = window.innerWidth;
    const h = window.innerHeight;
    if (!isReady || isDragging || contextMenu || showAddPopup || toast) {
      sendDockRegion([fullWindow(w, h)]);
      return;
    }
    const bar = dockRef.current?.getBoundingClientRect();
    if (bar && isExpanded && !isHidden) {
      dockBarExtentRef.current.width = Math.max(dockBarExtentRef.current.width, bar.width);
      dockBarExtentRef.current.height = Math.max(dockBarExtentRef.current.height, bar.height);
    }
    const extent = dockBarExtentRef.current;
    const wanted = [
      dockBand(
        w,
        h,
        isAdaptive ? w : Math.max(extent.width, 72 * scale),
        Math.max(extent.height, 72 * scale),
        DOCK_REGION_WIDTH_PAD * scale,
        DOCK_REGION_LIFT * scale,
      ),
    ];
    // The same area the backend's hit test gives an open preview.
    if (previewData && bar) {
      wanted.push({ x: bar.x - 500, y: bar.y - 320, width: bar.width + 1000, height: bar.height + 320 });
    }
    sendDockRegion(wanted);
  };
  // Sent after every render, but only when the region changes.
  useEffect(() => updateDockRegionRef.current());
  useEffect(() => {
    const onResize = () => updateDockRegionRef.current();
    window.addEventListener('resize', onResize);
    return () => window.removeEventListener('resize', onResize);
  }, []);

  return (
    <div
      className={`dock-container ${isDragging ? 'dragging' : ''}`}
      onClick={closeMenu}
      onDragOver={(e) => {
        e.preventDefault();
        if (!isDockHovered) setIsDockHovered(true);
      }}
      onDrop={(e) => {
        e.preventDefault();
      }}
    >
      <div style={{ width: '100%', height: '100%', display: 'flex', justifyContent: 'center', alignItems: 'flex-end' }}>
        <motion.div
          ref={dockRef}
          layout
          className={`dock ${isExpanded ? 'dock-expanded' : ''} ${(isImpacted || isExpanded) ? 'dock-impacted' : ''} ${dockIconOnly ? 'dock-icon-only' : ''} ${isAdaptive ? 'dock-adaptive' : ''}`}
          onMouseEnter={() => setIsDockHovered(true)}
          // The backend may have ended the hover without the DOM knowing, then no new mouseenter comes.
          onMouseMove={() => { if (!isDockHovered) setIsDockHovered(true); }}
          onMouseLeave={() => { setIsDockHovered(false); setHoveredApp(null); setPressedApp(null); }}
          onDragOver={(e) => {
            e.preventDefault();
            if (!isDockHovered) setIsDockHovered(true);
          }}
          onDragLeave={(e) => {
            if (!e.currentTarget.contains(e.relatedTarget as Node)) {
              if (dragHoverTimerRef.current) {
                clearTimeout(dragHoverTimerRef.current);
                dragHoverTimerRef.current = null;
              }
              dragHoverAppKeyRef.current = null;
              setDragHoveringApp(null);
            }
          }}
          onDrop={(e) => {
            e.preventDefault();
          }}
        initial={{ y: -800, opacity: 1, width: 34, height: 34, borderTopLeftRadius: 17, borderTopRightRadius: 17, borderBottomLeftRadius: 17, borderBottomRightRadius: 17 }}
        animate={{
          y: !isReady ? -800 : (isHidden ? 100 : 0),
          width: isExpanded ? (isAdaptive ? adaptiveWidth : 'auto') : 34,
          height: isExpanded ? 'auto' : 34,
          borderTopLeftRadius: (isImpacted || isExpanded) ? 18 : 17,
          borderTopRightRadius: (isImpacted || isExpanded) ? 18 : 17,
          borderBottomLeftRadius: (isImpacted || isExpanded) ? 0 : 17,
          borderBottomRightRadius: (isImpacted || isExpanded) ? 0 : 17,
          opacity: 1,
          scale: scale,
        }}
        transition={{
          y: { type: "spring", stiffness: 400, damping: 35, mass: 0.8 },
          width: { type: "spring", stiffness: 350, damping: 30 },
          height: { type: "spring", stiffness: 350, damping: 30 },
          layout: isDragging ? { duration: 0 } : DOCK_SPRING,
          borderTopLeftRadius: { type: "spring", stiffness: 500, damping: 30 },
          borderTopRightRadius: { type: "spring", stiffness: 500, damping: 30 },
          borderBottomLeftRadius: { type: "spring", stiffness: 500, damping: 30 },
          borderBottomRightRadius: { type: "spring", stiffness: 500, damping: 30 },
          opacity: { type: "tween", duration: 0.2 },
          scale: { duration: 0 },
        }}
        style={{ originX: 0.5, originY: 1, minWidth: 34 }}
        onContextMenu={(e) => handleContextMenu(e, null)}
      >
        <SurfaceShape
          edge="bottom"
          flare={isExpanded && !isHidden ? 13 : 0}
          radius={(isImpacted || isExpanded) ? 18 : 17}
          edgeRadius={(isImpacted || isExpanded) ? 0 : 17}
        />
        <AnimatePresence>
          {isExpanded && (
            <motion.div
              key="dock-content"
              layout
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              exit={{ opacity: 0 }}
              transition={isDragging ? { duration: 0 } : DOCK_SPRING}
              className="dock-reorder-container"
            >
              {startItem && (
                <motion.div
                  layout="position"
                  initial={{ opacity: 0, scale: 0.8 }}
                  animate={{ opacity: 1, scale: 1 }}
                  transition={{ opacity: { duration: 0.15, delay: 0.15 }, scale: { type: "spring", stiffness: 400, damping: 25, delay: 0.15 }, layout: isDragging ? { duration: 0 } : DOCK_SPRING }}
                  className="dock-icon-wrapper"
                  onContextMenu={(e) => handleContextMenu(e, startItem)}
                  onMouseEnter={() => { if (!contextMenu) setHoveredApp(itemKey(startItem)); }}
                  onMouseLeave={() => { setHoveredApp(null); setPressedApp(null); }}
                >
                  {!contextMenu && (!dockPreviewEnabled || (dockPreviewEnabled && hoveredApp === itemKey(startItem))) && (
                    <div className="tooltip">{t("dockmenu.start")}</div>
                  )}
                  <motion.div 
                    className="dock-icon"
                    variants={iconVariants}
                    animate={pressedApp === itemKey(startItem) ? "tap" : (hoveredApp === itemKey(startItem) ? "hover" : "idle")}
                    onPointerDown={() => setPressedApp(itemKey(startItem))}
                    onPointerUp={() => setPressedApp(null)}
                    onPointerCancel={() => setPressedApp(null)}
                    onClick={(e) => {
                      e.stopPropagation();
                      handleAppClick(startItem);
                    }}
                  >
                    <img src={resolveStartIcon(startIcon)} alt="Start" className={startIcon === "default" ? "fern-icon-img" : "fern-icon-img start-img"} style={startIcon.startsWith("custom:") ? { borderRadius: "8px" } : undefined} draggable={false} />
                  </motion.div>
                </motion.div>
              )}
              
              {dockSearchEnabled && (
                <motion.div
                  layout="position"
                  initial={{ opacity: 0, scale: 0.8 }}
                  animate={{ opacity: 1, scale: 1 }}
                  transition={{ opacity: { duration: 0.15, delay: 0.18 }, scale: { type: "spring", stiffness: 400, damping: 25, delay: 0.18 }, layout: isDragging ? { duration: 0 } : DOCK_SPRING }}
                  className="dock-icon-wrapper"
                  onMouseEnter={() => { if (!contextMenu) setHoveredApp('search'); }}
                  onMouseLeave={() => { setHoveredApp(null); setPressedApp(null); }}
                >
                  {!contextMenu && (!dockPreviewEnabled || (dockPreviewEnabled && hoveredApp === 'search')) && (
                    <div className="tooltip">{t("dockmenu.search")}</div>
                  )}
                  <motion.div
                    className="dock-icon"
                    variants={iconVariants}
                    animate={pressedApp === 'search' ? "tap" : (hoveredApp === 'search' ? "hover" : "idle")}
                    onPointerDown={() => setPressedApp('search')}
                    onPointerUp={() => setPressedApp(null)}
                    onPointerCancel={() => setPressedApp(null)}
                    onClick={(e) => {
                      e.stopPropagation();
                      invoke('open_app', { appName: 'search' }).catch((err) => console.error('Failed to open search:', err));
                    }}
                  >
                    <SearchIcon />
                  </motion.div>
                </motion.div>
              )}

              {dockCalendarEnabled && (
                <motion.div
                  layout="position"
                  initial={{ opacity: 0, scale: 0.8 }}
                  animate={{ opacity: 1, scale: 1 }}
                  transition={{ opacity: { duration: 0.15, delay: 0.2 }, scale: { type: "spring", stiffness: 400, damping: 25, delay: 0.2 }, layout: isDragging ? { duration: 0 } : DOCK_SPRING }}
                  className="dock-icon-wrapper"
                  onMouseEnter={() => { if (!contextMenu) setHoveredApp('calendar'); }}
                  onMouseLeave={() => { setHoveredApp(null); setPressedApp(null); }}
                >
                  {!contextMenu && (!dockPreviewEnabled || (dockPreviewEnabled && hoveredApp === 'calendar')) && (
                    <div className="tooltip">{t("dockmenu.calendar")}</div>
                  )}
                  <motion.div
                    className="dock-icon"
                    variants={iconVariants}
                    animate={pressedApp === 'calendar' ? "tap" : (hoveredApp === 'calendar' ? "hover" : "idle")}
                    onPointerDown={() => setPressedApp('calendar')}
                    onPointerUp={() => setPressedApp(null)}
                    onPointerCancel={() => setPressedApp(null)}
                    onClick={(e) => {
                      e.stopPropagation();
                      invoke('open_calendar_window').catch((err) => console.error('Failed to open calendar:', err));
                    }}
                  >
                    <CalendarIcon />
                  </motion.div>
                </motion.div>
              )}

              {dockTray && renderShortcut(
                'tray', t('dockmenu.hiddenIcons'), 0.21,
                (rect, e) => invoke('open_tray_from_button', {
                  dx: (rect.left + rect.width / 2 - e.clientX) * window.devicePixelRatio,
                  dy: (rect.top - e.clientY) * window.devicePixelRatio,
                }).catch((err) => console.error('Failed to open tray:', err)),
                <ChevronUpIcon />
              )}

              {dockTaskView && renderShortcut(
                'taskview', t('dockmenu.taskView'), 0.22,
                () => invoke('open_app', { appName: 'taskview' }).catch((err) => console.error('Failed to open Task View:', err)),
                <TaskViewIcon />
              )}

              {dockDrives.map((letter, i) => renderShortcut(
                `drive-${letter}`, t('dock.drive', { letter }), 0.24 + i * 0.02,
                () => invoke('open_dock_extra', { target: letter }).catch((err) => console.error('Failed to open drive:', err)),
                <DriveIcon letter={letter} />,
                (e) => {
                  e.stopPropagation();
                  e.preventDefault();
                  if (previewTimerRef.current) clearTimeout(previewTimerRef.current);
                  isPreviewHoveredRef.current = false;
                  setPreviewData(null);
                  setHoveredApp(null);
                  invoke('clear_live_thumbnails').catch(() => {});
                  setContextMenuHeight(0);
                  setContextMenuWidth(0);
                  refreshDriveDetails();
                  const target = e.currentTarget.getBoundingClientRect();
                  setContextMenu({
                    x: target.left + target.width / 2,
                    y: dockRef.current?.getBoundingClientRect().top ?? e.clientY,
                    app: null,
                    extraKind: 'drive',
                    driveLetter: letter,
                    isRemovable: driveDetails[letter]?.removable ?? false,
                  });
                }
              ))}

              {dockRecycle && renderShortcut(
                'recycle', t('dockmenu.recycleBin'), 0.26 + dockDrives.length * 0.02,
                () => {
                  invoke('open_dock_extra', { target: 'recycle' }).catch((err) => console.error('Failed to open Recycle Bin:', err));
                  refreshRecycleStatus();
                },
                <RecycleIcon isEmpty={recycleStatus ? recycleStatus.is_empty : true} />,
                (e) => {
                  e.stopPropagation();
                  e.preventDefault();
                  if (previewTimerRef.current) clearTimeout(previewTimerRef.current);
                  isPreviewHoveredRef.current = false;
                  setPreviewData(null);
                  setHoveredApp(null);
                  invoke('clear_live_thumbnails').catch(() => {});
                  setContextMenuHeight(0);
                  setContextMenuWidth(0);
                  refreshRecycleStatus();
                  const target = e.currentTarget.getBoundingClientRect();
                  setContextMenu({
                    x: target.left + target.width / 2,
                    y: dockRef.current?.getBoundingClientRect().top ?? e.clientY,
                    app: null,
                    extraKind: 'recycle',
                  });
                }
              )}

              {!dockMixedReorder ? (
              <>
              <Reorder.Group
                as="div"
                axis="x"
                layout
                values={pinnedItems.map(itemKey)}
                onReorder={handleReorder}
                className="dock-reorder-group"
              >
                {pinnedItems.map((app) => (
                  <Reorder.Item
                    as="div"
                    key={itemKey(app)}
                    value={itemKey(app)}
                    layout="position"
                    transition={isDragging ? { duration: 0 } : DOCK_SPRING}
                    style={{ position: 'relative', flexShrink: 0 }}
                    onDragStart={() => { setIsDragging(true); setHoveredApp(null); setPressedApp(null); }}
                    onDragEnd={handleDragEnd}
                    onDragOver={(e) => handleAppDragOver(e, app)}
                    onDragLeave={(e) => handleAppDragLeave(e, app)}
                    onDrop={handleAppDrop}
                    onContextMenu={(e) => handleContextMenu(e, app)}
                    onMouseDown={(e) => { if (e.button === 1) e.preventDefault(); }}
                    onAuxClick={(e) => handleMiddleClick(e, app)}
                    onWheel={(e) => handleAppWheel(e, app)}
                    onClick={(e) => {
                      e.stopPropagation();
                      if (!isDragging) handleAppClick(app);
                    }}
                  >
                    <motion.div
                      className={`dock-icon-wrapper ${dragHoveringApp === itemKey(app) ? 'dock-icon-drag-hover' : ''}`}
                      initial={ITEM_INITIAL}
                      animate={ITEM_ANIMATE}
                      exit={ITEM_EXIT}
                      transition={ITEM_ENTRY_TRANSITION}
                      onMouseEnter={() => { if (!contextMenu) setHoveredApp(itemKey(app)); }}
                      onMouseLeave={() => { if (!isPreviewHoveredRef.current) { setHoveredApp(null); setPressedApp(null); } }}
                    >
                
                <AnimatePresence>
                  {volumeHuds[itemKey(app)]?.visible && (
                    <motion.div
                      className="dock-app-volume-hud"
                      initial={{ opacity: 0, y: 6, scale: 0.92 }}
                      animate={{ opacity: 1, y: 0, scale: 1 }}
                      exit={{ opacity: 0, y: 3, scale: 0.95, transition: { duration: 0.1 } }}
                      transition={{ duration: 0.15, ease: [0.16, 1, 0.3, 1] }}
                    >
                      <div className="dock-app-volume-hud-header">
                        {volumeHuds[itemKey(app)].muted || volumeHuds[itemKey(app)].currentVol === 0 ? (
                          <VolumeX size={12} className="dock-app-volume-hud-icon muted" />
                        ) : volumeHuds[itemKey(app)].currentVol < 0.5 ? (
                          <Volume1 size={12} className="dock-app-volume-hud-icon" />
                        ) : (
                          <Volume2 size={12} className="dock-app-volume-hud-icon" />
                        )}
                        <span className="dock-app-volume-hud-pct">
                          {Math.round(volumeHuds[itemKey(app)].currentVol * 100)}%
                        </span>
                      </div>
                      <div className="dock-app-volume-hud-track">
                        <div
                          className={`dock-app-volume-hud-fill ${volumeHuds[itemKey(app)].muted ? 'muted' : ''}`}
                          style={{ width: `${Math.round(volumeHuds[itemKey(app)].currentVol * 100)}%` }}
                        />
                      </div>
                    </motion.div>
                  )}
                </AnimatePresence>

                <AnimatePresence>
                  {!activeVolumeKey && dockPreviewEnabled && !contextMenu && previewData && previewData.id === itemKey(app) && hoveredApp === itemKey(app) && (
                    <DockPreviewTooltip
                      app={app}
                      previews={previewData.previews}
                      onClosePreview={handleClosePreview}
                      renderMedia={renderAppIcon}
                      renderPreviewMedia={renderPreviewMedia}
                      onMouseEnter={() => { isPreviewHoveredRef.current = true; }}
                      onMouseLeave={() => { isPreviewHoveredRef.current = false; setHoveredApp(null); setPressedApp(null); }}
                    />
                  )}
                </AnimatePresence>
                
                {/* Fallback to text tooltip if previews are disabled, app isn't running, or preview failed to load */}
                {!contextMenu && (!dockPreviewEnabled || (dockPreviewEnabled && hoveredApp === itemKey(app) && !previewData)) && !volumeHuds[itemKey(app)]?.visible && (
                  <div className="tooltip">{localizeAppName(app.name)}</div>
                )}
                <motion.div 
                  className="dock-icon"
                  variants={iconVariants}
                  animate={pressedApp === itemKey(app) ? "tap" : (isDragging && !app.is_pinned ? "idle" : (hoveredApp === itemKey(app) && !isDragging ? "hover" : "idle"))}
                  whileDrag="drag"
                  onPointerDown={() => setPressedApp(itemKey(app))}
                  onPointerUp={() => setPressedApp(null)}
                  onPointerCancel={() => setPressedApp(null)}
                >
                  {renderAppIcon(app)}
                  {dockAttentionEnabled && (
                    <AttentionBadge
                      count={appUnreadCount(app)}
                      isFlashing={appAttention(app, attentionHwnds)}
                    />
                  )}
                </motion.div>
                  {app.is_running && <WindowDots count={app.all_hwnds?.length ?? 1} mode={windowIndicator} />}
                    </motion.div>
                  </Reorder.Item>
                ))}
              </Reorder.Group>

              {pinnedItems.length > 0 && unpinnedItems.length > 0 && (
                <motion.div
                  layout="position"
                  transition={isDragging ? { duration: 0 } : DOCK_SPRING}
                  className="dock-group-divider"
                />
              )}

              <Reorder.Group
                as="div"
                axis="x"
                layout
                values={unpinnedItems.map(itemKey)}
                onReorder={handleUnpinnedReorder}
                className="dock-reorder-group"
              >
                {unpinnedItems.map((app) => (
                  <Reorder.Item
                    as="div"
                    key={itemKey(app)}
                    value={itemKey(app)}
                    layout="position"
                    transition={isDragging ? { duration: 0 } : DOCK_SPRING}
                    style={{ position: 'relative', flexShrink: 0 }}
                      onDragStart={() => { setIsDragging(true); setHoveredApp(null); setPressedApp(null); }}
                      onDragEnd={handleUnpinnedDragEnd}
                      onDragOver={(e) => handleAppDragOver(e, app)}
                      onDragLeave={(e) => handleAppDragLeave(e, app)}
                      onDrop={handleAppDrop}
                      onContextMenu={(e) => handleContextMenu(e, app)}
                      onMouseDown={(e) => { if (e.button === 1) e.preventDefault(); }}
                      onAuxClick={(e) => handleMiddleClick(e, app)}
                      onWheel={(e) => handleAppWheel(e, app)}
                      onClick={(e) => {
                        e.stopPropagation();
                        if (!isDragging) handleAppClick(app);
                      }}
                    >
                      <motion.div
                        className={`dock-icon-wrapper ${dragHoveringApp === itemKey(app) ? 'dock-icon-drag-hover' : ''}`}
                        initial={ITEM_INITIAL}
                        animate={ITEM_ANIMATE}
                        exit={ITEM_EXIT}
                        transition={ITEM_ENTRY_TRANSITION}
                        onMouseEnter={() => { if (!contextMenu) setHoveredApp(itemKey(app)); }}
                        onMouseLeave={() => { if (!isPreviewHoveredRef.current) { setHoveredApp(null); setPressedApp(null); } }}
                      >
                        
                  <AnimatePresence>
                    {volumeHuds[itemKey(app)]?.visible && (
                      <motion.div
                        className="dock-app-volume-hud"
                        initial={{ opacity: 0, y: 6, scale: 0.92 }}
                        animate={{ opacity: 1, y: 0, scale: 1 }}
                        exit={{ opacity: 0, y: 3, scale: 0.95, transition: { duration: 0.1 } }}
                        transition={{ duration: 0.15, ease: [0.16, 1, 0.3, 1] }}
                      >
                        <div className="dock-app-volume-hud-header">
                          {volumeHuds[itemKey(app)].muted || volumeHuds[itemKey(app)].currentVol === 0 ? (
                            <VolumeX size={12} className="dock-app-volume-hud-icon muted" />
                          ) : volumeHuds[itemKey(app)].currentVol < 0.5 ? (
                            <Volume1 size={12} className="dock-app-volume-hud-icon" />
                          ) : (
                            <Volume2 size={12} className="dock-app-volume-hud-icon" />
                          )}
                          <span className="dock-app-volume-hud-pct">
                            {Math.round(volumeHuds[itemKey(app)].currentVol * 100)}%
                          </span>
                        </div>
                        <div className="dock-app-volume-hud-track">
                          <div
                            className={`dock-app-volume-hud-fill ${volumeHuds[itemKey(app)].muted ? 'muted' : ''}`}
                            style={{ width: `${Math.round(volumeHuds[itemKey(app)].currentVol * 100)}%` }}
                          />
                        </div>
                      </motion.div>
                    )}
                  </AnimatePresence>

                  <AnimatePresence>
                    {!activeVolumeKey && dockPreviewEnabled && !contextMenu && previewData && previewData.id === itemKey(app) && hoveredApp === itemKey(app) && (
                            <DockPreviewTooltip
                        app={app}
                        previews={previewData.previews}
                        onClosePreview={handleClosePreview}
                        renderMedia={renderAppIcon}
                        renderPreviewMedia={renderPreviewMedia}
                        onMouseEnter={() => { isPreviewHoveredRef.current = true; }}
                        onMouseLeave={() => { isPreviewHoveredRef.current = false; setHoveredApp(null); setPressedApp(null); }}
                      />
                          )}
                        </AnimatePresence>
                        {!contextMenu && (!dockPreviewEnabled || (dockPreviewEnabled && hoveredApp === itemKey(app) && !previewData)) && !volumeHuds[itemKey(app)]?.visible && (
                          <div className="tooltip">{localizeAppName(app.name)}</div>
                        )}
                        <motion.div
                          className="dock-icon"
                          variants={iconVariants}
                          animate={pressedApp === itemKey(app) ? "tap" : (isDragging && app.is_pinned ? "idle" : (hoveredApp === itemKey(app) && !isDragging ? "hover" : "idle"))}
                          whileDrag="drag"
                          onPointerDown={() => setPressedApp(itemKey(app))}
                          onPointerUp={() => setPressedApp(null)}
                          onPointerCancel={() => setPressedApp(null)}
                        >
                          {renderAppIcon(app)}
                          {dockAttentionEnabled && (
                            <AttentionBadge
                              count={appUnreadCount(app)}
                              isFlashing={appAttention(app, attentionHwnds)}
                            />
                          )}
                        </motion.div>
                        {app.is_running && <WindowDots count={app.all_hwnds?.length ?? 1} mode={windowIndicator} />}
                      </motion.div>
                    </Reorder.Item>
                  ))}
              </Reorder.Group>
              </>
              ) : (
              <Reorder.Group
                as="div"
                axis="x"
                layout
                values={mixedItems.map(itemKey)}
                onReorder={handleMixedReorder}
                className="dock-reorder-group"
              >
                {mixedItems.map((app) => (
                  <Reorder.Item
                    as="div"
                    key={itemKey(app)}
                    value={itemKey(app)}
                    layout="position"
                    transition={isDragging ? { duration: 0 } : DOCK_SPRING}
                    style={{ position: 'relative', flexShrink: 0 }}
                    onDragStart={() => { setIsDragging(true); setHoveredApp(null); setPressedApp(null); }}
                    onDragEnd={() => handleMixedDragEnd(app)}
                    onDragOver={(e) => handleAppDragOver(e, app)}
                    onDragLeave={(e) => handleAppDragLeave(e, app)}
                    onDrop={handleAppDrop}
                    onContextMenu={(e) => handleContextMenu(e, app)}
                    onMouseDown={(e) => { if (e.button === 1) e.preventDefault(); }}
                    onAuxClick={(e) => handleMiddleClick(e, app)}
                    onWheel={(e) => handleAppWheel(e, app)}
                    onClick={(e) => {
                      e.stopPropagation();
                      if (!isDragging) handleAppClick(app);
                    }}
                  >
                    <motion.div
                      className={`dock-icon-wrapper ${dragHoveringApp === itemKey(app) ? 'dock-icon-drag-hover' : ''}`}
                      initial={ITEM_INITIAL}
                      animate={ITEM_ANIMATE}
                      exit={ITEM_EXIT}
                      transition={ITEM_ENTRY_TRANSITION}
                      onMouseEnter={() => { if (!contextMenu) setHoveredApp(itemKey(app)); }}
                      onMouseLeave={() => { if (!isPreviewHoveredRef.current) { setHoveredApp(null); setPressedApp(null); } }}
                    >
                      
                <AnimatePresence>
                  {volumeHuds[itemKey(app)]?.visible && (
                    <motion.div
                      className="dock-app-volume-hud"
                      initial={{ opacity: 0, y: 6, scale: 0.92 }}
                      animate={{ opacity: 1, y: 0, scale: 1 }}
                      exit={{ opacity: 0, y: 3, scale: 0.95, transition: { duration: 0.1 } }}
                      transition={{ duration: 0.15, ease: [0.16, 1, 0.3, 1] }}
                    >
                      <div className="dock-app-volume-hud-header">
                        {volumeHuds[itemKey(app)].muted || volumeHuds[itemKey(app)].currentVol === 0 ? (
                          <VolumeX size={12} className="dock-app-volume-hud-icon muted" />
                        ) : volumeHuds[itemKey(app)].currentVol < 0.5 ? (
                          <Volume1 size={12} className="dock-app-volume-hud-icon" />
                        ) : (
                          <Volume2 size={12} className="dock-app-volume-hud-icon" />
                        )}
                        <span className="dock-app-volume-hud-pct">
                          {Math.round(volumeHuds[itemKey(app)].currentVol * 100)}%
                        </span>
                      </div>
                      <div className="dock-app-volume-hud-track">
                        <div
                          className={`dock-app-volume-hud-fill ${volumeHuds[itemKey(app)].muted ? 'muted' : ''}`}
                          style={{ width: `${Math.round(volumeHuds[itemKey(app)].currentVol * 100)}%` }}
                        />
                      </div>
                    </motion.div>
                  )}
                </AnimatePresence>

                <AnimatePresence>
                  {!activeVolumeKey && dockPreviewEnabled && !contextMenu && previewData && previewData.id === itemKey(app) && hoveredApp === itemKey(app) && (
                          <DockPreviewTooltip
                      app={app}
                      previews={previewData.previews}
                      onClosePreview={handleClosePreview}
                      renderMedia={renderAppIcon}
                      renderPreviewMedia={renderPreviewMedia}
                      onMouseEnter={() => { isPreviewHoveredRef.current = true; }}
                      onMouseLeave={() => { isPreviewHoveredRef.current = false; setHoveredApp(null); setPressedApp(null); }}
                    />
                        )}
                      </AnimatePresence>

                      {!contextMenu && (!dockPreviewEnabled || (dockPreviewEnabled && hoveredApp === itemKey(app) && !previewData)) && !volumeHuds[itemKey(app)]?.visible && (
                        <div className="tooltip">{localizeAppName(app.name)}</div>
                      )}
                      <motion.div
                        className="dock-icon"
                        variants={iconVariants}
                        animate={pressedApp === itemKey(app) ? "tap" : (hoveredApp === itemKey(app) && !isDragging ? "hover" : "idle")}
                        whileDrag="drag"
                        onPointerDown={() => setPressedApp(itemKey(app))}
                        onPointerUp={() => setPressedApp(null)}
                        onPointerCancel={() => setPressedApp(null)}
                      >
                        {renderAppIcon(app)}
                        {dockAttentionEnabled && (
                          <AttentionBadge
                            count={appUnreadCount(app)}
                            isFlashing={appAttention(app, attentionHwnds)}
                          />
                        )}
                      </motion.div>
                      {app.is_running && <WindowDots count={app.all_hwnds?.length ?? 1} mode={windowIndicator} />}
                    </motion.div>
                  </Reorder.Item>
                ))}
              </Reorder.Group>
              )}

              {dockShowDesktop && (
                <div
                  className="dock-show-desktop"
                  title={t('dockmenu.showDesktop')}
                  onClick={(e) => { e.stopPropagation(); invoke('show_desktop').catch(() => { }); }}
                />
              )}
            </motion.div>
          )}
        </AnimatePresence>
      </motion.div>
    </div>

      {contextMenu && (
        <div
          style={{
            position: 'fixed',
            left: Math.min(
              Math.max(8, contextMenu.x - (contextMenuWidth || 160 * scale) / 2),
              Math.max(8, window.innerWidth - (contextMenuWidth || 160 * scale) - 8),
            ),
            top: Math.max(8, contextMenu.y - 10 - (contextMenuHeight || (contextMenu.app ? 200 : 100) * scale)),
            zIndex: 9999,
          }}
        >
        <div
          ref={menuRef}
          className="context-menu"
          style={{
            position: 'static',
            zoom: scale
          }}
          onClick={(e) => e.stopPropagation()}
        >
          {contextMenu.extraKind === 'recycle' ? (
            <>
              <div className="menu-item" onClick={() => {
                invoke('open_dock_extra', { target: 'recycle' }).catch(() => {});
                closeMenu();
              }}>
                {t("dockmenu.open")}
              </div>
              <div
                className={`menu-item ${recycleStatus?.is_empty ? 'disabled' : ''}`}
                style={recycleStatus?.is_empty ? { opacity: 0.5, pointerEvents: 'none' } : undefined}
                onClick={() => {
                  if (!recycleStatus?.is_empty) {
                    invoke('empty_recycle_bin', { confirm: true })
                      .then(() => refreshRecycleStatus())
                      .catch(() => {});
                  }
                  closeMenu();
                }}
              >
                {t("dockmenu.emptyRecycle")}
              </div>
            </>
          ) : contextMenu.extraKind === 'drive' ? (
            <>
              <div className="menu-item" onClick={() => {
                if (contextMenu.driveLetter) {
                  invoke('open_dock_extra', { target: contextMenu.driveLetter }).catch(() => {});
                }
                closeMenu();
              }}>
                {t("dockmenu.open")}
              </div>
              {contextMenu.isRemovable && (
                <div className="menu-item" onClick={() => {
                  if (contextMenu.driveLetter) {
                    invoke('eject_drive', { letter: contextMenu.driveLetter })
                      .then(() => {
                        refreshDriveDetails();
                      })
                      .catch((err) => console.error('Failed to eject drive:', err));
                  }
                  closeMenu();
                }}>
                  {t("dockmenu.ejectDrive")}
                </div>
              )}
            </>
          ) : contextMenu.app ? (
            <>
              {contextMenu.app.is_running && contextMenu.app.path !== 'start' && (
                <>
                  <div className="menu-item" onClick={() => { handleNewInstance(contextMenu.app!); closeMenu(); }}>
                    {t("dockmenu.newInstance")}
                  </div>
                  <div className="menu-divider" />
                </>
              )}
              <div className="menu-item" onClick={() => togglePin(contextMenu.app!)}>
                {contextMenu.app.is_pinned ? t("dockmenu.unpin") : t("dockmenu.pin")}
              </div>
              {contextMenu.app.is_running && !contextMenu.app.is_pinned && contextMenu.app.path !== 'start' && (
                <div className="menu-item" onClick={() => { handleHideApp(contextMenu.app!); closeMenu(); }}>
                  {t("dockmenu.hideApp")}
                </div>
              )}
              {contextMenu.app.is_pinned && contextMenu.app.path !== 'start' && (
                <>
                  <div className="menu-divider" />
                  <div className="menu-item" onClick={() => {
                    const isHost = isBrowserHost(contextMenu.app!.path);
                    const ck = isHost ? `${contextMenu.app!.path}:${contextMenu.app!.name.toLowerCase()}` : contextMenu.app!.path;
                    iconPickerTargetRef.current = ck;
                    closeMenu();
                    setTimeout(() => {
                      document.getElementById('icon-file-input')?.click();
                    }, 50);
                  }}>
                    {t("dockmenu.changeIcon")}
                  </div>
                  {(() => {
                    const isHost = isBrowserHost(contextMenu.app!.path);
                    const ck = isHost ? `${contextMenu.app!.path}:${contextMenu.app!.name.toLowerCase()}` : contextMenu.app!.path;
                    return customIcons[ck] ? (
                      <div className="menu-item" onClick={() => {
                        handleRemoveCustomIcon(contextMenu.app!);
                        closeMenu();
                      }}>
                        {t("dockmenu.resetIcon")}
                      </div>
                    ) : null;
                  })()}
                </>
              )}
              <div className="menu-divider" />
              <div className="menu-item" onClick={() => { setShowAddPopup(true); closeMenu(); }}>
                {t("dockmenu.addApp")}
              </div>
              {fernOptionsMenu()}
              {contextMenu.app.is_running && (
                <>
                  <div className="menu-divider" />
                  <div className="menu-item" onClick={async () => {
                    if (contextMenu.app?.hwnd) {
                      await invoke('close_app', {
                        hwnd: contextMenu.app.hwnd,
                        allHwnds: contextMenu.app.all_hwnds?.map(([h]) => h) ?? null,
                      });
                      closeMenu();
                    }
                  }}>
                    {t("dockmenu.closeWindow")}
                  </div>
                  {showEndTask && (
                    <div className="menu-item quit" onClick={async () => {
                      if (contextMenu.app?.hwnd) {
                        await invoke('end_task', {
                          hwnd: contextMenu.app.hwnd,
                          allHwnds: contextMenu.app.all_hwnds?.map(([h]) => h) ?? null,
                        });
                        closeMenu();
                      }
                    }}>
                      {t("dockmenu.endTask")}
                    </div>
                  )}
                </>
              )}
            </>
          ) : (
            <>
              <div className="menu-item" onClick={() => { setShowAddPopup(true); closeMenu(); }}>
                {t("dockmenu.addApp")}
              </div>
              {fernOptionsMenu()}
            </>
          )}
        </div>
        </div>
      )}

      <input
        id="icon-file-input"
        type="file"
        accept=".png,.ico,.jpg,.jpeg,.svg,.bmp"
        style={{ display: 'none' }}
        onChange={handleIconFileSelect}
      />

      <AnimatePresence>
        {showAddPopup && (
          <AddAppPopup 
            containerRef={popupRef}
            onClose={closePopup} 
            onAdd={(app: AppInfo) => { togglePin(app); closePopup(); }}
            scale={scale}
            runningApps={activeApps}
            pinned={pinnedApps}
            bottom={popupBottom}
            isPickingProgramRef={isPickingProgramRef}
          />
        )}
      </AnimatePresence>

      <AnimatePresence>
        {toast && (
          <motion.div
            className="dock-toast"
            initial={{ opacity: 0, y: 10 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0, y: 10 }}
            transition={{ duration: 0.2 }}
            style={{ zoom: scale }}
          >
            {toast}
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
});

function AddAppPopup({
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
  const [apps, setApps] = useState<AppInfo[]>([]);

  const handleBrowse = async () => {
    if (isPickingProgramRef) isPickingProgramRef.current = true;
    try {
      const picked = await invoke<AppInfo | null>("pick_program");
      if (picked) {
        onAdd(picked);
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
  const [loading, setLoading] = useState(true);
  const [listIcons, setListIcons] = useState<Record<string, string>>({});
  const [selectedIndex, setSelectedIndex] = useState(0);
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
    for (const a of apps) {
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
  }, [apps, debouncedSearch, pinned, runningSuggestions]);

  // Flat selectable list; section headers are not selectable.
  const flat = useMemo(() => sections.flatMap((s) => s.items), [sections]);
  const flatIndex = useMemo(() => new Map(flat.map((a, i) => [a, i])), [flat]);

  // Keep selection in range as results narrow.
  useEffect(() => {
    setSelectedIndex((i) => Math.min(i, Math.max(0, flat.length - 1)));
  }, [flat.length]);

  // Scroll on keyboard nav only, doing it on mount yanks the list.
  useEffect(() => {
    if (!mountedRef.current) {
      mountedRef.current = true;
      return;
    }
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
        setApps(res.sort((a, b) => a.name.localeCompare(b.name)));
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
        setSelectedIndex((i) => Math.min(i + 1, flat.length - 1));
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
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
    const handleMouseDown = (e: MouseEvent) => {
      const popup = containerRef.current;
      if (popup && !popup.contains(e.target as Node)) {
        onClose();
      }
    };
    const handleBlur = () => onClose();
    window.addEventListener("keydown", handleKeyDown);
    window.addEventListener("blur", handleBlur);
    document.addEventListener("mousedown", handleMouseDown, true);
    return () => {
      window.removeEventListener("keydown", handleKeyDown);
      window.removeEventListener("blur", handleBlur);
      document.removeEventListener("mousedown", handleMouseDown, true);
    };
  }, [onClose, containerRef, flat, selectedIndex, onAdd, pinned]);

  // Fetch icons in parallel and apply them as one batch, one at a time looked like the list
  // shifting. Boxes are fixed size and images fade in.
  useEffect(() => {
    let active = true;
    const targets = flat.slice(0, 25).filter((a) => !listIcons[a.path]);
    if (targets.length === 0) return;
    Promise.all(
      targets.map(async (app) => {
        try {
          const icon = await invoke<string | null>("get_app_icon", { path: app.path });
          return [app.path, icon] as const;
        } catch (err) {
          console.error(err);
          return [app.path, null] as const;
        }
      })
    ).then((pairs) => {
      if (!active) return;
      const batch: Record<string, string> = {};
      for (const [path, icon] of pairs) if (icon) batch[path] = icon;
      if (Object.keys(batch).length > 0) setListIcons((prev) => ({ ...prev, ...batch }));
    });
    return () => {
      active = false;
    };
  }, [flat, listIcons]);

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
                  const icon = listIcons[app.path];
                  const alreadyPinned = pinned.some((p) => isSameApp(p, app));
                  return (
                    <div
                      key={`${app.path}::${app.name}`}
                      data-idx={gi}
                      className={`popup-app-row${gi === selectedIndex ? " selected" : ""}${alreadyPinned ? " is-pinned" : ""}`}
                      onClick={() => (alreadyPinned ? onClose() : onAdd(app))}
                      onMouseEnter={() => setSelectedIndex(gi)}
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

export default Dock;
