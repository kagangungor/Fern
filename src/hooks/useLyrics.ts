import { useState, useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { LyricsPayload } from '../components/SyncedLyricsView';

export function useLyrics(
  title: string,
  artist: string,
  durationMs: number | undefined,
  isOpen: boolean,
  enabled: boolean
) {
  const [lyrics, setLyrics] = useState<LyricsPayload | null>(null);
  const [lyricsLoading, setLyricsLoading] = useState(false);

  const trackKey = title.trim() ? `${artist.trim()}|${title.trim()}` : '';
  const requestIdRef = useRef(0);
  const loadedTrackKeyRef = useRef('');
  const durationRef = useRef(durationMs);
  durationRef.current = durationMs;

  // Immediately clear stale lyrics and invalidate any in-flight request when the track changes
  useEffect(() => {
    requestIdRef.current += 1;
    loadedTrackKeyRef.current = '';
    setLyrics(null);
    setLyricsLoading(false);
  }, [trackKey]);

  // Fetch lyrics when the drawer is open and enabled
  useEffect(() => {
    if (!isOpen || !enabled || !trackKey) {
      setLyricsLoading(false);
      return;
    }

    // If a previous fetch for this track returned null and user re-opened the drawer, allow retry;
    // otherwise skip duplicate fetches for the already-loaded track.
    if (loadedTrackKeyRef.current === trackKey) {
      return;
    }

    const reqId = ++requestIdRef.current;
    setLyricsLoading(true);

    const dur = durationRef.current;
    invoke<LyricsPayload | null>('get_lyrics', {
      title: title.trim(),
      artist: artist.trim() || null,
      durationMs: dur ? Math.round(dur) : null,
    })
      .then((res) => {
        if (requestIdRef.current !== reqId) return;
        if (res) {
          loadedTrackKeyRef.current = trackKey;
        }
        setLyrics(res);
      })
      .catch((err) => {
        if (requestIdRef.current !== reqId) return;
        console.warn('[lyrics] fetch failed:', err);
        setLyrics(null);
      })
      .finally(() => {
        if (requestIdRef.current === reqId) {
          setLyricsLoading(false);
        }
      });
  }, [isOpen, enabled, trackKey, title, artist]);

  return { lyrics, lyricsLoading };
}
