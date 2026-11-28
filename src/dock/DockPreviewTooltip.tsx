import { useState, useEffect, useMemo, useRef } from 'react';
import { motion, useIsPresent } from 'framer-motion';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useT, localizeAppName } from '../i18n';
import type { DockPreviewTooltipProps } from './types';
import {
  commProfileForApp,
  DEFAULT_COMM_TOGGLE,
  commToggleStateByHwnd,
  nextCommToggleState,
  CommActionIcon,
} from './commThumbar';

let nextLivePreviewSessionId = 0;

export function DockPreviewTooltip({
  app,
  previews,
  capturePending = false,
  onClosePreview,
  onSelectPreview,
  renderMedia,
  renderPreviewMedia,
  onMouseEnter,
  onMouseLeave,
}: DockPreviewTooltipProps) {
  const t = useT();
  const commProfile = useMemo(() => commProfileForApp(app), [app]);
  const [privacy, setPrivacy] = useState<{ mic_active: boolean; cam_active: boolean; active_apps: string[] } | null>(null);
  const [, setToggleTick] = useState(0);

  useEffect(() => {
    if (!commProfile) {
      setPrivacy(null);
      return;
    }
    let alive = true;
    invoke<{ mic_active: boolean; cam_active: boolean; active_apps: string[] }>('get_privacy_status')
      .then((status) => {
        if (alive) setPrivacy(status);
      })
      .catch(() => {});
    const unlisten = listen<{ mic_active: boolean; cam_active: boolean; active_apps: string[] }>(
      'privacy-status-changed',
      (event) => {
        if (alive) setPrivacy(event.payload);
      }
    );
    return () => {
      alive = false;
      unlisten.then((fn) => fn()).catch(() => {});
    };
  }, [commProfile]);

  const showCommActions = Boolean(
    commProfile &&
      privacy &&
      (privacy.mic_active || privacy.cam_active) &&
      privacy.active_apps.some((name) =>
        commProfile.matchTokens.some((tok) => name.toLowerCase().includes(tok))
      )
  );

  useEffect(() => {
    if (privacy !== null && !showCommActions) {
      for (const p of previews) {
        commToggleStateByHwnd.delete(p.hwnd);
      }
    }
  }, [privacy, showCommActions, previews]);

  const isPresent = useIsPresent();
  const sessionIdRef = useRef<number>(0);
  if (sessionIdRef.current === 0) {
    sessionIdRef.current = ++nextLivePreviewSessionId;
  }
  const surfaceRefs = useRef<Map<number, HTMLDivElement>>(new Map());
  const [fallbackLiveHwnds, setFallbackLiveHwnds] = useState<Set<number>>(() => new Set());
  const hasFallbackLiveRef = useRef(false);
  hasFallbackLiveRef.current = fallbackLiveHwnds.size > 0;
  const missingHwnds = useMemo(
    () => (capturePending ? [] : previews.filter((p) => !p.image).map((p) => p.hwnd)),
    [capturePending, previews]
  );
  const missingKey = missingHwnds.join(',');

  // Clear only this preview session's DWM thumbnails on unmount so an exiting tooltip
  // can never wipe out a newer preview's live thumbnails.
  useEffect(() => {
    const sid = sessionIdRef.current;
    return () => {
      invoke('clear_live_thumbnails', { sessionId: sid }).catch(() => {});
    };
  }, []);

  // Sync DWM fallback opacity fade-out with the 120ms framer-motion exit animation.
  useEffect(() => {
    if (isPresent || !hasFallbackLiveRef.current) return;
    const sid = sessionIdRef.current;
    let cancelled = false;
    let rafId = 0;
    let lastSent = 255;
    const start = performance.now();
    const duration = 120;
    const step = (now: number) => {
      if (cancelled) return;
      const progress = Math.min(1, (now - start) / duration);
      const opacity = Math.round(255 * (1 - progress));
      if (opacity !== lastSent) {
        lastSent = opacity;
        invoke('set_live_thumbnails_opacity', { opacity, sessionId: sid }).catch(() => {});
      }
      if (progress < 1) {
        rafId = requestAnimationFrame(step);
      } else {
        invoke('clear_live_thumbnails', { sessionId: sid }).catch(() => {});
      }
    };
    rafId = requestAnimationFrame(step);
    return () => {
      cancelled = true;
      cancelAnimationFrame(rafId);
    };
  }, [isPresent]);

  const previewMediaNode = renderPreviewMedia ? renderPreviewMedia(app) : null;
  const hasPreviewMedia = Boolean(previewMediaNode);

  useEffect(() => {
    if (!isPresent) return;
    const sid = sessionIdRef.current;
    if (!missingKey) {
      setFallbackLiveHwnds(new Set());
      invoke('clear_live_thumbnails', { sessionId: sid }).catch(() => {});
      return;
    }
    let cancelled = false;
    let t1: ReturnType<typeof setTimeout> | null = null;
    let t2: ReturnType<typeof setTimeout> | null = null;
    const syncFallback = async () => {
      if (cancelled) return;
      const dpr = window.devicePixelRatio || 1;
      const targets: { hwnd: number; x: number; y: number; width: number; height: number }[] = [];
      for (const hwnd of missingHwnds) {
        const el = surfaceRefs.current.get(hwnd);
        if (!el) continue;
        const r = el.getBoundingClientRect();
        if (r.width > 0 && r.height > 0) {
          targets.push({
            hwnd,
            x: Math.round(r.left * dpr),
            y: Math.round(r.top * dpr),
            width: Math.round(r.width * dpr),
            height: Math.round(r.height * dpr),
          });
        }
      }
      if (targets.length === 0) return;
      try {
        const active = await invoke<number[]>('set_live_thumbnails', {
          targets,
          clientAreaOnly: false,
          sessionId: sid,
        });
        if (!cancelled && active) setFallbackLiveHwnds(new Set(active));
      } catch {}
    };
    const rafId = requestAnimationFrame(() => {
      syncFallback();
      t1 = setTimeout(syncFallback, 80);
      t2 = setTimeout(syncFallback, 200);
    });
    return () => {
      cancelled = true;
      cancelAnimationFrame(rafId);
      if (t1) clearTimeout(t1);
      if (t2) clearTimeout(t2);
    };
  }, [missingKey, showCommActions, hasPreviewMedia, isPresent]);

  return (
    <motion.div
      className={`preview-tooltip ${previews.length > 1 ? 'multi' : ''}`}
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      transition={{ duration: 0.12 }}
      onClick={(e) => e.stopPropagation()}
      onMouseEnter={onMouseEnter}
      onMouseLeave={onMouseLeave}
    >
      <div className="preview-items">
        {previews.map((prev, idx) => (
            <div
              key={prev.hwnd}
              className="preview-item"
              onClick={() => {
                invoke('clear_live_thumbnails').catch(() => {});
                onSelectPreview(prev.hwnd);
              }}
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
                    invoke('clear_live_thumbnails').catch(() => {});
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
                className="preview-surface"
                ref={(el) => {
                  if (el) surfaceRefs.current.set(prev.hwnd, el);
                  else surfaceRefs.current.delete(prev.hwnd);
                }}
              >
                {prev.image ? (
                  <img
                    src={prev.image}
                    alt={`Preview ${idx}`}
                    className="preview-thumb-img"
                  />
                ) : (
                  <div
                    className={`preview-item-icon-fallback${fallbackLiveHwnds.has(prev.hwnd) ? ' is-live' : capturePending ? ' is-pending' : ''}`}
                  >
                    {renderMedia(app)}
                  </div>
                )}
              </div>
              {showCommActions && commProfile && (
                <div
                  className="preview-thumbar-actions"
                  onClick={(e) => e.stopPropagation()}
                >
                  {commProfile.actions.map((action) => {
                    const toggleState = commToggleStateByHwnd.get(prev.hwnd) ?? {
                      ...DEFAULT_COMM_TOGGLE,
                      cameraOn: Boolean(privacy?.cam_active),
                    };
                    const isCameraActive = action.kind === 'camera' && toggleState.cameraOn;
                    const isMuted =
                      (action.kind === 'mic' && toggleState.micMuted) ||
                      (action.kind === 'deafen' && toggleState.deafened);
                    const iconToggled = action.kind === 'camera' ? isCameraActive : isMuted;
                    return (
                      <button
                        key={action.index}
                        type="button"
                        className={`preview-thumbar-btn${isCameraActive ? ' is-active' : ''}${isMuted ? ' is-muted' : ''}${action.danger ? ' is-danger' : ''}`}
                        title={t(action.labelKey)}
                        aria-label={t(action.labelKey)}
                        aria-pressed={action.kind === 'disconnect' ? undefined : iconToggled}
                        onClick={(e) => {
                          e.stopPropagation();
                          const prevToggle = commToggleStateByHwnd.get(prev.hwnd);
                          if (action.kind === 'disconnect') {
                            commToggleStateByHwnd.delete(prev.hwnd);
                          } else {
                            commToggleStateByHwnd.set(
                              prev.hwnd,
                              nextCommToggleState(toggleState, action.kind)
                            );
                          }
                          setToggleTick((n) => n + 1);
                          invoke('trigger_thumbar_button', {
                            hwnd: prev.hwnd,
                            buttonIndex: action.index,
                          }).catch(() => {
                            if (prevToggle) commToggleStateByHwnd.set(prev.hwnd, prevToggle);
                            else commToggleStateByHwnd.delete(prev.hwnd);
                            setToggleTick((n) => n + 1);
                          });
                        }}
                      >
                        <CommActionIcon kind={action.kind} toggled={iconToggled} />
                      </button>
                    );
                  })}
                </div>
              )}
              {previews.length === 1 && previewMediaNode}
            </div>
        ))}
      </div>
      {previews.length > 1 && previewMediaNode}
    </motion.div>
  );
}
