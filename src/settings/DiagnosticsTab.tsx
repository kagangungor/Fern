import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ClipboardCopy, FolderOpen, Crosshair, Sparkles } from "lucide-react";
import { SettingRow } from "./SettingRow";
import { SettingSection } from "./SettingSection";
import { useT } from "../i18n";

export function DiagnosticsTab() {
  const t = useT();
  const [diagStatus, setDiagStatus] = useState<"idle" | "working" | "copied" | "error">("idle");
  const [hitboxLogging, setHitboxLogging] = useState(false);
  const [iconStatus, setIconStatus] = useState<"idle" | "working" | "done" | "error">("idle");

  useEffect(() => {
    invoke<boolean>("get_hitbox_logging").then(setHitboxLogging).catch(() => {});
  }, []);

  const copyDiagnostics = async () => {
    setDiagStatus("working");
    try {
      const report = await invoke<string>("get_diagnostics");
      await navigator.clipboard.writeText(report);
      setDiagStatus("copied");
    } catch {
      setDiagStatus("error");
    }
    setTimeout(() => setDiagStatus("idle"), 2500);
  };

  const refreshIconCache = async () => {
    setIconStatus("working");
    try {
      await invoke("clear_icon_cache");
      setIconStatus("done");
    } catch {
      setIconStatus("error");
    }
    setTimeout(() => setIconStatus("idle"), 2500);
  };

  const getIconCacheLabel = () => {
    if (iconStatus === "working") return t("about.icons.working");
    if (iconStatus === "done") return t("about.icons.done");
    if (iconStatus === "error") return t("about.icons.error");
    return t("about.icons");
  };

  const toggleHitboxLogging = () => {
    const next = !hitboxLogging;
    setHitboxLogging(next);
    invoke("set_hitbox_logging", { enabled: next }).catch(() => setHitboxLogging(!next));
  };

  const getDiagLabel = () => {
    if (diagStatus === "working") return t("about.diagnostics.working");
    if (diagStatus === "copied") return t("about.diagnostics.copied");
    if (diagStatus === "error") return t("about.diagnostics.error");
    return t("about.diagnostics.copy");
  };

  return (
    <>
      <SettingSection id="diagnostics-system" title={t("about.diagnostics.group")}>
        <SettingRow
          icon={ClipboardCopy}
          label={getDiagLabel()}
          desc={t("about.diagnostics.desc")}
          action
          onClick={copyDiagnostics}
        />
        <SettingRow
          icon={FolderOpen}
          label={t("about.logs")}
          desc={t("about.logs.desc")}
          action
          onClick={() => invoke("open_log_folder").catch(() => {})}
        />
        <SettingRow
          icon={Crosshair}
          label={t("about.hitbox")}
          desc={t("about.hitbox.desc")}
        >
          <label className="toggle-switch">
            <input type="checkbox" checked={hitboxLogging} onChange={toggleHitboxLogging} />
            <span className="slider"></span>
          </label>
        </SettingRow>
      </SettingSection>

      <SettingSection id="diagnostics-maintenance" title={t("about.data.group")}>
        <SettingRow
          icon={Sparkles}
          label={getIconCacheLabel()}
          desc={t("about.icons.desc")}
          action
          divider={false}
          onClick={refreshIconCache}
        />
      </SettingSection>
    </>
  );
}
