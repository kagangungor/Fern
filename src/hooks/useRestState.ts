import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { restingInterval, type RestState } from "../restingInterval";

/** Whether Memory Saver considers the user away. Always "not resting" when it's turned off. */
export function useRestState(): RestState {
  const [state, setState] = useState<RestState>({ resting: false, locked: false });
  useEffect(() => {
    const unlisten = listen<RestState>("memory-saver-state", (event) => setState(event.payload));
    return () => { unlisten.then((fn) => fn()); };
  }, []);
  return state;
}

/**
 * Runs `callback` now and every `baseMs`, slowing down while the user is away and stopping while the
 * session is locked. It runs right away on any change of pace, so returning shows fresh data. Pass
 * null as `baseMs` to keep it off.
 */
export function useRestingInterval(callback: () => void, baseMs: number | null, rest: RestState) {
  const callbackRef = useRef(callback);
  callbackRef.current = callback;
  const ms = baseMs === null ? null : restingInterval(baseMs, rest);
  useEffect(() => {
    if (ms === null) return;
    callbackRef.current();
    const id = setInterval(() => callbackRef.current(), ms);
    return () => clearInterval(id);
  }, [ms]);
}
