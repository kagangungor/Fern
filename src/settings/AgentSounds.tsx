import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Volume2, RotateCcw, FolderOpen } from "lucide-react";
import { playAgentSound, type SoundState } from "../agentSounds";
import { SettingRow } from "./SettingRow";
import { useT } from "../i18n";

const STATES: SoundState[] = ["needs_input", "done", "error"];

export function AgentSounds() {
  const t = useT();
  const [hasCustom, setHasCustom] = useState<Record<SoundState, boolean>>({
    needs_input: false,
    done: false,
    error: false,
  });

  const refreshCustom = async () => {
    try {
      const active = await invoke<SoundState[]>("agent_sound_custom");
      const activeSet = new Set(Array.isArray(active) ? active : []);
      setHasCustom({
        needs_input: activeSet.has("needs_input"),
        done: activeSet.has("done"),
        error: activeSet.has("error"),
      });
    } catch {
      setHasCustom({ needs_input: false, done: false, error: false });
    }
  };

  useEffect(() => {
    refreshCustom();
  }, []);

  const handlePick = async (state: SoundState) => {
    try {
      const picked = await invoke<boolean>("pick_agent_sound", { state });
      if (picked) {
        await refreshCustom();
        playAgentSound(state, true);
      }
    } catch (e) {
      console.error("Failed to pick agent sound:", e);
    }
  };

  const handleReset = async (state: SoundState) => {
    try {
      await invoke("reset_agent_sound", { state });
      await refreshCustom();
      playAgentSound(state, true);
    } catch (e) {
      console.error("Failed to reset agent sound:", e);
    }
  };

  return (
    <>
      {STATES.map((state, idx) => (
        <SettingRow
          key={state}
          icon={Volume2}
          label={t(`notch.agent.sound.${state}`)}
          desc={hasCustom[state] ? t("notch.agent.sound.customActive") : t("notch.agent.sound.defaultActive")}
          divider={idx < STATES.length - 1}
        >
          <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
            <button
              type="button"
              className="settings-btn icon-only"
              onClick={() => playAgentSound(state, true)}
              title={t("notch.agent.sound.test")}
            >
              <Volume2 size={13} />
            </button>
            <button
              type="button"
              className="settings-btn"
              onClick={() => handlePick(state)}
            >
              <FolderOpen size={13} />
              <span>{hasCustom[state] ? t("notch.agent.sound.change") : t("notch.agent.sound.choose")}</span>
            </button>
            {hasCustom[state] && (
              <button
                type="button"
                className="settings-btn icon-only"
                onClick={() => handleReset(state)}
                title={t("notch.agent.sound.reset")}
              >
                <RotateCcw size={13} />
              </button>
            )}
          </div>
        </SettingRow>
      ))}
    </>
  );
}
