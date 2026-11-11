import { useState, useEffect, useRef, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';

export interface MediaTimingInfo {
  position_ms?: number;
  duration_ms?: number;
  position_updated_at?: number;
}

export function useMediaPosition(mediaInfo: MediaTimingInfo, isPlaying: boolean) {
  const positionMs = mediaInfo.position_ms ?? 0;
  const durationMs = mediaInfo.duration_ms ?? 0;
  const positionUpdatedAt = mediaInfo.position_updated_at ?? 0;

  const backendPosRef = useRef(positionMs);
  const backendTimeRef = useRef(positionUpdatedAt || Date.now());
  const isSeekingRef = useRef(false);

  useEffect(() => {
    if (!isSeekingRef.current) {
      backendPosRef.current = positionMs;
      backendTimeRef.current = positionUpdatedAt || Date.now();
    }
  }, [positionMs, positionUpdatedAt]);

  const [displayPosition, setDisplayPosition] = useState(() => {
    const elapsed = isPlaying ? Math.max(0, Date.now() - (positionUpdatedAt || Date.now())) : 0;
    return Math.min(positionMs + elapsed, durationMs || Infinity);
  });

  useEffect(() => {
    const tick = () => {
      if (isSeekingRef.current) return;
      const elapsed = isPlaying ? Math.max(0, Date.now() - backendTimeRef.current) : 0;
      const pos = Math.min(backendPosRef.current + elapsed, durationMs || Infinity);
      setDisplayPosition(pos);
    };
    tick();
    if (!isPlaying) return;
    const interval = setInterval(tick, 250);
    return () => clearInterval(interval);
  }, [isPlaying, durationMs, positionMs, positionUpdatedAt]);

  const handleSeekStart = useCallback(() => {
    isSeekingRef.current = true;
  }, []);

  const handleSeekChange = useCallback((e: React.ChangeEvent<HTMLInputElement>) => {
    setDisplayPosition(Number(e.target.value));
  }, []);

  const handleSeekEnd = useCallback(() => {
    const seekPos = displayPosition;
    isSeekingRef.current = false;
    backendPosRef.current = seekPos;
    backendTimeRef.current = Date.now();
    invoke('media_seek', { positionMs: seekPos }).catch(() => {});
  }, [displayPosition]);

  const handleSeekToMs = useCallback((seekPos: number) => {
    setDisplayPosition(seekPos);
    backendPosRef.current = seekPos;
    backendTimeRef.current = Date.now();
    invoke('media_seek', { positionMs: seekPos }).catch(() => {});
  }, []);

  return {
    displayPosition,
    handleSeekStart,
    handleSeekChange,
    handleSeekEnd,
    handleSeekToMs,
  };
}
