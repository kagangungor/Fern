import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useSettingsSync } from "../hooks/useSettingsSync";
import {
  AI_MAX_IMAGES,
  dataUrlToAiImage,
  fileToAiImage,
  imageFilesFrom,
} from "./aiImage";
import {
  AI_MODEL_SETTINGS,
  aiProviderLabel,
  normalizeAiProvider,
  type AiChatMessage,
  type AiImage,
  type AiProviderId,
  type AiStatusSnapshot,
} from "./types";

interface AiStreamPayload {
  id: string;
  delta: string;
}

interface WireAiMessage {
  role: "user" | "assistant";
  text: string;
  images: string[];
}

const PROVIDER_STORAGE_KEY = "fern-ai-provider";
const NOTICE_TIMEOUT_MS = 4000;
const MAX_IN_MEMORY_MESSAGES = 20;

// In-memory session store so closing/reopening the Notch keeps the last 20 messages
// without ever writing chat history to disk or localStorage.
let persistedSessionMessages: AiChatMessage[] = [];

let seqCounter = 0;

function nextId(prefix: string): string {
  seqCounter += 1;
  return `${prefix}-${Date.now()}-${seqCounter}`;
}

function readStoredProvider(): AiProviderId {
  try {
    return normalizeAiProvider(localStorage.getItem(PROVIDER_STORAGE_KEY));
  } catch {
    return "claude";
  }
}

function readStoredModel(provider: AiProviderId): string | undefined {
  try {
    const raw = localStorage.getItem(AI_MODEL_SETTINGS[provider]);
    const trimmed = raw?.trim();
    return trimmed ? trimmed : undefined;
  } catch {
    return undefined;
  }
}

function errorToString(err: unknown): string {
  if (typeof err === "string") return err;
  if (err instanceof Error) return err.message;
  return String(err ?? "unknown-error");
}

function toWireMessages(list: AiChatMessage[]): WireAiMessage[] {
  return list
    .filter((m) => m.text.trim().length > 0 || (m.images && m.images.length > 0))
    .map((m) => ({
      role: m.role,
      text: m.text,
      images: (m.images ?? []).map((img) => img.dataUrl),
    }));
}

export function useAiChat() {
  const [messages, setMessagesState] = useState<AiChatMessage[]>(() => persistedSessionMessages);
  const setMessages = useCallback(
    (updater: AiChatMessage[] | ((prev: AiChatMessage[]) => AiChatMessage[])) => {
      setMessagesState((prev) => {
        const next = typeof updater === "function" ? updater(prev) : updater;
        const capped = next.length > MAX_IN_MEMORY_MESSAGES ? next.slice(-MAX_IN_MEMORY_MESSAGES) : next;
        persistedSessionMessages = capped;
        return capped;
      });
    },
    []
  );
  const [prompt, setPrompt] = useState("");
  const [images, setImages] = useState<AiImage[]>([]);
  const [imageBusy, setImageBusy] = useState(false);
  const [capturing, setCapturing] = useState(false);
  const [busy, setBusy] = useState(false);
  const [provider, setProvider] = useState<AiProviderId>(readStoredProvider);
  const [status, setStatus] = useState<AiStatusSnapshot | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const messagesRef = useRef<AiChatMessage[]>(messages);
  messagesRef.current = messages;
  const imagesRef = useRef<AiImage[]>(images);
  imagesRef.current = images;
  const promptRef = useRef<string>(prompt);
  promptRef.current = prompt;
  const providerRef = useRef<AiProviderId>(provider);
  providerRef.current = provider;
  const busyRef = useRef<boolean>(busy);
  busyRef.current = busy;
  const capturingRef = useRef<boolean>(capturing);
  capturingRef.current = capturing;

  const requestIdRef = useRef<string | null>(null);
  const assistantMsgIdRef = useRef<string | null>(null);
  const requestedProviderRef = useRef<AiProviderId>(provider);
  const noticeTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const showNotice = useCallback((text: string) => {
    setNotice(text);
    if (noticeTimerRef.current !== null) {
      clearTimeout(noticeTimerRef.current);
    }
    noticeTimerRef.current = setTimeout(() => {
      setNotice(null);
      noticeTimerRef.current = null;
    }, NOTICE_TIMEOUT_MS);
  }, []);

  const refreshStatus = useCallback(async (refreshCli = false) => {
    try {
      const snap = await invoke<AiStatusSnapshot>("ai_get_status", { refreshCli });
      setStatus(snap);
      return snap;
    } catch {
      return null;
    }
  }, []);

  useEffect(() => {
    invoke<void>("ai_warm").catch(() => {});
    refreshStatus(false);

    return () => {
      if (noticeTimerRef.current !== null) {
        clearTimeout(noticeTimerRef.current);
      }
    };
  }, [refreshStatus]);

  useSettingsSync({
    "fern-ai-provider": (val) => {
      const next = normalizeAiProvider(String(val));
      setProvider(next);
      try {
        localStorage.setItem(PROVIDER_STORAGE_KEY, next);
      } catch {
        /* storage unavailable */
      }
    },
    "fern-ai-backend": () => {
      refreshStatus(false);
    },
  });

  useEffect(() => {
    const unlistenStream = listen<AiStreamPayload>("ai-stream", (event) => {
      const { id, delta } = event.payload;
      if (!delta || requestIdRef.current !== id) return;
      const targetMsgId = assistantMsgIdRef.current;
      if (!targetMsgId) return;

      setMessages((prev) =>
        prev.map((m) => (m.id === targetMsgId ? { ...m, text: m.text + delta } : m))
      );
    });

    const unlistenProviderUsed = listen<string>("ai-provider-used", (event) => {
      if (!requestIdRef.current) return;
      const used = normalizeAiProvider(event.payload);
      const targetMsgId = assistantMsgIdRef.current;
      if (targetMsgId) {
        setMessages((prev) =>
          prev.map((m) => (m.id === targetMsgId ? { ...m, provider: used } : m))
        );
      }
      if (used !== requestedProviderRef.current) {
        showNotice(`Switched to ${aiProviderLabel(used)}`);
      }
    });

    return () => {
      unlistenStream.then((fn) => fn());
      unlistenProviderUsed.then((fn) => fn());
    };
  }, [showNotice]);

  const selectProvider = useCallback((next: AiProviderId) => {
    setProvider(next);
    try {
      localStorage.setItem(PROVIDER_STORAGE_KEY, next);
    } catch {
      /* storage unavailable */
    }
    invoke("save_setting", { key: PROVIDER_STORAGE_KEY, value: next }).catch(() => {});
  }, []);

  const addImages = useCallback(async (input: FileList | File[]) => {
    const files = imageFilesFrom(input);
    if (files.length === 0) return;

    const remaining = AI_MAX_IMAGES - imagesRef.current.length;
    if (remaining <= 0) {
      setError("image-limit");
      return;
    }

    const batch = files.slice(0, remaining);
    setImageBusy(true);
    setError(null);
    try {
      const converted: AiImage[] = [];
      for (const file of batch) {
        try {
          converted.push(await fileToAiImage(file));
        } catch (err) {
          setError(errorToString(err));
        }
      }
      if (converted.length > 0) {
        setImages((prev) => [...prev, ...converted].slice(0, AI_MAX_IMAGES));
      }
    } finally {
      setImageBusy(false);
    }
  }, []);

  const removeImage = useCallback((id: string) => {
    setImages((prev) => prev.filter((img) => img.id !== id));
  }, []);

  const captureScreen = useCallback(async (mode: "screen" | "window" = "screen") => {
    if (capturingRef.current) return;
    if (imagesRef.current.length >= AI_MAX_IMAGES) {
      setError("image-limit");
      return;
    }

    setCapturing(true);
    setError(null);
    try {
      const dataUrl = await invoke<string>("ai_capture_screen", { mode });
      const img = dataUrlToAiImage(
        dataUrl,
        mode === "window" ? "window.jpg" : "screen.jpg"
      );
      setImages((prev) => [...prev, img].slice(0, AI_MAX_IMAGES));
    } catch (err) {
      setError(errorToString(err));
    } finally {
      setCapturing(false);
    }
  }, []);

  const dispatchAsk = useCallback(
    async (
      baseHistory: AiChatMessage[],
      userText: string,
      userImages: AiImage[]
    ) => {
      const activeProvider = providerRef.current;
      const requestId = nextId("req");
      const userMsg: AiChatMessage = {
        id: nextId("usr"),
        role: "user",
        text: userText,
        images: userImages.length > 0 ? userImages : undefined,
      };
      const assistantId = nextId("ast");
      const assistantPlaceholder: AiChatMessage = {
        id: assistantId,
        role: "assistant",
        text: "",
        streaming: true,
        provider: activeProvider,
      };

      const nextHistory = [...baseHistory, userMsg];
      setMessages([...nextHistory, assistantPlaceholder]);
      setError(null);
      setBusy(true);
      requestIdRef.current = requestId;
      assistantMsgIdRef.current = assistantId;
      requestedProviderRef.current = activeProvider;

      try {
        const reply = await invoke<string>("ai_ask", {
          requestId,
          messages: toWireMessages(nextHistory),
          provider: activeProvider,
          model: readStoredModel(activeProvider),
        });

        setMessages((prev) =>
          prev.map((m) =>
            m.id === assistantId ? { ...m, text: reply, streaming: false } : m
          )
        );
      } catch (err) {
        const errMsg = errorToString(err);
        if (errMsg === "cancelled") {
          setMessages((prev) => {
            const target = prev.find((m) => m.id === assistantId);
            if (!target || target.text.trim().length === 0) {
              return prev.filter((m) => m.id !== assistantId);
            }
            return prev.map((m) =>
              m.id === assistantId ? { ...m, streaming: false } : m
            );
          });
        } else {
          setError(errMsg);
          setMessages((prev) => {
            const target = prev.find((m) => m.id === assistantId);
            if (!target || target.text.trim().length === 0) {
              return prev.filter((m) => m.id !== assistantId);
            }
            return prev.map((m) =>
              m.id === assistantId ? { ...m, streaming: false } : m
            );
          });
        }
      } finally {
        if (requestIdRef.current === requestId) {
          requestIdRef.current = null;
          assistantMsgIdRef.current = null;
          setBusy(false);
        }
      }
    },
    []
  );

  const send = useCallback(
    async (overrideText?: string) => {
      if (busyRef.current) return;
      const text = (overrideText ?? promptRef.current).trim();
      const attached = imagesRef.current;
      if (!text && attached.length === 0) return;

      setPrompt("");
      setImages([]);
      await dispatchAsk(messagesRef.current, text, attached);
    },
    [dispatchAsk]
  );

  const retry = useCallback(async () => {
    if (busyRef.current) return;
    const current = messagesRef.current;
    let lastUserIdx = -1;
    for (let i = current.length - 1; i >= 0; i -= 1) {
      if (current[i].role === "user") {
        lastUserIdx = i;
        break;
      }
    }
    if (lastUserIdx < 0) return;

    const lastUser = current[lastUserIdx];
    const baseHistory = current.slice(0, lastUserIdx);
    await dispatchAsk(baseHistory, lastUser.text, lastUser.images ?? []);
  }, [dispatchAsk]);

  const cancel = useCallback(() => {
    const activeId = requestIdRef.current;
    if (!activeId) return;
    invoke<void>("ai_cancel", { requestId: activeId }).catch(() => {});
  }, []);

  const clear = useCallback(() => {
    if (requestIdRef.current) {
      invoke<void>("ai_cancel", { requestId: requestIdRef.current }).catch(() => {});
      requestIdRef.current = null;
      assistantMsgIdRef.current = null;
      setBusy(false);
    }
    setMessages([]);
    setPrompt("");
    setImages([]);
    setError(null);
    setNotice(null);
  }, []);

  const saveKey = useCallback(
    async (targetProvider: AiProviderId, key: string, sessionOnly = false) => {
      await invoke<void>("ai_set_key", {
        provider: targetProvider,
        key,
        sessionOnly,
      });
      await refreshStatus(false);
    },
    [refreshStatus]
  );

  const deleteKey = useCallback(
    async (targetProvider: AiProviderId) => {
      await invoke<void>("ai_delete_key", { provider: targetProvider });
      await refreshStatus(false);
    },
    [refreshStatus]
  );

  const openConsole = useCallback(async (targetProvider: AiProviderId) => {
    await invoke<void>("ai_open_console", { provider: targetProvider });
  }, []);

  return {
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
  };
}
