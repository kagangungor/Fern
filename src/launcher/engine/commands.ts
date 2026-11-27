import { invoke } from "@tauri-apps/api/core";
import { emit } from "@tauri-apps/api/event";
import type { QuickCommand } from "../types";
import { fuzzyScore } from "./fuzzy";

export function buildQuickCommands(): QuickCommand[] {
  return [
    {
      id: "fern-settings",
      labelKey: "launcher.cmd.settings",
      keywords: ["settings", "ayar", "ayarlar", "config", "preferences", "fern"],
      icon: "settings",
      run: () => invoke<void>("open_settings_window"),
    },
    {
      id: "fern-calendar",
      labelKey: "launcher.cmd.calendar",
      keywords: ["calendar", "takvim", "schedule", "events", "etkinlik"],
      icon: "calendar",
      run: () => invoke<void>("open_calendar_window"),
    },
    {
      id: "fern-mic-mute",
      labelKey: "launcher.cmd.micMute",
      keywords: ["mic", "microphone", "mute", "mikrofon", "sessiz", "sustur"],
      icon: "mic",
      run: () => invoke<void>("toggle_microphone_mute"),
    },
    {
      id: "fern-notch-toggle",
      labelKey: "launcher.cmd.notchToggle",
      keywords: ["notch", "island", "çentik", "ada", "toggle"],
      icon: "sparkles",
      run: () => emit("notch-hotkey-toggle"),
    },
    {
      id: "fern-restart",
      labelKey: "launcher.cmd.restart",
      keywords: ["restart", "yeniden başlat", "reload", "fern"],
      icon: "rotate",
      run: () => invoke<void>("restart_fern"),
    },
    {
      id: "sys-taskmgr",
      labelKey: "launcher.cmd.taskmgr",
      keywords: ["taskmgr", "task manager", "görev yöneticisi", "görev", "processes"],
      icon: "activity",
      run: () => invoke<void>("open_app", { appName: "taskmgr" }),
    },
    {
      id: "sys-desktop",
      labelKey: "launcher.cmd.desktop",
      keywords: ["desktop", "masaüstü", "minimize", "show desktop"],
      icon: "monitor",
      run: () => invoke<void>("show_desktop"),
    },
    {
      id: "sys-lock",
      labelKey: "launcher.cmd.lock",
      keywords: ["lock", "kilitle", "ekran", "ekranı kilitle", "lock screen"],
      icon: "lock",
      run: () => invoke<void>("launcher_system_action", { action: "lock" }),
    },
    {
      id: "sys-sleep",
      labelKey: "launcher.cmd.sleep",
      keywords: ["sleep", "uyku", "uyku modu", "suspend"],
      icon: "moon",
      confirmRequired: true,
      run: () => invoke<void>("launcher_system_action", { action: "sleep" }),
    },
    {
      id: "sys-empty-bin",
      labelKey: "launcher.cmd.emptyBin",
      keywords: ["recycle", "recycle bin", "trash", "çöp", "geri dönüşüm", "boşalt", "empty"],
      icon: "trash",
      confirmRequired: true,
      run: () => invoke<void>("empty_recycle_bin"),
    },
  ];
}

export function matchQuickCommands(
  commands: QuickCommand[],
  query: string,
  t: (key: string) => string
): { command: QuickCommand; label: string; score: number }[] {
  const trimmed = query.trim();
  if (!trimmed) return [];

  const matches: { command: QuickCommand; label: string; score: number }[] = [];
  for (const command of commands) {
    const label = t(command.labelKey);
    let best = fuzzyScore(label, trimmed);
    for (const kw of command.keywords) {
      const score = fuzzyScore(kw, trimmed);
      if (score > best) {
        best = score;
      }
    }
    if (best > 0) {
      matches.push({ command, label, score: best });
    }
  }

  matches.sort((a, b) => b.score - a.score);
  return matches;
}
