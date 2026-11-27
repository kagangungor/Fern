import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Sparkles } from "lucide-react";
import { SettingRow } from "./SettingRow";
import { ShortcutRecorder } from "./ShortcutRecorder";
import { useT } from "../i18n";

export function AiHotkey() {
  const t = useT();
  const [value, setValue] = useState(() => localStorage.getItem("fern-ai-hotkey") || "");

  const commit = (next: string) => {
    setValue(next);
    localStorage.setItem("fern-ai-hotkey", next);
    invoke("save_setting", { key: "fern-ai-hotkey", value: next })
      .catch(() => {})
      .finally(() => {
        invoke("reload_hotkeys").catch(() => {});
      });
  };

  return (
    <SettingRow icon={Sparkles} label={t("notch.aiHotkey")} desc={t("notch.aiHotkey.desc")}>
      <ShortcutRecorder value={value} onCommit={commit} />
    </SettingRow>
  );
}
