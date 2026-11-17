const MAX_SECONDS = 99 * 3600;

export function formatStopwatch(ms: number, precise: boolean): string {
  const total = Math.floor(ms / 1000);
  const hours = Math.floor(total / 3600);
  const mins = Math.floor((total % 3600) / 60);
  const secs = total % 60;
  const base = hours > 0 ? `${hours}:${String(mins).padStart(2, "0")}:${String(secs).padStart(2, "0")}` : `${mins}:${String(secs).padStart(2, "0")}`;
  if (!precise) return base;
  return `${base}.${String(Math.floor((ms % 1000) / 10)).padStart(2, "0")}`;
}

export function parseDuration(text: string): number | null {
  const value = text.trim();
  let seconds: number;
  if (/^\d+$/.test(value)) {
    seconds = Number(value) * 60;
  } else if (/^\d+:\d{1,2}$/.test(value)) {
    const [m, s] = value.split(":").map(Number);
    if (s > 59) return null;
    seconds = m * 60 + s;
  } else if (/^\d+:\d{1,2}:\d{1,2}$/.test(value)) {
    const [h, m, s] = value.split(":").map(Number);
    if (m > 59 || s > 59) return null;
    seconds = h * 3600 + m * 60 + s;
  } else {
    return null;
  }
  return seconds > 0 && seconds <= MAX_SECONDS ? seconds : null;
}

/** Formats up to 6 right-to-left typed digits as MM:SS or HH:MM:SS (e.g. "130" -> "01:30"). */
export function rtlDigitsToDisplay(digits: string): string {
  const clean = digits.replace(/\D/g, "").slice(-6);
  if (clean.length > 4) {
    const padded = clean.padStart(6, "0");
    return `${padded.slice(0, 2)}:${padded.slice(2, 4)}:${padded.slice(4, 6)}`;
  }
  const padded = clean.padStart(4, "0");
  return `${padded.slice(0, 2)}:${padded.slice(2, 4)}`;
}

/** Converts right-to-left typed digits ("130" -> 90s, "2500" -> 1500s) into seconds. */
export function rtlDigitsToSeconds(digits: string): number | null {
  const clean = digits.replace(/\D/g, "").slice(-6);
  if (!clean) return null;
  const padded = clean.padStart(6, "0");
  const h = Number(padded.slice(0, 2));
  const m = Number(padded.slice(2, 4));
  const s = Number(padded.slice(4, 6));
  const total = h * 3600 + m * 60 + s;
  return total > 0 && total <= MAX_SECONDS ? total : null;
}
