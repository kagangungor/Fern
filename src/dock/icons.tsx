import { useState } from 'react';

export const solidIconCache = new Map<string, boolean>();

export function hasOpaqueCorners(img: HTMLImageElement): boolean {
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

export function DockAppIcon({ src, alt, fern }: { src: string | undefined; alt: string; fern: boolean }) {
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

export function SearchIcon() {
  return (
    <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
      <circle cx="11" cy="11" r="7" />
      <line x1="21" y1="21" x2="16.65" y2="16.65" />
    </svg>
  );
}

export function CalendarIcon() {
  return (
    <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round">
      <rect x="3" y="4.5" width="18" height="16" rx="3" />
      <line x1="3" y1="10" x2="21" y2="10" />
      <line x1="8" y1="2.5" x2="8" y2="6.5" />
      <line x1="16" y1="2.5" x2="16" y2="6.5" />
    </svg>
  );
}

export function ChevronUpIcon() {
  return (
    <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.6" strokeLinecap="round" strokeLinejoin="round">
      <polyline points="6 15 12 9 18 15" />
    </svg>
  );
}

export function TaskViewIcon() {
  return (
    <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round">
      <rect x="3" y="5" width="14" height="11" rx="2.5" />
      <path d="M7 19.5h12a2.5 2.5 0 0 0 2.5-2.5V9.5" />
    </svg>
  );
}

export function RecycleIcon({ isEmpty = true }: { isEmpty?: boolean }) {
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

export function DriveIcon({ letter }: { letter: string }) {
  return (
    <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round">
      <rect x="2.5" y="7" width="19" height="10" rx="3" />
      <text x="12" y="14.6" textAnchor="middle" fontSize="7.5" fontWeight="700" fill="currentColor" stroke="none">{letter}</text>
    </svg>
  );
}

// One dot per open window, capped at three.
export function WindowDots({ count, mode }: { count: number; mode: string }) {
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

export function AttentionBadge({ count, isFlashing }: { count: number | null; isFlashing: boolean }) {
  if (count === null && !isFlashing) return null;
  const countStr = count !== null ? (count > 99 ? "99+" : String(count)) : null;
  return (
    <div className={`attention-badge ${countStr ? "has-count" : ""} ${isFlashing ? "flashing" : ""}`}>
      {countStr}
    </div>
  );
}
