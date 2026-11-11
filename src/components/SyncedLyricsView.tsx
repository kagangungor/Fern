import React, { useEffect, useRef, useMemo } from 'react';
import { Music2 } from 'lucide-react';
import { useT } from '../i18n';
import './SyncedLyricsView.css';

export interface LyricLine {
  time_ms: number;
  text: string;
}

export interface LyricsPayload {
  synced: boolean;
  lines: LyricLine[];
  plain?: string;
  source: string;
}

interface SyncedLyricsViewProps {
  lyrics: LyricsPayload | null;
  loading: boolean;
  currentTimeMs: number;
  onSeek?: (timeMs: number) => void;
  canSeek?: boolean;
}

export const SyncedLyricsView: React.FC<SyncedLyricsViewProps> = ({
  lyrics,
  loading,
  currentTimeMs,
  onSeek,
  canSeek = true,
}) => {
  const t = useT();
  const containerRef = useRef<HTMLDivElement>(null);
  const activeLineRef = useRef<HTMLDivElement>(null);
  const isUserScrollingRef = useRef(false);
  const userScrollTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Active line calculation with 150ms lead time
  const activeIndex = useMemo(() => {
    if (!lyrics?.synced || !lyrics?.lines?.length) return -1;
    const leadTimeMs = 150;
    const target = currentTimeMs + leadTimeMs;
    let idx = -1;
    const lines = lyrics.lines || [];
    for (let i = 0; i < lines.length; i++) {
      if (lines[i].time_ms <= target) {
        idx = i;
      } else {
        break;
      }
    }
    return idx;
  }, [lyrics, currentTimeMs]);

  // Auto-scroll when active index changes
  useEffect(() => {
    if (isUserScrollingRef.current) return;
    if (activeLineRef.current && containerRef.current) {
      activeLineRef.current.scrollIntoView({
        behavior: 'smooth',
        block: 'center',
      });
    }
  }, [activeIndex]);

  const handleWheel = (e: React.WheelEvent) => {
    e.stopPropagation();
    isUserScrollingRef.current = true;
    if (userScrollTimeoutRef.current) clearTimeout(userScrollTimeoutRef.current);
    userScrollTimeoutRef.current = setTimeout(() => {
      isUserScrollingRef.current = false;
    }, 2500);
  };

  const handleLineClick = (e: React.MouseEvent, timeMs: number) => {
    e.stopPropagation();
    if (canSeek && onSeek) {
      onSeek(timeMs);
    }
  };

  if (loading) {
    return (
      <div className="synced-lyrics-wrap">
        <div className="synced-lyrics-status synced-lyrics-pulse">
          <Music2 size={18} strokeWidth={1.8} style={{ opacity: 0.7 }} />
          <span>{t("media.lyricsLoading")}</span>
        </div>
      </div>
    );
  }

  if (!lyrics || (!lyrics.synced && !lyrics.plain)) {
    return (
      <div className="synced-lyrics-wrap">
        <div className="synced-lyrics-status">
          <Music2 size={18} strokeWidth={1.8} style={{ opacity: 0.4 }} />
          <span>{t("media.noLyrics")}</span>
        </div>
      </div>
    );
  }

  // Fallback for unsynced plain lyrics
  if (!lyrics.synced && lyrics.plain) {
    return (
      <div className="synced-lyrics-wrap">
        <div className="synced-lyrics-container" onWheel={handleWheel}>
          <div className="synced-lyrics-plain">{lyrics.plain}</div>
        </div>
      </div>
    );
  }

  return (
    <div className="synced-lyrics-wrap" onClick={(e) => e.stopPropagation()}>
      <div ref={containerRef} className="synced-lyrics-container" onWheel={handleWheel}>
        {(lyrics.lines || []).map((line, idx) => {
          const isActive = idx === activeIndex;
          if (!line.text.trim()) return null; // skip empty pauses
          return (
            <div
              key={`${line.time_ms}-${idx}`}
              ref={isActive ? activeLineRef : null}
              className={`synced-lyric-line${isActive ? ' active' : ''}`}
              onClick={(e) => handleLineClick(e, line.time_ms)}
              title={canSeek ? `${Math.floor(line.time_ms / 1000)}s` : undefined}
            >
              {line.text}
            </div>
          );
        })}
      </div>
    </div>
  );
};
