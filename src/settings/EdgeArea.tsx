import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { MousePointer2 } from "lucide-react";
import { SettingRow } from "./SettingRow";
import { Dropdown } from "./Dropdown";
import { useT } from "../i18n";

type EdgeAreaKey = "fern-notch-edge-area" | "fern-dock-edge-area" | "fern-overlay-edge-area";

const EDGE_AREA_DESC: Record<EdgeAreaKey, string> = {
  "fern-notch-edge-area": "edgeArea.notch.desc",
  "fern-dock-edge-area": "edgeArea.dock.desc",
  "fern-overlay-edge-area": "edgeArea.overlay.desc",
};

export function EdgeArea({ settingKey }: { settingKey: EdgeAreaKey }) {
  const t = useT();
  const [value, setValue] = useState(() => (localStorage.getItem(settingKey) === "narrow" ? "narrow" : "full"));

  const change = (next: string) => {
    setValue(next);
    localStorage.setItem(settingKey, next);
    invoke("save_setting", { key: settingKey, value: next }).catch(() => {});
  };

  return (
    <SettingRow icon={MousePointer2} label={t("edgeArea.label")} desc={t(EDGE_AREA_DESC[settingKey])}>
      <Dropdown
        value={value}
        onChange={change}
        options={[
          { value: "full", label: t("edgeArea.full") },
          { value: "narrow", label: t("edgeArea.narrow") },
        ]}
      />
    </SettingRow>
  );
}
