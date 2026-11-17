import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Bot, CheckCircle2, CircleDashed, RefreshCw, Terminal, Volume2 } from "lucide-react";
import { SettingRow } from "./SettingRow";
import { SettingSection } from "./SettingSection";
import { AgentSounds } from "./AgentSounds";
import { AgentIntro } from "./AgentIntro";
import { useT } from "../i18n";

interface IntegrationStatus {
  id: string;
  name: string;
  detected: boolean;
  hooks_installed: boolean;
  config_path: string | null;
}

interface IntegrationsTabProps {
  agentApiEnabled: boolean;
  toggleAgentApi: () => void;
  agentSoundsEnabled: boolean;
  toggleAgentSounds: () => void;
}

export function IntegrationsTab({
  agentApiEnabled,
  toggleAgentApi,
  agentSoundsEnabled,
  toggleAgentSounds,
}: IntegrationsTabProps) {
  const t = useT();
  const [items, setItems] = useState<IntegrationStatus[]>([]);
  const [busyId, setBusyId] = useState<string | null>(null);

  const refresh = useCallback(() => {
    invoke<IntegrationStatus[]>("get_agent_integrations_status")
      .then(setItems)
      .catch(() => {});
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh, agentApiEnabled]);

  const toggleHook = async (item: IntegrationStatus) => {
    setBusyId(item.id);
    try {
      const next = await invoke<IntegrationStatus[]>("sync_agent_integration", {
        id: item.id,
        enable: !item.hooks_installed,
      });
      setItems(next);
    } finally {
      setBusyId(null);
    }
  };

  return (
    <>
      <SettingSection id="integrations-agents" title={t("notch.agent.group")} defaultOpen>
        <SettingRow icon={Bot} label={t("notch.agent.enable")} desc={t("integrations.agent.desc")}>
          <label className="toggle-switch">
            <input type="checkbox" checked={agentApiEnabled} onChange={toggleAgentApi} />
            <span className="slider"></span>
          </label>
        </SettingRow>

        {agentApiEnabled && (
          <>
            <SettingRow icon={Volume2} label={t("notch.agent.sounds")} desc={t("notch.agent.sounds.desc")}>
              <label className="toggle-switch">
                <input type="checkbox" checked={agentSoundsEnabled} onChange={toggleAgentSounds} />
                <span className="slider"></span>
              </label>
            </SettingRow>
            {agentSoundsEnabled && <AgentSounds />}
            <AgentIntro enabled={agentApiEnabled} />
          </>
        )}
      </SettingSection>

      <SettingSection id="integrations-clients" title={t("integrations.clients.group")} defaultOpen>
        <SettingRow icon={RefreshCw} label={t("integrations.refresh")} desc={t("integrations.refresh.desc")}>
          <button type="button" className="settings-btn" onClick={refresh}>
            <RefreshCw size={13} />
            <span>{t("integrations.refreshBtn")}</span>
          </button>
        </SettingRow>

        {items.map((item, idx) => {
          const statusText = item.hooks_installed
            ? t("integrations.status.connected")
            : item.detected
              ? t("integrations.status.detected")
              : t("integrations.status.notFound");
          const desc = item.config_path ? `${statusText} · ${item.config_path}` : statusText;

          return (
            <SettingRow
              key={item.id}
              icon={Terminal}
              label={item.name}
              desc={desc}
              divider={idx < items.length - 1}
            >
              <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                {item.hooks_installed ? (
                  <CheckCircle2 size={14} style={{ color: "#4ade80" }} />
                ) : (
                  <CircleDashed size={14} style={{ opacity: 0.45 }} />
                )}
                <button
                  type="button"
                  className="settings-btn"
                  disabled={busyId === item.id || (!item.detected && !item.hooks_installed)}
                  onClick={() => toggleHook(item)}
                >
                  {item.hooks_installed ? t("integrations.remove") : t("integrations.install")}
                </button>
              </div>
            </SettingRow>
          );
        })}
      </SettingSection>
    </>
  );
}
