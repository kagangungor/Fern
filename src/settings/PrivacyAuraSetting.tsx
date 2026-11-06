import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { emit } from "@tauri-apps/api/event";
import { Shield } from "lucide-react";
import { SettingRow } from "./SettingRow";
import { useT } from "../i18n";

export function PrivacyAuraSetting() {
  const t = useT();
  const [auraEnabled, setAuraEnabled] = useState(() => {
    const saved = localStorage.getItem("fern-privacy-aura-enabled");
    return saved !== null ? saved === "true" : true;
  });

  const toggle = () => {
    const next = !auraEnabled;
    setAuraEnabled(next);
    localStorage.setItem("fern-privacy-aura-enabled", String(next));
    invoke("save_setting", { key: "fern-privacy-aura-enabled", value: String(next) }).catch(() => { });
    emit("privacy-aura-setting-changed", { enabled: next }).catch(() => { });
  };

  return (
    <SettingRow icon={Shield} label={t("meeting.privacyAura")} desc={t("meeting.privacyAura.desc")}>
      <label className="toggle-switch">
        <input type="checkbox" checked={auraEnabled} onChange={toggle} />
        <span className="slider"></span>
      </label>
    </SettingRow>
  );
}
