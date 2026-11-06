import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Keyboard } from "lucide-react";
import { SettingRow } from "./SettingRow";
import { ShortcutRecorder } from "./ShortcutRecorder";
import { useT } from "../i18n";

export function NotchHotkey() {
  const t = useT();
  const [value, setValue] = useState(() => localStorage.getItem("fern-notch-hotkey") || "");

  const commit = (next: string) => {
    setValue(next);
    localStorage.setItem("fern-notch-hotkey", next);
    invoke("save_setting", { key: "fern-notch-hotkey", value: next }).catch(() => { });
    invoke("reload_hotkeys").catch(() => { });
  };

  return (
    <SettingRow icon={Keyboard} label={t("notch.hotkey")} desc={t("notch.hotkey.desc")}>
      <ShortcutRecorder value={value} onCommit={commit} />
    </SettingRow>
  );
}
