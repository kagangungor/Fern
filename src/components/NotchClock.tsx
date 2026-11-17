import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { Stopwatch } from "../hooks/useStopwatch";
import { formatStopwatch, rtlDigitsToDisplay, rtlDigitsToSeconds } from "../clockFormat";
import { useT } from "../i18n";

const DIGITS = ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"];

function OdometerDisplay({ value }: { value: string }) {
  return (
    <span className="odometer-clock" aria-label={value}>
      {value.split("").map((ch, idx) => {
        if (ch < "0" || ch > "9") {
          return (
            <span key={`sep-${idx}`} className="odometer-sep">
              {ch}
            </span>
          );
        }
        const d = Number(ch);
        return (
          <span key={`d-${value.length - idx}`} className="odometer-digit">
            <span className="odometer-strip" style={{ transform: `translateY(-${d * 10}%)` }}>
              {DIGITS.map((n) => (
                <span key={n} className="odometer-cell">
                  {n}
                </span>
              ))}
            </span>
          </span>
        );
      })}
    </span>
  );
}

type Tab = "timer" | "stopwatch";

const TAB_KEY = "fern-clock-tab";
const PRESETS = [5, 15, 25, 50];

export function StopwatchReadout({ base, startedAt, precise }: { base: number; startedAt: number | null; precise: boolean }) {
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    if (startedAt === null) return;
    const id = setInterval(() => setNow(Date.now()), precise ? 50 : 250);
    return () => clearInterval(id);
  }, [startedAt, precise]);

  const ms = base + (startedAt === null ? 0 : Math.max(0, Math.max(now, startedAt) - startedAt));
  return <>{formatStopwatch(ms, precise)}</>;
}

function remember(key: string, value: string) {
  try {
    localStorage.setItem(key, value);
  } catch {
    return;
  }
}

function readTab(): Tab {
  try {
    return localStorage.getItem(TAB_KEY) === "stopwatch" ? "stopwatch" : "timer";
  } catch {
    return "timer";
  }
}

interface Props {
  timerEnabled: boolean;
  stopwatchEnabled: boolean;
  solo: boolean;
  timerSeconds: number;
  isTimerRunning: boolean;
  formatTimerTime: (seconds: number) => string;
  onStartTimer: (seconds: number) => void;
  onToggleTimer: () => void;
  onResetTimer: () => void;
  stopwatch: Stopwatch;
}

export function NotchClock({
  timerEnabled,
  stopwatchEnabled,
  solo,
  timerSeconds,
  isTimerRunning,
  formatTimerTime,
  onStartTimer,
  onToggleTimer,
  onResetTimer,
  stopwatch,
}: Props) {
  const t = useT();
  const [savedTab, setSavedTab] = useState<Tab>(readTab);
  const [editing, setEditing] = useState(false);
  const [editDigits, setEditDigits] = useState("");
  const displayRef = useRef<HTMLDivElement>(null);

  const setKeyboardFocus = (enabled: boolean) => invoke("set_window_keyboard_focus", { enabled }).catch(() => {});
  useEffect(() => () => void setKeyboardFocus(false), []);

  const tab: Tab = savedTab === "stopwatch" && stopwatchEnabled ? "stopwatch" : timerEnabled ? "timer" : "stopwatch";

  const pickTab = (next: Tab) => {
    setSavedTab(next);
    remember(TAB_KEY, next);
  };

  const startEditing = () => {
    if (isTimerRunning) return;
    setEditing(true);
    setEditDigits("");
    setKeyboardFocus(true).finally(() => displayRef.current?.focus());
  };

  const stopEditing = () => {
    setEditing(false);
    setEditDigits("");
    setKeyboardFocus(false);
  };

  const commitEdit = () => {
    const secs = rtlDigitsToSeconds(editDigits);
    stopEditing();
    if (secs !== null) onStartTimer(secs);
  };

  const handleDisplayKeyDown = (e: React.KeyboardEvent<HTMLDivElement>) => {
    if (!editing) {
      if (e.key === "Enter" || e.key === " ") {
        e.preventDefault();
        startEditing();
      }
      return;
    }
    if (/^\d$/.test(e.key)) {
      e.preventDefault();
      setEditDigits((prev) => (prev + e.key).slice(-6));
    } else if (e.key === "Backspace") {
      e.preventDefault();
      setEditDigits((prev) => prev.slice(0, -1));
    } else if (e.key === "Enter") {
      e.preventDefault();
      commitEdit();
    } else if (e.key === "Escape") {
      e.preventDefault();
      stopEditing();
    }
  };

  const recentLaps = stopwatch.laps.slice(-3).map((total, i, list) => {
    const index = stopwatch.laps.length - list.length + i;
    const previous = index === 0 ? 0 : stopwatch.laps[index - 1];
    return { number: index + 1, split: total - previous };
  });

  return (
    <div className={`timer-column${solo ? " solo" : ""}`}>
      <div className="timer-section-new">
        {timerEnabled && stopwatchEnabled && (
          <div className="clock-tabs" role="tablist">
            <button role="tab" aria-selected={tab === "timer"} className={tab === "timer" ? "active" : ""} onClick={() => pickTab("timer")}>
              {t("clock.timer")}
            </button>
            <button
              role="tab"
              aria-selected={tab === "stopwatch"}
              className={tab === "stopwatch" ? "active" : ""}
              onClick={() => pickTab("stopwatch")}
            >
              {t("clock.stopwatch")}
            </button>
          </div>
        )}

        {tab === "timer" ? (
          <>
            <div
              ref={displayRef}
              tabIndex={isTimerRunning ? -1 : 0}
              role="button"
              aria-label={t("clock.custom.label")}
              title={isTimerRunning ? undefined : t("clock.custom.placeholder")}
              className={`timer-display-large odometer-editable${editing ? " editing" : ""}${isTimerRunning ? " running" : ""}`}
              onMouseDown={(e) => {
                if (!isTimerRunning) {
                  e.preventDefault();
                  startEditing();
                }
              }}
              onKeyDown={handleDisplayKeyDown}
              onBlur={stopEditing}
            >
              <span className="timer-time-large">
                <OdometerDisplay value={editing ? rtlDigitsToDisplay(editDigits) : formatTimerTime(timerSeconds)} />
              </span>
            </div>

            <div className="timer-controls-new">
              <button onClick={editing ? commitEdit : onToggleTimer} className="timer-btn primary">
                {isTimerRunning ? t("clock.pause") : t("clock.start")}
              </button>
              <button onClick={() => { stopEditing(); onResetTimer(); }} className="timer-btn secondary">
                {t("clock.reset")}
              </button>
            </div>

            <div className="timer-presets-new">
              {PRESETS.map((mins) => (
                <button key={mins} onClick={() => { stopEditing(); onStartTimer(mins * 60); }} className="preset-btn-small">
                  {mins}m
                </button>
              ))}
            </div>
          </>
        ) : (
          <>
            <div className="timer-display-large">
              <span className="timer-time-large timer-time-precise">
                <StopwatchReadout base={stopwatch.base} startedAt={stopwatch.startedAt} precise />
              </span>
            </div>

            <div className="timer-controls-new">
              <button onClick={stopwatch.toggle} className="timer-btn primary">
                {stopwatch.running ? t("clock.pause") : t("clock.start")}
              </button>
              <button onClick={stopwatch.lap} className="timer-btn secondary" disabled={!stopwatch.running && stopwatch.base === 0}>
                {t("clock.lap")}
              </button>
              <button onClick={stopwatch.reset} className="timer-btn secondary">
                {t("clock.reset")}
              </button>
            </div>

            <div className="sw-laps">
              {recentLaps.length === 0 && <span className="sw-hint">{t("clock.laps.empty")}</span>}
              {recentLaps.map((lap) => (
                <div key={lap.number} className="sw-lap">
                  <span>{t("clock.lapNumber", { number: lap.number })}</span>
                  <span>{formatStopwatch(lap.split, true)}</span>
                </div>
              ))}
            </div>
          </>
        )}
      </div>
    </div>
  );
}
