import type { DeviceArtKey } from "../devices/art";
import { t as translate } from "../i18n";
import type { BatteryDetails } from "./types";

// Timer chime, synthesized with Web Audio so there's no asset. The context is made on the
// Start click so autoplay lets it play when the timer ends.
let timerChimeCtx: AudioContext | null = null;

export const getTimerChimeCtx = (): AudioContext | null => {
  try {
    if (!timerChimeCtx) timerChimeCtx = new AudioContext();
    if (timerChimeCtx.state === "suspended") timerChimeCtx.resume().catch(() => {});
    return timerChimeCtx;
  } catch {
    return null;
  }
};

export const playTimerChime = () => {
  const ctx = getTimerChimeCtx();
  if (!ctx) return;
  const start = ctx.currentTime + 0.02;
  const master = ctx.createGain();
  master.gain.value = 0.45;
  master.connect(ctx.destination);

  // Rising bell (A5, C#6, E6) with a quiet harmonic.
  const notes = [
    { freq: 880.0, at: 0 },
    { freq: 1108.73, at: 0.18 },
    { freq: 1318.51, at: 0.36 },
  ];
  notes.forEach(({ freq, at }) => {
    const osc = ctx.createOscillator();
    const harmonic = ctx.createOscillator();
    const gain = ctx.createGain();
    const harmonicGain = ctx.createGain();
    osc.type = "sine";
    osc.frequency.value = freq;
    harmonic.type = "sine";
    harmonic.frequency.value = freq * 2.01;
    harmonicGain.gain.value = 0.12;
    gain.gain.setValueAtTime(0.0001, start + at);
    gain.gain.exponentialRampToValueAtTime(0.32, start + at + 0.02);
    gain.gain.exponentialRampToValueAtTime(0.0001, start + at + 1.4);
    osc.connect(gain);
    harmonic.connect(harmonicGain);
    harmonicGain.connect(gain);
    gain.connect(master);
    osc.start(start + at);
    harmonic.start(start + at);
    osc.stop(start + at + 1.5);
    harmonic.stop(start + at + 1.5);
  });
};

export function formatDuration(seconds: number): string {
  const minutes = Math.max(1, Math.round(seconds / 60));
  const h = Math.floor(minutes / 60);
  const m = minutes % 60;
  return h > 0 ? `${h}h ${m}m` : `${m}m`;
}

export function batteryTooltip(
  charging: boolean,
  level: number,
  secondsLeft: number | null,
  details?: BatteryDetails | null
): string {
  const parts: string[] = [];
  if (charging && level >= 100) {
    parts.push(translate("battery.full"));
  } else {
    const state = translate(charging ? "battery.charging" : "battery.onBattery");
    const effectiveSecs = charging
      ? (details?.seconds_to_full ?? secondsLeft)
      : (details?.seconds_left ?? secondsLeft);
    if (effectiveSecs === null || effectiveSecs <= 0) {
      parts.push(translate("battery.status", { state, level }));
    } else {
      parts.push(translate(charging ? "battery.untilFull" : "battery.left", { state, level, duration: formatDuration(effectiveSecs) }));
    }
  }
  if (details?.health_percent != null) {
    parts.push(translate("battery.health", { percent: details.health_percent }));
  }
  if (details?.power_mode) {
    parts.push(details.power_mode);
  }
  return parts.join(" · ");
}

export const PLUGGED_ART = new Set<DeviceArtKey>([
  "headphones-wired",
  "earbuds-usbc",
  "earbuds-jack",
  "flash-drive",
  "ssd",
  "hub",
  "webcam",
  "microphone"
]);

export function deviceAlertStatus(art: DeviceArtKey, connected: boolean, wired = false): string {
  if (art === "sd-card") return connected ? translate("device.inserted") : translate("device.removed");
  if (wired || PLUGGED_ART.has(art)) return connected ? translate("device.plugged_in") : translate("device.unplugged");
  return connected ? translate("device.connected") : translate("device.disconnected");
}

export const DEVICE_ALERT_MS = { connected: 4200, disconnected: 2600 };
export const DEVICE_ALERT_QUEUE = 3;

export function formatLocalClock(date: Date, hour12: boolean): string {
  return date.toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
    hour12,
  });
}

export function msUntilNextMinute(now = new Date()): number {
  return Math.max(250, 60_000 - (now.getSeconds() * 1000 + now.getMilliseconds()) + 50);
}
