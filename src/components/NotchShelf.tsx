import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { emit, listen } from "@tauri-apps/api/event";
import { startDrag } from "@crabnebula/tauri-plugin-drag";
import { AnimatePresence, Reorder } from "framer-motion";
import { Eye, Trash2 } from "lucide-react";
import { useT } from "../i18n";

export type ShelfItem = {
  id: string;
  kind: "file" | "text";
  path?: string;
  name: string;
  text?: string;
  thumb?: string | null;
  preview?: string | null;
};

const STORAGE_KEY = "fern-shelf-items";
const MAX_ITEMS = 10;

const baseName = (path: string) => path.split(/[\\/]/).filter(Boolean).pop() ?? path;

function readSaved(): ShelfItem[] {
  try {
    const parsed = JSON.parse(localStorage.getItem(STORAGE_KEY) || "[]");
    return Array.isArray(parsed) ? parsed.map((i: ShelfItem) => ({ ...i, thumb: null, preview: null })) : [];
  } catch {
    return [];
  }
}

type ShelfCallbacks = {
  onDragEnter: () => void;
  onDragLeave: () => void;
  onDrop: () => void;
};

export function useShelf(windowLabel: string, enabled: boolean, persist: boolean, cb: ShelfCallbacks) {
  const [items, setItems] = useState<ShelfItem[]>(() => (persist ? readSaved() : []));
  const cbRef = useRef(cb);
  cbRef.current = cb;

  useEffect(() => {
    if (!persist) {
      try { localStorage.removeItem(STORAGE_KEY); } catch { /* storage unavailable */ }
      return;
    }
    try { localStorage.setItem(STORAGE_KEY, JSON.stringify(items.map(({ thumb, preview, ...rest }) => rest))); } catch { /* storage unavailable */ }
  }, [items, persist]);

  useEffect(() => {
    if (!persist) return;
    const files = items.filter((i) => i.kind === "file" && i.path).map((i) => i.path as string);
    if (files.length === 0) return;
    invoke<string[]>("shelf_prune", { paths: files }).then((alive) => {
      const keep = new Set(alive);
      setItems((prev) => prev.filter((i) => i.kind !== "file" || keep.has(i.path as string)));
    }).catch(() => {});
  }, []);

  // Explorer has no cached thumbnail for a fresh file yet (videos especially), so ask again a few times.
  const loadThumb = useCallback((id: string, path: string, attempt = 0) => {
    invoke<string | null>("shelf_thumbnail", { path }).then((thumb) => {
      if (thumb) {
        setItems((prev) => prev.map((i) => (i.id === id ? { ...i, thumb } : i)));
        return;
      }
      invoke<string | null>("shelf_text_preview", { path }).then((preview) => {
        if (preview) {
          setItems((prev) => prev.map((i) => (i.id === id ? { ...i, preview } : i)));
        } else if (attempt < 3) {
          setTimeout(() => loadThumb(id, path, attempt + 1), 600 * (attempt + 1));
        }
      }).catch(() => {});
    }).catch(() => {});
  }, []);

  useEffect(() => {
    items.forEach((i) => {
      if (i.kind === "file" && i.path && !i.thumb && !i.preview) loadThumb(i.id, i.path);
    });
  }, []);

  const add = useCallback((paths?: string[], text?: string) => {
    const fresh: ShelfItem[] = [];
    if (paths) {
      for (const path of paths) {
        fresh.push({ id: `f-${Date.now()}-${fresh.length}-${path}`, kind: "file", path, name: baseName(path) });
      }
    } else if (text) {
      fresh.push({ id: `t-${Date.now()}`, kind: "text", name: text.trim().slice(0, 40), text });
    }
    if (fresh.length === 0) return;
    setItems((prev) => {
      const taken = new Set(fresh.filter((i) => i.path).map((i) => i.path));
      const rest = prev.filter((i) => !(i.path && taken.has(i.path)));
      return [...fresh, ...rest].slice(0, MAX_ITEMS);
    });
    fresh.forEach((i) => { if (i.path) loadThumb(i.id, i.path); });
  }, [loadThumb]);

  useEffect(() => {
    if (!enabled || !windowLabel) return;
    const isNotch = windowLabel === "main" || windowLabel.startsWith("main-");
    if (!isNotch) return;
    invoke("setup_shelf_drop").catch(() => {});

    const unlistens: Promise<() => void>[] = [
      listen<{ label: string }>("shelf-drag-enter", (e) => {
        if (e.payload.label === windowLabel) {
          cbRef.current.onDragEnter();
        }
      }),
      listen<{ label: string }>("shelf-drag-leave", (e) => {
        if (e.payload.label === windowLabel) {
          cbRef.current.onDragLeave();
        }
      }),
      listen<{ label: string; paths?: string[]; text?: string }>("shelf-drag-drop", (e) => {
        add(e.payload.paths, e.payload.text);
        if (e.payload.label === windowLabel) {
          emit("shelf-item-added").catch(() => {});
          cbRef.current.onDrop();
        }
      }),
    ];
    return () => { unlistens.forEach((u) => u.then((fn) => fn())); };
  }, [enabled, windowLabel, add]);

  const remove = useCallback((id: string) => setItems((prev) => prev.filter((i) => i.id !== id)), []);
  const clear = useCallback(() => setItems([]), []);

  return { items, reorder: setItems, remove, clear };
}

type Props = {
  items: ShelfItem[];
  onReorder: (items: ShelfItem[]) => void;
  onRemove: (id: string) => void;
  onClear: () => void;
  onDragOutStart: () => void;
  onDragOutEnd: () => void;
};

const CARD_SPRING = { type: "spring", stiffness: 520, damping: 34 } as const;

const extOf = (name: string) => {
  const dot = name.lastIndexOf(".");
  return dot > 0 && name.length - dot <= 5 ? name.slice(dot + 1).toUpperCase() : "";
};

const isTextItem = (item: ShelfItem) => {
  if (item.kind === "text") return true;
  if (!item.name) return false;
  return /\.(txt|md|markdown|json|log|js|ts|tsx|jsx|rs|css|html|xml|toml|yaml|yml|ini|cfg|csv|py|sh|bat|ps1)$/i.test(item.name);
};

export function NotchShelf({ items, onReorder, onRemove, onClear, onDragOutStart, onDragOutEnd }: Props) {
  const t = useT();
  const [copiedId, setCopiedId] = useState<string | null>(null);
  const [resetKey, setResetKey] = useState(0);
  const [previewItem, setPreviewItem] = useState<{ id: string; name: string; content: string } | null>(null);
  const [previewCopied, setPreviewCopied] = useState(false);
  const panelRef = useRef<HTMLDivElement>(null);
  const dragState = useRef<{ out: boolean } | null>(null);
  const dragIcon = useRef<string | null>(null);

  const handlePreview = async (e: React.MouseEvent, item: ShelfItem) => {
    e.stopPropagation();
    if (previewItem?.id === item.id) {
      setPreviewItem(null);
      return;
    }
    if (item.kind === "text") {
      setPreviewItem({ id: item.id, name: item.name, content: item.text ?? "" });
      return;
    }
    if (item.kind === "file" && item.path) {
      try {
        const content = await invoke<string>("shelf_read_text_preview", { path: item.path });
        setPreviewItem({ id: item.id, name: item.name, content });
      } catch (err) {
        console.error("Failed to read text preview:", err);
      }
    }
  };

  useEffect(() => {
    invoke<string | null>("shelf_drag_icon").then((icon) => { dragIcon.current = icon; }).catch(() => {});
  }, []);

  const beginDrag = async (item: ShelfItem) => {
    const icon = item.thumb ?? dragIcon.current;
    if (!icon) return;
    const path = item.kind === "file" ? item.path : await invoke<string | null>("shelf_text_file", { text: item.text ?? "" });
    if (!path) return;
    onDragOutStart();
    try {
      await startDrag({ item: [path], icon, mode: "copy" });
    } catch { /* drag was cancelled or refused */ }
    setResetKey((k) => k + 1);
    onDragOutEnd();
  };

  const activate = (item: ShelfItem) => {
    if (item.kind === "file" && item.path) {
      invoke("shelf_open", { path: item.path }).catch(() => {});
    } else if (item.text) {
      navigator.clipboard.writeText(item.text).then(() => {
        setCopiedId(item.id);
        setTimeout(() => setCopiedId((cur) => (cur === item.id ? null : cur)), 1200);
      }).catch(() => {});
    }
  };

  const previewText = (item: ShelfItem) => item.preview ?? (item.kind === "text" ? item.text?.slice(0, 160) : null);

  return (
    <div ref={panelRef} className="shelf-panel" onClick={(e) => e.stopPropagation()}>
      {previewItem && (
        <div
          className="shelf-preview-card"
          onPointerDown={(e) => e.stopPropagation()}
          onClick={(e) => e.stopPropagation()}
        >
          <div className="shelf-preview-header">
            <span className="shelf-preview-title">{previewItem.name}</span>
            <div className="shelf-preview-actions">
              <button
                className="shelf-preview-btn"
                onClick={() => {
                  navigator.clipboard.writeText(previewItem.content);
                  setPreviewCopied(true);
                  setTimeout(() => setPreviewCopied(false), 1200);
                }}
                title={t("shelf.copyContent")}
              >
                {previewCopied ? t("shelf.copied") : t("shelf.copyContent")}
              </button>
              <button
                className="shelf-preview-close"
                onClick={() => setPreviewItem(null)}
                title={t("shelf.closePreview")}
              >
                ×
              </button>
            </div>
          </div>
          <pre className="shelf-preview-body">{previewItem.content}</pre>
        </div>
      )}
      {items.length === 0 ? (
        <div className="shelf-empty">{t("shelf.empty")}</div>
      ) : (
        <>
          <Reorder.Group key={resetKey} as="div" axis="x" values={items} onReorder={onReorder} className={`shelf-row${items.length <= 4 ? " fits" : ""}`}>
            <AnimatePresence initial={false}>
              {items.map((item) => {
                const text = item.thumb ? null : previewText(item);
                const ext = item.kind === "file" ? extOf(item.name) : "";
                return (
                  <Reorder.Item
                    key={item.id}
                    as="div"
                    value={item}
                    className={`shelf-item ${item.kind}`}
                    title={item.kind === "text" ? item.text : item.path}
                    initial={{ opacity: 0, scale: 0.5, y: -10 }}
                    animate={{ opacity: 1, scale: 1, y: 0 }}
                    exit={{ opacity: 0, scale: 0.5, transition: { duration: 0.12 } }}
                    whileDrag={{ scale: 1.1, zIndex: 5 }}
                    transition={CARD_SPRING}
                    onDragStart={() => { dragState.current = { out: false }; }}
                    onDrag={(_, info) => {
                      const state = dragState.current;
                      const rect = panelRef.current?.getBoundingClientRect();
                      if (!state || state.out || !rect) return;
                      const { x, y } = info.point;
                      if (x < rect.left || x > rect.right || y > rect.bottom || y < rect.top - 20) {
                        state.out = true;
                        beginDrag(item);
                      }
                    }}
                    onDragEnd={() => { dragState.current = null; }}
                    onTap={() => { if (item.kind === "text") activate(item); }}
                    onDoubleClick={() => { if (item.kind === "file") activate(item); }}
                  >
                    <div className="shelf-thumb">
                      {item.thumb ? (
                        <img src={item.thumb} alt="" draggable={false} />
                      ) : text ? (
                        <pre className={`shelf-preview${text.length < 24 ? " short" : ""}`}>{text}</pre>
                      ) : (
                        <span className="shelf-glyph">{item.kind === "text" ? "T" : "▤"}</span>
                      )}
                      {ext && !item.thumb && <span className="shelf-ext">{ext}</span>}
                    </div>
                    <span className="shelf-name">{copiedId === item.id ? t("shelf.copied") : item.name}</span>
                    {isTextItem(item) && (
                      <button
                        className="shelf-preview-trigger"
                        title={t("shelf.preview")}
                        onPointerDown={(e) => e.stopPropagation()}
                        onMouseDown={(e) => e.stopPropagation()}
                        onMouseUp={(e) => e.stopPropagation()}
                        onClick={(e) => handlePreview(e, item)}
                      >
                        <Eye size={10} strokeWidth={2.4} />
                      </button>
                    )}
                    <button
                      className="shelf-remove"
                      title={t("shelf.remove")}
                      onPointerDown={(e) => e.stopPropagation()}
                      onMouseDown={(e) => e.stopPropagation()}
                      onMouseUp={(e) => e.stopPropagation()}
                      onClick={(e) => {
                        e.stopPropagation();
                        if (previewItem?.id === item.id) setPreviewItem(null);
                        onRemove(item.id);
                      }}
                    >
                      ×
                    </button>
                  </Reorder.Item>
                );
              })}
            </AnimatePresence>
          </Reorder.Group>
          <button
            className="shelf-clear"
            onClick={() => {
              setPreviewItem(null);
              onClear();
            }}
          >
            <Trash2 size={12} strokeWidth={2} />
            {t("shelf.clear")}
          </button>
        </>
      )}
    </div>
  );
}
