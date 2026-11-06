import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Mic } from "lucide-react";
import { SettingRow } from "./SettingRow";
import { ShortcutRecorder } from "./ShortcutRecorder";
import { useT } from "../i18n";

export function MicMuteHotkey() {
  const t = useT();
  const [value, setValue] = useState(() => {
    const saved = localStorage.getItem("fern-mic-mute-hotkey");
    return saved !== null ? saved : "Ctrl+F1";
  });

  const commit = (next: string) => {
    setValue(next);
    localStorage.setItem("fern-mic-mute-hotkey", next);
    invoke("save_setting", { key: "fern-mic-mute-hotkey", value: next }).catch(() => { });
    invoke("reload_hotkeys").catch(() => { });
  };

  return (
    <SettingRow icon={Mic} label={t("meeting.micHotkey")} desc={t("meeting.micHotkey.desc")}>
      <ShortcutRecorder value={value} onCommit={commit} />
    </SettingRow>
  );
}
