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

  useEffect(() => {
    return () => {
      setKeyDraft("");
      onFocusChangeRef.current(false);
      if (copyTimerRef.current !== null) {
        clearTimeout(copyTimerRef.current);
      }
    };
  }, []);

  useEffect(() => {
    onBusyChange(busy);
  }, [busy, onBusyChange]);

  useEffect(() => {
    onHasMessagesChange(messages.length > 0 || setupOpen);
  }, [messages.length, setupOpen, onHasMessagesChange]);

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
                  className={`ai-provider-chip${isSelected ? " active" : ""}`}
                  title={providerTooltip(p.id)}
                  onClick={() => selectProvider(p.id)}
                >
                  <span
                    className={`ai-provider-dot${ready ? " ready" : ""}`}
                    aria-hidden="true"
                  />
                  <span>{p.label}</span>
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

      {/* Main body: Quick Setup OR Message Stream */}
      {setupOpen ? (
        <div className="ai-setup">
          <div className="ai-setup-head">
            <span className="ai-setup-title">
              {tr("ai.setup.title", "{provider} Setup", {
                provider: aiProviderLabel(provider),
              })}
            </span>
            <span
              className={`ai-setup-badge${activeStatus && activeStatus.active_backend !== "none" ? " ready" : ""}`}
            >
              {activeStatus?.active_backend === "api"
                ? tr("ai.backend.api", "API")
                : activeStatus?.active_backend === "cli"
                  ? tr("ai.backend.cli", "CLI")
                  : tr("ai.backend.none", "Not configured")}
            </span>
          </div>

          <form className="ai-setup-form" onSubmit={handleSaveKey}>
            <div className="ai-setup-input-row">
              <input
                type="password"
                className="ai-setup-input"
                value={keyDraft}
                onChange={(e) => setKeyDraft(e.target.value)}
                onFocus={() => onFocusChange(true)}
                onBlur={() => onFocusChange(false)}
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
                  className="ai-setup-delete-btn"
                  disabled={deletingKey}
                  onClick={() => void handleDeleteKey()}
                  title={tr("ai.setup.deleteKey", "Remove saved API key")}
                  aria-label={tr("ai.setup.deleteKey", "Remove saved API key")}
                >
                  <Trash2 size={12} />
                </button>
              )}
            </div>

            <label className="ai-setup-checkbox">
              <input
                type="checkbox"
                checked={sessionOnly}
                onChange={(e) => setSessionOnly(e.target.checked)}
              />
              <span>
                {tr(
                  "ai.setup.sessionOnly",
                  "Keep for this session only (do not save to vault)"
                )}
              </span>
            </label>

            {setupError && <div className="ai-setup-error">{setupError}</div>}
          </form>

          <div className="ai-setup-model-row">
            <label className="ai-setup-label" htmlFor="ai-model-select">
              {tr("ai.setup.model", "Model")}
            </label>
            <select
              id="ai-model-select"
              className="ai-setup-select"
              value={model}
              onChange={(e) => handleModelChange(e.target.value)}
              onFocus={() => onFocusChange(true)}
              onBlur={() => onFocusChange(false)}
            >
              {modelOptions.map((m) => (
                <option key={m} value={m}>
                  {m}
                </option>
              ))}
            </select>
          </div>

          <div className="ai-setup-footer">
            <button
              type="button"
              className="ai-setup-link-btn"
              onClick={() => void openConsole(provider)}
            >
              <ExternalLink size={11} />
              <span>{tr("ai.setup.getKey", "Get API Key")}</span>
            </button>
            <button
              type="button"
              className="ai-setup-link-btn"
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
              <img
                className="ai-empty-mascot"
                src={mascotSrc}
                alt=""
                draggable={false}
              />
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

      {/* Composer */}
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
            onFocus={() => onFocusChange(true)}
            onBlur={() => onFocusChange(false)}
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
    </div>
  );
}
