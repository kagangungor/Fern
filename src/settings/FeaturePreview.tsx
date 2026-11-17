import { useState } from "react";
import { BatteryWarning, Bluetooth, Clock, Monitor, Music, Sun, Volume2 } from "lucide-react";
import { useT } from "../i18n";

type PreviewScene = "resting" | "clock" | "music" | "battery" | "device" | "hud" | "dock";

const SCENES: { id: PreviewScene; labelKey: string }[] = [
  { id: "resting", labelKey: "preview.scene.resting" },
  { id: "clock", labelKey: "preview.scene.clock" },
  { id: "music", labelKey: "preview.scene.music" },
  { id: "battery", labelKey: "preview.scene.battery" },
  { id: "device", labelKey: "preview.scene.device" },
  { id: "hud", labelKey: "preview.scene.hud" },
  { id: "dock", labelKey: "preview.scene.dock" },
];

export function FeaturePreview({ themeStyle, themeMode }: { themeStyle: string; themeMode: string }) {
  const t = useT();
  const [scene, setScene] = useState<PreviewScene>("music");

  return (
    <div className="feature-preview-card">
      <div className="feature-preview-header">
        <span className="feature-preview-title">{t("preview.title")}</span>
        <span className="feature-preview-meta">{themeStyle} · {themeMode}</span>
      </div>

      <div className="feature-preview-stage">
        {scene === "resting" && (
          <div className="fp-pill compact">
            <span className="fp-dot" />
            <span>19:42</span>
          </div>
        )}
        {scene === "clock" && (
          <div className="fp-pill wide">
            <Clock size={14} />
            <span className="fp-mono">25:00</span>
            <span className="fp-badge">{t("clock.timer")}</span>
          </div>
        )}
        {scene === "music" && (
          <div className="fp-pill wide">
            <Music size={14} />
            <span>Midnight City · M83</span>
            <span className="fp-eq"><span /><span /><span /></span>
          </div>
        )}
        {scene === "battery" && (
          <div className="fp-pill wide warn">
            <BatteryWarning size={14} />
            <span>{t("battery.low")} · 14%</span>
          </div>
        )}
        {scene === "device" && (
          <div className="fp-pill wide">
            <Bluetooth size={14} />
            <span>AirPods Pro · 92%</span>
          </div>
        )}
        {scene === "hud" && (
          <div className="fp-hud-pair">
            <div className="fp-hud-box"><Volume2 size={13} /><div className="fp-bar"><div style={{ width: "68%" }} /></div></div>
            <div className="fp-hud-box"><Sun size={13} /><div className="fp-bar"><div style={{ width: "80%" }} /></div></div>
          </div>
        )}
        {scene === "dock" && (
          <div className="fp-dock-bar">
            <span className="fp-dock-icon"><Monitor size={14} /></span>
            <span className="fp-dock-icon active"><Music size={14} /></span>
            <span className="fp-dock-icon"><Clock size={14} /></span>
          </div>
        )}
      </div>

      <div className="feature-preview-tabs">
        {SCENES.map((s) => (
          <button
            key={s.id}
            type="button"
            className={`fp-tab${scene === s.id ? " active" : ""}`}
            onClick={() => setScene(s.id)}
          >
            {t(s.labelKey)}
          </button>
        ))}
      </div>
    </div>
  );
}
