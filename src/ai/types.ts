export type AiProviderId = "claude" | "openai" | "gemini";

export type AiBackendMode = "auto" | "api" | "cli";

export interface AiImage {
  id: string;
  name: string;
  dataUrl: string;
}

export interface AiChatMessage {
  id: string;
  role: "user" | "assistant";
  text: string;
  images?: AiImage[];
  streaming?: boolean;
  provider?: AiProviderId;
}

export interface AiProviderStatus {
  id: AiProviderId;
  label: string;
  has_api_key: boolean;
  cli_installed: boolean;
  active_backend: "api" | "cli" | "none";
}

export interface AiStatusSnapshot {
  default_provider: AiProviderId;
  backend_mode: AiBackendMode;
  providers: AiProviderStatus[];
}

export const AI_PROVIDERS: readonly { id: AiProviderId; label: string }[] = [
  { id: "claude", label: "Claude" },
  { id: "openai", label: "ChatGPT" },
  { id: "gemini", label: "Gemini" },
] as const;

export const AI_DEFAULT_MODELS: Record<AiProviderId, string> = {
  claude: "claude-sonnet-4-5",
  openai: "gpt-4o-mini",
  gemini: "gemini-flash-latest",
};

export const AI_MODEL_PRESETS: Record<AiProviderId, readonly string[]> = {
  claude: ["claude-sonnet-4-5", "claude-haiku-4-5", "claude-opus-4-1"],
  openai: ["gpt-4o-mini", "gpt-4o", "o4-mini"],
  gemini: ["gemini-flash-latest", "gemini-2.5-flash", "gemini-2.5-pro"],
};

export const AI_MODEL_SETTINGS: Record<AiProviderId, string> = {
  claude: "fern-ai-model",
  openai: "fern-ai-model-openai",
  gemini: "fern-ai-model-gemini",
};

export function normalizeAiProvider(raw: string | null | undefined): AiProviderId {
  switch ((raw ?? "").trim().toLowerCase()) {
    case "openai":
    case "chatgpt":
    case "gpt":
      return "openai";
    case "gemini":
    case "google":
      return "gemini";
    default:
      return "claude";
  }
}

export function aiProviderLabel(id: AiProviderId | string): string {
  switch (id.trim().toLowerCase()) {
    case "claude":
      return "Claude";
    case "openai":
    case "chatgpt":
    case "gpt":
      return "ChatGPT";
    case "gemini":
    case "google":
      return "Gemini";
    default:
      return id;
  }
}
