import { Volume2, Sun, ArrowLeftToLine, ArrowRightToLine, Clock } from "lucide-react";
import { SettingRow } from "./SettingRow";
import { SettingSection } from "./SettingSection";
import { Dropdown } from "./Dropdown";
import { useT } from "../i18n";

interface OverlaysTabProps {
  volumeOverlayEnabled: boolean;
  toggleVolumeOverlay: () => void;
  volumeEdgeEnabled: boolean;
  toggleVolumeEdge: () => void;
  volumeSide: "left" | "right";
  handleVolumeSideChange: (side: string) => void;
  brightnessOverlayEnabled: boolean;
  toggleBrightnessOverlay: () => void;
  brightnessEdgeEnabled: boolean;
  toggleBrightnessEdge: () => void;
  brightnessSide: "left" | "right";
  handleBrightnessSideChange: (side: string) => void;
  edgeDelay: string;
  handleEdgeDelayChange: (delay: string) => void;
}

export function OverlaysTab({
  volumeOverlayEnabled,
  toggleVolumeOverlay,
  volumeEdgeEnabled,
  toggleVolumeEdge,
  volumeSide,
  handleVolumeSideChange,
  brightnessOverlayEnabled,
  toggleBrightnessOverlay,
  brightnessEdgeEnabled,
  toggleBrightnessEdge,
  brightnessSide,
  handleBrightnessSideChange,
  edgeDelay,
  handleEdgeDelayChange,
}: OverlaysTabProps) {
  const t = useT();
  const hasAnyEdge = (volumeOverlayEnabled && volumeEdgeEnabled) || (brightnessOverlayEnabled && brightnessEdgeEnabled);

  return (
    <>
      <SettingSection id="overlays-main" title={t("overlays.group")} defaultOpen>
        <SettingRow icon={Volume2} label={t("overlays.volume")} desc={t("overlays.volume.desc")}>
          <label className="toggle-switch">
            <input type="checkbox" checked={volumeOverlayEnabled} onChange={toggleVolumeOverlay} />
            <span className="slider"></span>
          </label>
        </SettingRow>

        {volumeOverlayEnabled && (
          <>
            <SettingRow icon={Volume2} label={t("overlays.volumeSide")} desc={t("overlays.volumeSide.desc")}>
              <Dropdown
                value={volumeSide}
                onChange={handleVolumeSideChange}
                options={[
                  { value: "left", label: t("common.side.left") },
                  { value: "right", label: t("common.side.right") },
                ]}
              />
            </SettingRow>

            <SettingRow icon={ArrowLeftToLine} label={t("overlays.edgeHover")} desc={t("overlays.edgeHover.left")}>
              <label className="toggle-switch">
                <input type="checkbox" checked={volumeEdgeEnabled} onChange={toggleVolumeEdge} />
                <span className="slider"></span>
              </label>
            </SettingRow>
          </>
        )}

        <SettingRow icon={Sun} label={t("overlays.brightness")} desc={t("overlays.brightness.desc")}>
          <label className="toggle-switch">
            <input type="checkbox" checked={brightnessOverlayEnabled} onChange={toggleBrightnessOverlay} />
            <span className="slider"></span>
          </label>
        </SettingRow>

        {brightnessOverlayEnabled && (
          <>
            <SettingRow icon={Sun} label={t("overlays.brightnessSide")} desc={t("overlays.brightnessSide.desc")}>
              <Dropdown
                value={brightnessSide}
                onChange={handleBrightnessSideChange}
                options={[
                  { value: "left", label: t("common.side.left") },
                  { value: "right", label: t("common.side.right") },
                ]}
              />
            </SettingRow>

            <SettingRow icon={ArrowRightToLine} label={t("overlays.edgeHover")} desc={t("overlays.edgeHover.right")} divider={!hasAnyEdge}>
              <label className="toggle-switch">
                <input type="checkbox" checked={brightnessEdgeEnabled} onChange={toggleBrightnessEdge} />
                <span className="slider"></span>
              </label>
            </SettingRow>
          </>
        )}

        {hasAnyEdge && (
          <SettingRow icon={Clock} label={t("overlays.edgeDelay")} desc={t("overlays.edgeDelay.desc")} divider={false}>
            <Dropdown
              value={edgeDelay}
              onChange={handleEdgeDelayChange}
              options={[
                { value: "0", label: "0 ms" },
                { value: "150", label: "150 ms" },
                { value: "300", label: "300 ms" },
                { value: "500", label: "500 ms" },
              ]}
            />
          </SettingRow>
        )}
      </SettingSection>
    </>
  );
}
