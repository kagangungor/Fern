import { useEffect, useState } from "react";
import { Megaphone, X } from "lucide-react";
import { useT } from "../i18n";

export interface AnnouncementItem {
  id: string;
  level?: "info" | "warn" | "critical";
  title: string;
  body: string;
  url?: string;
}

const DISMISSED_KEY = "fern-dismissed-announcements";

function readDismissed(): Set<string> {
  try {
    const raw = localStorage.getItem(DISMISSED_KEY);
    const parsed = raw ? JSON.parse(raw) : [];
    return new Set(Array.isArray(parsed) ? parsed.filter((x): x is string => typeof x === "string") : []);
  } catch {
    return new Set();
  }
}

function saveDismissed(ids: Set<string>) {
  try {
    localStorage.setItem(DISMISSED_KEY, JSON.stringify([...ids]));
  } catch { /* storage unavailable */ }
}

export function AnnouncementsBanner() {
  const t = useT();
  const [items, setItems] = useState<AnnouncementItem[]>([]);
  const [dismissed, setDismissed] = useState<Set<string>>(() => readDismissed());

  useEffect(() => {
    let cancelled = false;
    fetch("/announcements.json")
      .then((res) => (res.ok ? res.json() : []))
      .then((data) => {
        if (!cancelled && Array.isArray(data)) {
          setItems(data.filter((x): x is AnnouncementItem => Boolean(x?.id && x?.title && x?.body)));
        }
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, []);

  const visible = items.filter((item) => !dismissed.has(item.id));
  if (visible.length === 0) return null;

  const dismiss = (id: string) => {
    setDismissed((prev) => {
      const next = new Set(prev);
      next.add(id);
      saveDismissed(next);
      return next;
    });
  };

  return (
    <div className="announcements-stack">
      {visible.map((item) => (
        <div key={item.id} className={`announcement-card ${item.level ?? "info"}`}>
          <div className="announcement-icon">
            <Megaphone size={14} />
          </div>
          <div className="announcement-copy">
            <strong>{item.title}</strong>
            <span>{item.body}</span>
          </div>
          <button
            type="button"
            className="announcement-close"
            aria-label={t("announcements.dismiss")}
            onClick={() => dismiss(item.id)}
          >
            <X size={13} />
          </button>
        </div>
      ))}
    </div>
  );
}
