/**
 * Attention and unread count detection for dock items.
 */

// Title count extraction: extracts "(3) Inbox", "(12+) WhatsApp", or "Telegram (5)"
export function titleCount(title: string): number | null {
  const trimmed = title.trim();
  const digits = (s: string): number | null => {
    const cleaned = s.replace(/\+$/, "");
    const n = parseInt(cleaned, 10);
    return isNaN(n) || n <= 0 ? null : n;
  };

  if (trimmed.startsWith("(")) {
    const idx = trimmed.indexOf(")");
    if (idx !== -1) {
      const n = digits(trimmed.slice(1, idx));
      if (n !== null) return n;
    }
  }

  if (trimmed.endsWith(")")) {
    const idx = trimmed.lastIndexOf("(");
    if (idx !== -1) {
      const n = digits(trimmed.slice(idx + 1, -1));
      if (n !== null) return n;
    }
  }

  return null;
}

export interface AppWithWindows {
  hwnd?: number;
  all_hwnds?: [number, string][];
}

/**
 * Checks if the application or any of its windows is currently flashing / requesting attention.
 */
export function appAttention(app: AppWithWindows, flashingHwnds: number[]): boolean {
  if (!flashingHwnds || flashingHwnds.length === 0) return false;
  if (app.hwnd && flashingHwnds.includes(app.hwnd)) return true;
  if (app.all_hwnds) {
    for (const [hwnd] of app.all_hwnds) {
      if (flashingHwnds.includes(hwnd)) return true;
    }
  }
  return false;
}

/**
 * Extracts total unread notifications count across all windows of an application.
 */
export function appUnreadCount(app: AppWithWindows): number | null {
  if (!app.all_hwnds || app.all_hwnds.length === 0) return null;
  let total = 0;
  let found = false;

  for (const [, title] of app.all_hwnds) {
    const count = titleCount(title);
    if (count !== null && count > 0) {
      total += count;
      found = true;
    }
  }

  return found ? total : null;
}
