import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Palette } from "lucide-react";
import { SettingRow } from "./SettingRow";
import { useT } from "../i18n";

export function WindowsAccentSync({ accentColor }: { accentColor: string }) {
  const t = useT();
  const [enabled, setEnabled] = useState(() => localStorage.getItem("fern-sync-windows-accent") === "true");

  useEffect(() => {
    if (enabled && accentColor) {
      invoke("sync_windows_accent", { hex: accentColor }).catch(console.error);
    }
  }, [accentColor, enabled]);

  const toggle = async () => {
    const next = !enabled;
    setEnabled(next);
    localStorage.setItem("fern-sync-windows-accent", String(next));
    invoke("save_setting", { key: "fern-sync-windows-accent", value: String(next) }).catch(console.error);
    if (next) {
      invoke("sync_windows_accent", { hex: accentColor }).catch(console.error);
    } else {
      invoke("restore_windows_accent").catch(console.error);
    }
  };

  return (
    <SettingRow icon={Palette} label={t("appearance.syncWindowsAccent")} desc={t("appearance.syncWindowsAccent.desc")}>
      <label className="toggle-switch">
        <input type="checkbox" checked={enabled} onChange={toggle} />
        <span className="slider"></span>
      </label>
    </SettingRow>
  );
}
