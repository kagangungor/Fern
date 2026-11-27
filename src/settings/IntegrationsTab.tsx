import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Bot,
  CheckCircle2,
  CircleDashed,
  ExternalLink,
  KeyRound,
  RefreshCw,
  Sparkles,
  Terminal,
  Trash2,
  Volume2,
} from "lucide-react";
import { SettingRow } from "./SettingRow";
import { SettingSection } from "./SettingSection";
import { AgentSounds } from "./AgentSounds";
import { AgentIntro } from "./AgentIntro";
import { AiCliGuide } from "./AiCliGuide";
import { Dropdown } from "./Dropdown";
import { useT } from "../i18n";
import {
  AI_DEFAULT_MODELS,
  AI_MODEL_PRESETS,
  AI_MODEL_SETTINGS,
  AI_PROVIDERS,
  aiModelLabel,
  aiProviderLabel,
  type AiBackendMode,
  type AiProviderId,
  type AiStatusSnapshot,
} from "../ai/types";

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

  // Fernie AI Chat settings state
  const [aiStatus, setAiStatus] = useState<AiStatusSnapshot | null>(null);
  const [aiProvider, setAiProvider] = useState<AiProviderId>(() => {
    const raw = localStorage.getItem("fern-ai-provider");
    return raw === "openai" || raw === "gemini" ? raw : "claude";
  });
  const [aiBackend, setAiBackend] = useState<AiBackendMode>(() => {
    const raw = localStorage.getItem("fern-ai-backend");
    return raw === "api" || raw === "cli" ? raw : "auto";
  });
  const [keyDrafts, setKeyDrafts] = useState<Record<AiProviderId, string>>({
    claude: "",
    openai: "",
    gemini: "",
  });
  const [sessionOnlyMap, setSessionOnlyMap] = useState<Record<AiProviderId, boolean>>({
    claude: false,
    openai: false,
    gemini: false,
  });
  const [models, setModels] = useState<Record<AiProviderId, string>>(() => ({
    claude: localStorage.getItem(AI_MODEL_SETTINGS.claude) || AI_DEFAULT_MODELS.claude,
    openai: localStorage.getItem(AI_MODEL_SETTINGS.openai) || AI_DEFAULT_MODELS.openai,
    gemini: localStorage.getItem(AI_MODEL_SETTINGS.gemini) || AI_DEFAULT_MODELS.gemini,
  }));
  const [aiBusyProvider, setAiBusyProvider] = useState<AiProviderId | null>(null);

  const refresh = useCallback(() => {
    invoke<IntegrationStatus[]>("get_agent_integrations_status")
      .then(setItems)
      .catch(() => {});
  }, []);

  const refreshAi = useCallback((refreshCli = false) => {
    invoke<AiStatusSnapshot>("ai_get_status", { refreshCli })
      .then((snap) => setAiStatus(snap))
      .catch(() => {});
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh, agentApiEnabled]);

  useEffect(() => {
    refreshAi(false);
    return () => {
      // Clear any unsaved key drafts on unmount
      setKeyDrafts({ claude: "", openai: "", gemini: "" });
    };
  }, [refreshAi]);

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

  const changeAiProvider = (next: string) => {
    const id: AiProviderId = next === "openai" || next === "gemini" ? next : "claude";
    setAiProvider(id);
    localStorage.setItem("fern-ai-provider", id);
    invoke("save_setting", { key: "fern-ai-provider", value: id }).catch(() => {});
  };

  const changeAiBackend = (next: string) => {
    const mode: AiBackendMode = next === "api" || next === "cli" ? next : "auto";
    setAiBackend(mode);
    localStorage.setItem("fern-ai-backend", mode);
    invoke("save_setting", { key: "fern-ai-backend", value: mode }).catch(() => {});
  };

  const changeAiModel = (provider: AiProviderId, model: string) => {
    setModels((prev) => ({ ...prev, [provider]: model }));
    const key = AI_MODEL_SETTINGS[provider];
    localStorage.setItem(key, model);
    invoke("save_setting", { key, value: model }).catch(() => {});
  };

  const handleSaveKey = async (provider: AiProviderId) => {
    const trimmed = keyDrafts[provider].trim();
    if (!trimmed) return;
    setAiBusyProvider(provider);
    try {
      await invoke("ai_set_key", {
        provider,
        key: trimmed,
        sessionOnly: sessionOnlyMap[provider],
      });
      setKeyDrafts((prev) => ({ ...prev, [provider]: "" }));
      refreshAi(false);
    } finally {
      setAiBusyProvider(null);
    }
  };

  const handleDeleteKey = async (provider: AiProviderId) => {
    setAiBusyProvider(provider);
    try {
      await invoke("ai_delete_key", { provider });
      setKeyDrafts((prev) => ({ ...prev, [provider]: "" }));
      refreshAi(false);
    } finally {
      setAiBusyProvider(null);
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

      <SettingSection id="integrations-ai-chat" title={t("integrations.ai.group")} defaultOpen>
        <SettingRow icon={Sparkles} label={t("integrations.ai.provider")} desc={t("integrations.ai.provider.desc")}>
          <Dropdown
            value={aiProvider}
            onChange={changeAiProvider}
            options={AI_PROVIDERS.map(({ id, label }) => ({ value: id, label }))}
          />
        </SettingRow>

        <SettingRow icon={Terminal} label={t("integrations.ai.backend")} desc={t("integrations.ai.backend.desc")}>
          <Dropdown
            value={aiBackend}
            onChange={changeAiBackend}
            options={[
              { value: "auto", label: t("integrations.ai.backend.auto") },
              { value: "api", label: t("integrations.ai.backend.api") },
              { value: "cli", label: t("integrations.ai.backend.cli") },
            ]}
          />
        </SettingRow>

        <SettingRow icon={RefreshCw} label={t("integrations.ai.rescan")} desc={t("integrations.ai.rescan.desc")}>
          <button type="button" className="settings-btn" onClick={() => refreshAi(true)}>
            <RefreshCw size={13} />
            <span>{t("ai.setup.rescanCli")}</span>
          </button>
        </SettingRow>

        <AiCliGuide status={aiStatus} />

        {AI_PROVIDERS.map(({ id }, idx) => {
          const st = aiStatus?.providers.find((p) => p.id === id);
          const statusParts: string[] = [];
          if (st?.has_api_key) statusParts.push(t("integrations.ai.keySaved"));
          if (st?.cli_installed) statusParts.push(t("integrations.ai.cliDetected"));
          const desc = statusParts.length > 0 ? statusParts.join(" · ") : t("ai.backend.none");
          const presets = AI_MODEL_PRESETS[id];
          const busy = aiBusyProvider === id;

          return (
            <SettingRow
              key={id}
              icon={KeyRound}
              label={aiProviderLabel(id)}
              desc={desc}
              stacked
              divider={idx < AI_PROVIDERS.length - 1}
            >
              <div style={{ display: "flex", flexDirection: "column", gap: 8, width: "100%" }}>
                <div style={{ display: "flex", alignItems: "center", gap: 8, flexWrap: "wrap" }}>
                  <input
                    type="password"
                    className="settings-input"
                    style={{ flex: 1, minWidth: 180 }}
                    value={keyDrafts[id]}
                    onChange={(e) => setKeyDrafts((prev) => ({ ...prev, [id]: e.target.value }))}
                    placeholder={
                      st?.has_api_key
                        ? t("ai.setup.keyReplacePlaceholder")
                        : t("ai.setup.keyPlaceholder", { provider: aiProviderLabel(id) })
                    }
                    autoComplete="off"
                    spellCheck={false}
                  />
                  <button
                    type="button"
                    className="settings-btn"
                    disabled={busy || !keyDrafts[id].trim()}
                    onClick={() => void handleSaveKey(id)}
                  >
                    <span>{t("ai.setup.save")}</span>
                  </button>
                  {st?.has_api_key && (
                    <button
                      type="button"
                      className="settings-btn"
                      disabled={busy}
                      onClick={() => void handleDeleteKey(id)}
                      title={t("ai.setup.deleteKey")}
                    >
                      <Trash2 size={13} />
                    </button>
                  )}
                  <button
                    type="button"
                    className="settings-btn"
                    onClick={() => invoke("ai_open_console", { provider: id }).catch(() => {})}
                    title={t("ai.setup.getKey")}
                  >
                    <ExternalLink size={13} />
                  </button>
                </div>

                <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: 10, flexWrap: "wrap" }}>
                  <div style={{ display: "inline-flex", alignItems: "center", gap: 8 }}>
                    <label className="toggle-switch" style={{ transform: "scale(0.8)", transformOrigin: "left center" }}>
                      <input
                        type="checkbox"
                        checked={sessionOnlyMap[id]}
                        onChange={(e) => setSessionOnlyMap((prev) => ({ ...prev, [id]: e.target.checked }))}
                      />
                      <span className="slider"></span>
                    </label>
                    <span style={{ fontSize: 12, opacity: 0.72 }}>{t("ai.setup.sessionOnly")}</span>
                  </div>
                  <div style={{ display: "inline-flex", alignItems: "center", gap: 6 }}>
                    <span style={{ fontSize: 12, opacity: 0.7 }}>{t("ai.setup.model")}:</span>
                    <Dropdown
                      value={models[id]}
                      onChange={(val) => changeAiModel(id, val)}
                      options={presets.map((m) => ({ value: m, label: aiModelLabel(m) }))}
                    />
                  </div>
                </div>
              </div>
            </SettingRow>
          );
        })}
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
