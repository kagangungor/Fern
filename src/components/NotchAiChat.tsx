import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type ClipboardEvent,
  type DragEvent,
  type FormEvent,
  type JSX,
  type KeyboardEvent,
  type MouseEvent,
} from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  ArrowUp,
  Camera,
  Check,
  Copy,
  ExternalLink,
  ImagePlus,
  KeyRound,
  RefreshCw,
  RotateCcw,
  Square,
  Trash2,
  X,
} from "lucide-react";
import { AiMarkdown } from "../ai/AiMarkdown";
import {
  AI_MAX_IMAGES,
  imageFilesFrom,
  imageFilesFromDataTransfer,
} from "../ai/aiImage";
import {
  AI_DEFAULT_MODELS,
  AI_MODEL_PRESETS,
  AI_MODEL_SETTINGS,
  AI_PROVIDERS,
  aiModelLabel,
  aiProviderLabel,
  type AiProviderId,
} from "../ai/types";
import { useAiChat } from "../ai/useAiChat";
import { useT } from "../i18n";
import "./NotchAiChat.css";

export interface NotchAiChatProps {
  autoFocusToken: number;
  onFocusChange: (focused: boolean) => void;
  onBusyChange: (busy: boolean) => void;
  onHasMessagesChange: (hasMessages: boolean) => void;
  onEscape: () => void;
}

const CONFIG_ERROR_CODES = new Set([
  "ai-not-configured",
  "api-key-missing",
  "cli-not-found",
  "cli-auth-required",
]);

function AiProviderLogo({ provider }: { provider: AiProviderId }): JSX.Element {
  if (provider === "claude") {
    return (
      <svg className="ai-provider-logo" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
        <path d="M12 1.5a1.1 1.1 0 0 1 1.1 1.1v5.9l4.17-4.17a1.1 1.1 0 1 1 1.56 1.56L14.66 10.06h5.84a1.1 1.1 0 1 1 0 2.2h-5.84l4.17 4.17a1.1 1.1 0 1 1-1.56 1.56L13.1 13.82v5.93a1.1 1.1 0 1 1-2.2 0v-5.93l-4.17 4.17a1.1 1.1 0 1 1-1.56-1.56l4.17-4.17H3.5a1.1 1.1 0 1 1 0-2.2h5.84L5.17 5.89a1.1 1.1 0 1 1 1.56-1.56L10.9 8.5V2.6A1.1 1.1 0 0 1 12 1.5Z" />
      </svg>
    );
  }
  if (provider === "openai") {
    return (
      <svg className="ai-provider-logo" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
        <path d="M22.28 9.82a5.98 5.98 0 0 0-.52-4.91 6.05 6.05 0 0 0-6.51-2.9A6.07 6.07 0 0 0 4.98 4.18a5.98 5.98 0 0 0-4 2.9 6.05 6.05 0 0 0 .74 7.1 5.98 5.98 0 0 0 .51 4.91 6.05 6.05 0 0 0 6.51 2.9A5.98 5.98 0 0 0 13.26 24a6.06 6.06 0 0 0 5.77-4.21 5.99 5.99 0 0 0 4-2.9 6.06 6.06 0 0 0-.75-7.07Zm-9.02 12.61a4.48 4.48 0 0 1-2.88-1.04l.14-.08 4.78-2.76a.8.8 0 0 0 .39-.68v-6.74l2.02 1.17a.07.07 0 0 1 .04.05v5.58a4.5 4.5 0 0 1-4.49 4.5Zm-9.66-4.13a4.47 4.47 0 0 1-.54-3.01l.14.09 4.78 2.76a.77.77 0 0 0 .78 0l5.84-3.37v2.33a.08.08 0 0 1-.03.06l-4.84 2.79a4.5 4.5 0 0 1-6.13-1.65ZM2.34 7.9a4.49 4.49 0 0 1 2.37-1.97V11.6a.77.77 0 0 0 .39.68l5.81 3.35-2.02 1.17a.08.08 0 0 1-.07 0l-4.83-2.79A4.5 4.5 0 0 1 2.34 7.9Zm16.6 3.86-5.84-3.37 2.02-1.16a.08.08 0 0 1 .07 0l4.83 2.79a4.49 4.49 0 0 1-.68 8.1v-5.68a.79.79 0 0 0-.4-.68Zm2.01-3.02-.14-.09-4.77-2.78a.78.78 0 0 0-.79 0L9.41 9.23V6.9a.07.07 0 0 1 .03-.06l4.83-2.79a4.5 4.5 0 0 1 6.68 4.66ZM8.31 12.86l-2.02-1.16a.08.08 0 0 1-.04-.06V6.07a4.5 4.5 0 0 1 7.38-3.45l-.14.08-4.78 2.76a.8.8 0 0 0-.39.68l-.01 6.72Zm1.1-2.36 2.6-1.5 2.6 1.5v3l-2.6 1.5-2.6-1.5v-3Z" />
      </svg>
    );
  }
  return (
    <svg className="ai-provider-logo ai-gemini-svg" viewBox="0 0 24 24" aria-hidden="true">
      <defs>
        <linearGradient id="fern-gemini-grad" x1="0%" y1="0%" x2="100%" y2="100%">
          <stop offset="0%" stopColor="#4E86F7" />
          <stop offset="52%" stopColor="#9B72CB" />
          <stop offset="100%" stopColor="#D96570" />
        </linearGradient>
      </defs>
      <path
        className="ai-gemini-path"
        d="M12 0.8C12 7.2 16.8 12 23.2 12C16.8 12 12 16.8 12 23.2C12 16.8 7.2 12 0.8 12C7.2 12 12 7.2 12 0.8Z"
      />
    </svg>
  );
}

function readStoredModel(provider: AiProviderId): string {
  try {
    const saved = localStorage.getItem(AI_MODEL_SETTINGS[provider])?.trim();
    if (saved) return saved;
  } catch {
    /* storage unavailable */
  }
  return AI_DEFAULT_MODELS[provider];
}

function interpolate(
  template: string,
  params?: Record<string, string | number>
): string {
  if (!params) return template;
  return template.replace(/\{(\w+)\}/g, (match, name) =>
    name in params ? String(params[name]) : match
  );
}

export function NotchAiChat({
  autoFocusToken,
  onFocusChange,
  onBusyChange,
  onHasMessagesChange,
  onEscape,
}: NotchAiChatProps): JSX.Element {
  const t = useT();
  const tr = useCallback(
    (
      key: string,
      fallback: string,
      params?: Record<string, string | number>
    ): string => {
      const value = t(key, params);
      return value === key ? interpolate(fallback, params) : value;
    },
    [t]
  );

  const {
    messages,
    prompt,
    setPrompt,
    images,
    imageBusy,
    addImages,
    removeImage,
    captureScreen,
    capturing,
    busy,
    provider,
    selectProvider,
    status,
    refreshStatus,
    notice,
    error,
    send,
    retry,
    cancel,
    clear,
    saveKey,
    deleteKey,
    openConsole,
  } = useAiChat();

  const [setupManual, setSetupManual] = useState<boolean | null>(null);
  const [keyDraft, setKeyDraft] = useState("");
  const [sessionOnly, setSessionOnly] = useState(false);
  const [savingKey, setSavingKey] = useState(false);
  const [deletingKey, setDeletingKey] = useState(false);
  const [rescanning, setRescanning] = useState(false);
  const [setupError, setSetupError] = useState<string | null>(null);
  const [model, setModel] = useState<string>(() => readStoredModel(provider));
  const [dragActive, setDragActive] = useState(false);
  const [copiedMsgId, setCopiedMsgId] = useState<string | null>(null);

  const textareaRef = useRef<HTMLTextAreaElement | null>(null);
  const fileInputRef = useRef<HTMLInputElement | null>(null);
  const messagesEndRef = useRef<HTMLDivElement | null>(null);
  const copyTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const dragDepthRef = useRef(0);
  const onFocusChangeRef = useRef(onFocusChange);
  onFocusChangeRef.current = onFocusChange;
  const prevSetupOpenRef = useRef(false);

  const activeStatus = useMemo(
    () => status?.providers.find((p) => p.id === provider) ?? null,
    [status, provider]
  );

  const isUnconfigured =
    status !== null && activeStatus?.active_backend === "none";
  const autoSetup = isUnconfigured && messages.length === 0;
  const setupOpen = setupManual ?? autoSetup;

  useEffect(() => {
    setModel(readStoredModel(provider));
    setKeyDraft("");
    setSetupError(null);
    setSetupManual(null);
  }, [provider]);

  useEffect(() => {
    if (prevSetupOpenRef.current && !setupOpen) {
      onFocusChangeRef.current(false);
    }
    prevSetupOpenRef.current = setupOpen;
  }, [setupOpen]);

  const enableKeyboardFocus = useCallback(() => {
    invoke("set_window_keyboard_focus", { enabled: true }).catch(() => {});
  }, []);

  const disableKeyboardFocus = useCallback(() => {
    invoke("set_window_keyboard_focus", { enabled: false }).catch(() => {});
  }, []);

  useEffect(() => {
    return () => {
      setKeyDraft("");
      onFocusChangeRef.current(false);
      invoke("set_window_keyboard_focus", { enabled: false }).catch(() => {});
      if (copyTimerRef.current !== null) {
        clearTimeout(copyTimerRef.current);
      }
    };
  }, []);

  useEffect(() => {
    onBusyChange(busy);
  }, [busy, onBusyChange]);

  useEffect(() => {
    onHasMessagesChange(messages.length > 0);
  }, [messages.length, onHasMessagesChange]);

  useEffect(() => {
    if (autoFocusToken > 0 && !setupOpen) {
      textareaRef.current?.focus();
    }
  }, [autoFocusToken, setupOpen]);

  useEffect(() => {
    if (!setupOpen) {
      messagesEndRef.current?.scrollIntoView({ behavior: "auto", block: "end" });
    }
  }, [messages, setupOpen]);

  useLayoutEffect(() => {
    const el = textareaRef.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, 76)}px`;
  }, [prompt]);

  const mascotSrc = busy
    ? "/mascot/working.svg"
    : error
      ? "/mascot/error.svg"
      : "/mascot/idle.svg";

  const providerTooltip = useCallback(
    (id: AiProviderId): string => {
      const label = aiProviderLabel(id);
      const pStatus = status?.providers.find((p) => p.id === id);
      if (!pStatus) return label;
      if (pStatus.active_backend === "api") {
        return `${label} (${tr("ai.backend.api", "API")})`;
      }
      if (pStatus.active_backend === "cli") {
        return `${label} (${tr("ai.backend.cli", "CLI")})`;
      }
      return `${label} (${tr("ai.backend.none", "Not configured")})`;
    },
    [status, tr]
  );

  const translateError = useCallback(
    (code: string): string => {
      const providerName = aiProviderLabel(provider);
      switch (code) {
        case "ai-not-configured":
          return tr(
            "ai.error.notConfigured",
            "{provider} is not configured. Add an API key or install its CLI.",
            { provider: providerName }
          );
        case "api-key-missing":
          return tr(
            "ai.error.apiKeyMissing",
            "Missing API key for {provider}.",
            { provider: providerName }
          );
        case "cli-not-found":
          return tr(
            "ai.error.cliNotFound",
            "{provider} CLI was not found on your PATH.",
            { provider: providerName }
          );
        case "cli-auth-required":
          return tr(
            "ai.error.cliAuthRequired",
            "{provider} CLI requires sign-in before use.",
            { provider: providerName }
          );
        case "cli-timeout":
          return tr(
            "ai.error.cliTimeout",
            "Request timed out. Please try again."
          );
        case "image-limit":
          return tr(
            "ai.error.imageLimit",
            "You can attach up to {max} images per message.",
            { max: AI_MAX_IMAGES }
          );
        case "image-unsupported":
          return tr("ai.error.imageUnsupported", "Unsupported image format.");
        case "image-too-large":
          return tr("ai.error.imageTooLarge", "Image file is too large.");
        case "image-decode-failed":
          return tr(
            "ai.error.imageDecodeFailed",
            "Could not read the selected image."
          );
        default:
          return code;
      }
    },
    [provider, tr]
  );

  const handleToggleSetup = () => {
    setSetupError(null);
    setSetupManual((prev) => !(prev ?? autoSetup));
  };

  const handleSaveKey = async (e: FormEvent) => {
    e.preventDefault();
    const trimmed = keyDraft.trim();
    if (!trimmed || savingKey) return;
    setSavingKey(true);
    setSetupError(null);
    try {
      await saveKey(provider, trimmed, sessionOnly);
      setKeyDraft("");
      setSessionOnly(false);
      setSetupManual(false);
      onFocusChangeRef.current(false);
    } catch (err) {
      setSetupError(err instanceof Error ? err.message : String(err));
    } finally {
      setSavingKey(false);
    }
  };

  const handleDeleteKey = async () => {
    if (deletingKey) return;
    setDeletingKey(true);
    setSetupError(null);
    try {
      await deleteKey(provider);
      setKeyDraft("");
    } catch (err) {
      setSetupError(err instanceof Error ? err.message : String(err));
    } finally {
      setDeletingKey(false);
    }
  };

  const handleModelChange = (nextModel: string) => {
    setModel(nextModel);
    const settingKey = AI_MODEL_SETTINGS[provider];
    try {
      localStorage.setItem(settingKey, nextModel);
    } catch {
      /* storage unavailable */
    }
    invoke("save_setting", { key: settingKey, value: nextModel }).catch(
      () => {}
    );
  };

  const handleRescan = async () => {
    if (rescanning) return;
    setRescanning(true);
    try {
      await refreshStatus(true);
    } finally {
      setRescanning(false);
    }
  };

  const handleCopyMessage = (msgId: string, text: string) => {
    navigator.clipboard
      ?.writeText(text)
      .then(() => {
        setCopiedMsgId(msgId);
        if (copyTimerRef.current !== null) {
          clearTimeout(copyTimerRef.current);
        }
        copyTimerRef.current = setTimeout(() => {
          setCopiedMsgId(null);
          copyTimerRef.current = null;
        }, 1500);
      })
      .catch(() => {});
  };

  const handleSend = () => {
    if (busy || (prompt.trim().length === 0 && images.length === 0)) return;
    setSetupManual(false);
    void send();
  };

  const handleKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Escape") {
      e.preventDefault();
      e.currentTarget.blur();
      disableKeyboardFocus();
      onEscape();
      return;
    }
    if (
      e.key === "Enter" &&
      !e.shiftKey &&
      !e.nativeEvent.isComposing
    ) {
      e.preventDefault();
      handleSend();
    }
  };

  const handlePaste = (e: ClipboardEvent<HTMLTextAreaElement>) => {
    const pastedImages = imageFilesFromDataTransfer(e.clipboardData);
    if (pastedImages.length > 0) {
      e.preventDefault();
      void addImages(pastedImages);
    }
  };

  const handleDragEnter = (e: DragEvent<HTMLDivElement>) => {
    e.preventDefault();
    dragDepthRef.current += 1;
    setDragActive(true);
  };

  const handleDragOver = (e: DragEvent<HTMLDivElement>) => {
    e.preventDefault();
    if (!dragActive) setDragActive(true);
  };

  const handleDragLeave = (e: DragEvent<HTMLDivElement>) => {
    e.preventDefault();
    dragDepthRef.current = Math.max(0, dragDepthRef.current - 1);
    if (dragDepthRef.current === 0) {
      setDragActive(false);
    }
  };

  const handleDrop = (e: DragEvent<HTMLDivElement>) => {
    e.preventDefault();
    dragDepthRef.current = 0;
    setDragActive(false);
    const droppedImages = imageFilesFromDataTransfer(e.dataTransfer);
    if (droppedImages.length > 0) {
      void addImages(droppedImages);
    }
  };

  const handleCaptureContextMenu = (e: MouseEvent<HTMLButtonElement>) => {
    e.preventDefault();
    if (!busy && !capturing && images.length < AI_MAX_IMAGES) {
      void captureScreen("window");
    }
  };

  const modelOptions = useMemo(() => {
    const presets = AI_MODEL_PRESETS[provider];
    return presets.includes(model) ? [...presets] : [model, ...presets];
  }, [provider, model]);

  const canSend =
    !busy && (prompt.trim().length > 0 || images.length > 0);
  const canRetry =
    !busy && messages.some((m) => m.role === "user");
  const isConfigError = error !== null && CONFIG_ERROR_CODES.has(error);
  const atImageLimit = images.length >= AI_MAX_IMAGES;

  return (
    <div className="ai-chat-panel">
      {/* Header bar */}
      <div className="ai-header">
        <div className="ai-header-left">
          <img
            className="ai-mascot"
            src={mascotSrc}
            alt=""
            draggable={false}
          />
          <div className="ai-providers" role="tablist" aria-label={tr("ai.providers", "AI providers")}>
            {AI_PROVIDERS.map((p) => {
              const pStatus = status?.providers.find((s) => s.id === p.id);
              const ready = Boolean(pStatus && pStatus.active_backend !== "none");
              const isSelected = p.id === provider;
              return (
                <button
                  key={p.id}
                  type="button"
                  role="tab"
                  aria-selected={isSelected}
                  aria-label={providerTooltip(p.id)}
                  className={`ai-provider-chip ai-brand-${p.id}${isSelected ? " active" : ""}`}
                  title={providerTooltip(p.id)}
                  onClick={() => selectProvider(p.id)}
                >
                  <AiProviderLogo provider={p.id} />
                  {ready && <span className="ai-provider-dot ready" aria-hidden="true" />}
                </button>
              );
            })}
          </div>
        </div>

        <div className="ai-header-actions">
          <button
            type="button"
            className="ai-icon-btn"
            disabled={messages.length === 0 && !busy}
            onClick={clear}
            title={tr("ai.newChat", "New chat")}
            aria-label={tr("ai.newChat", "New chat")}
          >
            <RotateCcw size={13} />
          </button>
          <button
            type="button"
            className={`ai-icon-btn${setupOpen ? " active" : ""}${isUnconfigured ? " attention" : ""}`}
            onClick={handleToggleSetup}
            title={tr("ai.setup.toggle", "Quick setup & keys")}
            aria-label={tr("ai.setup.toggle", "Quick setup & keys")}
            aria-expanded={setupOpen}
          >
            <KeyRound size={13} />
          </button>
        </div>
      </div>

      {/* Notice pill */}
      {notice && (
        <div className="ai-notice" role="status">
          {notice}
        </div>
      )}

      {/* Error banner */}
      {error && (
        <div className="ai-error" role="alert">
          <span className="ai-error-text">{translateError(error)}</span>
          <div className="ai-error-actions">
            {isConfigError && !setupOpen && (
              <button
                type="button"
                className="ai-pill-btn"
                onClick={() => setSetupManual(true)}
              >
                {tr("ai.configure", "Configure")}
              </button>
            )}
            {canRetry && (
              <button
                type="button"
                className="ai-pill-btn primary"
                onClick={() => void retry()}
              >
                {tr("ai.retry", "Retry")}
              </button>
            )}
          </div>
        </div>
      )}

      {/* Main body: Compact Inline Setup OR Message Stream */}
      {setupOpen ? (
        <div className="ai-setup">
          <form className="ai-setup-form" onSubmit={handleSaveKey}>
            <div className="ai-setup-input-row">
              <input
                type="password"
                className="ai-setup-input"
                value={keyDraft}
                onChange={(e) => setKeyDraft(e.target.value)}
                onMouseDown={(e) => {
                  const target = e.currentTarget;
                  enableKeyboardFocus();
                  requestAnimationFrame(() => target.focus());
                }}
                onFocus={() => {
                  enableKeyboardFocus();
                  onFocusChange(true);
                }}
                onBlur={() => {
                  onFocusChange(false);
                  disableKeyboardFocus();
                }}
                autoComplete="off"
                spellCheck={false}
                placeholder={
                  activeStatus?.has_api_key
                    ? tr("ai.setup.keyReplacePlaceholder", "Enter new API key to replace...")
                    : tr("ai.setup.keyPlaceholder", "Paste {provider} API key...", {
                        provider: aiProviderLabel(provider),
                      })
                }
                aria-label={tr("ai.setup.apiKeyLabel", "API key")}
              />
              <button
                type="submit"
                className="ai-setup-save-btn"
                disabled={!keyDraft.trim() || savingKey}
              >
                {tr("ai.setup.save", "Save")}
              </button>
              {activeStatus?.has_api_key && (
                <button
                  type="button"
                  className="ai-setup-icon-btn danger"
                  disabled={deletingKey}
                  onClick={() => void handleDeleteKey()}
                  title={tr("ai.setup.deleteKey", "Remove saved API key")}
                  aria-label={tr("ai.setup.deleteKey", "Remove saved API key")}
                >
                  <Trash2 size={12} />
                </button>
              )}
              <button
                type="button"
                className="ai-setup-icon-btn"
                onClick={() => void openConsole(provider)}
                title={tr("ai.setup.getKey", "Get API Key")}
                aria-label={tr("ai.setup.getKey", "Get API Key")}
              >
                <ExternalLink size={12} />
              </button>
            </div>
            {setupError && <div className="ai-setup-error">{setupError}</div>}
          </form>

          <div className="ai-setup-models" role="radiogroup" aria-label={tr("ai.setup.model", "Model")}>
            {modelOptions.map((m) => (
              <button
                key={m}
                type="button"
                role="radio"
                aria-checked={m === model}
                className={`ai-model-chip${m === model ? " active" : ""}`}
                title={m}
                onClick={() => handleModelChange(m)}
              >
                {aiModelLabel(m)}
              </button>
            ))}
          </div>

          <div className="ai-setup-meta">
            <button
              type="button"
              role="switch"
              aria-checked={sessionOnly}
              className={`ai-session-toggle${sessionOnly ? " active" : ""}`}
              onClick={() => setSessionOnly((v) => !v)}
            >
              <span className="ai-session-track" aria-hidden="true">
                <span className="ai-session-thumb" />
              </span>
              <span>
                {tr(
                  "ai.setup.sessionOnly",
                  "Keep for this session only (do not save to vault)"
                )}
              </span>
            </button>

            <button
              type="button"
              className="ai-setup-ghost-btn"
              disabled={rescanning}
              onClick={() => void handleRescan()}
            >
              <RefreshCw
                size={11}
                className={rescanning ? "ai-spin" : undefined}
              />
              <span>{tr("ai.setup.rescanCli", "Rescan CLIs")}</span>
            </button>
          </div>
        </div>
      ) : (
        <div className="ai-messages">
          {messages.length === 0 ? (
            <div className="ai-empty">
              <span className="ai-empty-hint">
                {tr("ai.emptyHint", "Ask anything, paste an image, or capture your screen.")}
              </span>
            </div>
          ) : (
            messages.map((msg) => {
              const isUser = msg.role === "user";
              const isThinking = !isUser && msg.streaming && !msg.text.trim();
              const showCopy = !isUser && !msg.streaming && msg.text.trim().length > 0;
              return (
                <div
                  key={msg.id}
                  className={`ai-msg ${isUser ? "ai-msg-user" : "ai-msg-assistant"}`}
                >
                  {isUser ? (
                    <div className="ai-bubble-user">
                      {msg.images && msg.images.length > 0 && (
                        <div className="ai-msg-images">
                          {msg.images.map((img) => (
                            <img
                              key={img.id}
                              src={img.dataUrl}
                              alt={img.name}
                              className="ai-msg-thumb"
                              draggable={false}
                            />
                          ))}
                        </div>
                      )}
                      {msg.text && <div className="ai-msg-text">{msg.text}</div>}
                    </div>
                  ) : (
                    <div className="ai-bubble-assistant">
                      {isThinking ? (
                        <div
                          className="ai-thinking"
                          aria-label={tr("ai.thinking", "Thinking...")}
                        >
                          <span />
                          <span />
                          <span />
                        </div>
                      ) : (
                        <AiMarkdown text={msg.text} />
                      )}
                      {showCopy && (
                        <div className="ai-msg-actions">
                          <button
                            type="button"
                            className="ai-msg-copy-btn"
                            onClick={() => handleCopyMessage(msg.id, msg.text)}
                            title={tr("ai.copy", "Copy")}
                            aria-label={tr("ai.copy", "Copy")}
                          >
                            {copiedMsgId === msg.id ? (
                              <Check size={11} />
                            ) : (
                              <Copy size={11} />
                            )}
                          </button>
                        </div>
                      )}
                    </div>
                  )}
                </div>
              );
            })
          )}
          <div ref={messagesEndRef} />
        </div>
      )}

      {/* Composer (hidden while inline setup is open to avoid visual clutter) */}
      {!setupOpen && (
      <div
        className={`ai-composer${dragActive ? " drag-active" : ""}`}
        onDragEnter={handleDragEnter}
        onDragOver={handleDragOver}
        onDragLeave={handleDragLeave}
        onDrop={handleDrop}
      >
        {images.length > 0 && (
          <div className="ai-attach-strip">
            {images.map((img) => (
              <div key={img.id} className="ai-attach-item">
                <img
                  src={img.dataUrl}
                  alt={img.name}
                  className="ai-attach-thumb"
                  draggable={false}
                />
                <button
                  type="button"
                  className="ai-attach-remove"
                  onClick={() => removeImage(img.id)}
                  title={tr("ai.removeImage", "Remove image")}
                  aria-label={tr("ai.removeImage", "Remove image")}
                >
                  <X size={9} />
                </button>
              </div>
            ))}
          </div>
        )}

        <div className="ai-composer-row">
          <input
            ref={fileInputRef}
            type="file"
            accept="image/*"
            multiple
            className="ai-file-input"
            onChange={(e) => {
              const picked = imageFilesFrom(e.target.files);
              if (picked.length > 0) {
                void addImages(picked);
              }
              e.target.value = "";
            }}
          />

          <div className="ai-composer-tools">
            <button
              type="button"
              className="ai-composer-btn"
              disabled={busy || imageBusy || atImageLimit}
              onClick={() => fileInputRef.current?.click()}
              title={tr("ai.attachImage", "Attach image")}
              aria-label={tr("ai.attachImage", "Attach image")}
            >
              <ImagePlus size={14} />
            </button>
            <button
              type="button"
              className={`ai-composer-btn${capturing ? " capturing" : ""}`}
              disabled={busy || capturing || atImageLimit}
              onClick={() => void captureScreen("screen")}
              onContextMenu={handleCaptureContextMenu}
              title={tr(
                "ai.captureTooltip",
                "Click to capture screen · Right-click for active window"
              )}
              aria-label={tr("ai.captureScreen", "Capture screen")}
            >
              <Camera size={14} />
            </button>
          </div>

          <textarea
            ref={textareaRef}
            rows={1}
            className="ai-textarea"
            value={prompt}
            onChange={(e) => setPrompt(e.target.value)}
            onKeyDown={handleKeyDown}
            onPaste={handlePaste}
            onMouseDown={(e) => {
              const target = e.currentTarget;
              enableKeyboardFocus();
              requestAnimationFrame(() => target.focus());
            }}
            onFocus={() => {
              enableKeyboardFocus();
              onFocusChange(true);
            }}
            onBlur={() => {
              onFocusChange(false);
              disableKeyboardFocus();
            }}
            placeholder={tr("ai.placeholder", "Ask {provider}...", {
              provider: aiProviderLabel(provider),
            })}
            aria-label={tr("ai.inputLabel", "Message input")}
          />

          {busy ? (
            <button
              type="button"
              className="ai-send-btn stop"
              onClick={cancel}
              title={tr("ai.stop", "Stop generating")}
              aria-label={tr("ai.stop", "Stop generating")}
            >
              <Square size={11} fill="currentColor" />
            </button>
          ) : (
            <button
              type="button"
              className="ai-send-btn"
              disabled={!canSend}
              onClick={handleSend}
              title={tr("ai.send", "Send message")}
              aria-label={tr("ai.send", "Send message")}
            >
              <ArrowUp size={14} />
            </button>
          )}
        </div>
      </div>
      )}
    </div>
  );
}
