import { useState } from "react";
import { BookOpen, Check, CheckCircle2, Copy } from "lucide-react";
import { SettingRow } from "./SettingRow";
import { useT } from "../i18n";
import type { AiProviderId, AiStatusSnapshot } from "../ai/types";

interface CliGuideEntry {
  id: AiProviderId;
  name: string;
  installCmd: string;
  loginCmd: string;
  planKey: string;
}

const CLI_GUIDES: readonly CliGuideEntry[] = [
  {
    id: "claude",
    name: "Claude",
    installCmd: "npm install -g @anthropic-ai/claude-code",
    loginCmd: "claude",
    planKey: "integrations.ai.cliGuide.claudePlan",
  },
  {
    id: "openai",
    name: "ChatGPT",
    installCmd: "npm install -g @openai/codex",
    loginCmd: "codex",
    planKey: "integrations.ai.cliGuide.openaiPlan",
  },
  {
    id: "gemini",
    name: "Gemini",
    installCmd: "npm install -g @google/gemini-cli",
    loginCmd: "gemini",
    planKey: "integrations.ai.cliGuide.geminiPlan",
  },
];

export function AiCliGuide({ status }: { status: AiStatusSnapshot | null }) {
  const t = useT();
  const [open, setOpen] = useState(false);
  const [copiedId, setCopiedId] = useState<string | null>(null);

  const copyCommand = (id: string, cmd: string) => {
    navigator.clipboard
      ?.writeText(cmd)
      .then(() => {
        setCopiedId(id);
        setTimeout(() => {
          setCopiedId((cur) => (cur === id ? null : cur));
        }, 1600);
      })
      .catch(() => {});
  };

  return (
    <>
      <SettingRow
        icon={BookOpen}
        label={t("integrations.ai.cliGuide.title")}
        desc={t("integrations.ai.cliGuide.desc")}
        divider={!open}
      >
        <button type="button" className="settings-btn" onClick={() => setOpen((v) => !v)}>
          {open ? t("agent.intro.hide") : t("agent.intro.show")}
        </button>
      </SettingRow>

      {open && (
        <div className="ai-cli-guide">
          <div className="ai-cli-guide-note">{t("integrations.ai.cliGuide.note")}</div>
          {CLI_GUIDES.map((entry) => {
            const installed = status?.providers.find((p) => p.id === entry.id)?.cli_installed ?? false;
            const fullCmd = `${entry.installCmd}; ${entry.loginCmd}`;
            const copied = copiedId === entry.id;
            return (
              <div className="ai-cli-guide-item" key={entry.id}>
                <div className="ai-cli-guide-head">
                  <div className="ai-cli-guide-name">
                    <b>{entry.name}</b>
                    <span>{t(entry.planKey)}</span>
                  </div>
                  {installed && (
                    <span className="ai-cli-guide-badge">
                      <CheckCircle2 size={12} />
                      <span>{t("integrations.ai.cliDetected")}</span>
                    </span>
                  )}
                </div>
                <div className="ai-cli-guide-cmd-row">
                  <code className="ai-cli-guide-code">{fullCmd}</code>
                  <button
                    type="button"
                    className="settings-btn"
                    onClick={() => copyCommand(entry.id, fullCmd)}
                    title={t("ai.copy")}
                  >
                    {copied ? <Check size={12} /> : <Copy size={12} />}
                    <span>{copied ? t("integrations.ai.cliGuide.copied") : t("ai.copy")}</span>
                  </button>
                </div>
              </div>
            );
          })}
        </div>
      )}
    </>
  );
}
