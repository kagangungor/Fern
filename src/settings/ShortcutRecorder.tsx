import { useEffect, useRef, useState } from "react";
import { X } from "lucide-react";
import { useT } from "../i18n";

const NAMED: Record<string, string> = {
  Space: "Space", Enter: "Enter", NumpadEnter: "Enter", Tab: "Tab", Backspace: "Backspace", Delete: "Delete", Insert: "Insert",
  Home: "Home", End: "End", PageUp: "PageUp", PageDown: "PageDown",
  ArrowUp: "Up", ArrowDown: "Down", ArrowLeft: "Left", ArrowRight: "Right",
  Backquote: "`", Minus: "-", Equal: "=", Comma: ",", Period: ".", Slash: "/",
  Semicolon: ";", Quote: "'", BracketLeft: "[", BracketRight: "]", Backslash: "\\",
  NumpadAdd: "NumpadAdd", NumpadSubtract: "NumpadSubtract", NumpadMultiply: "NumpadMultiply",
  NumpadDivide: "NumpadDivide", NumpadDecimal: "NumpadDecimal",
};

function keyName(code: string): string | null {
  if (/^Key[A-Z]$/.test(code)) return code.slice(3);
  if (/^Digit[0-9]$/.test(code)) return code.slice(5);
  if (/^Numpad[0-9]$/.test(code)) return code;
  if (/^F([1-9]|1\d|2[0-4])$/.test(code)) return code;
  return NAMED[code] ?? null;
}

const MODIFIER_CODES = ["ControlLeft", "ControlRight", "AltLeft", "AltRight", "ShiftLeft", "ShiftRight", "MetaLeft", "MetaRight"];

function modifiersOf(e: KeyboardEvent): string[] {
  return [e.metaKey && "Win", e.ctrlKey && "Ctrl", e.altKey && "Alt", e.shiftKey && "Shift"].filter(Boolean) as string[];
}

export function ShortcutRecorder({ value, onCommit }: { value: string; onCommit: (shortcut: string) => void }) {
  const t = useT();
  const [recording, setRecording] = useState(false);
  const [held, setHeld] = useState<string[]>([]);
  const [hint, setHint] = useState<string | null>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const onCommitRef = useRef(onCommit);
  onCommitRef.current = onCommit;

  const stop = () => {
    setRecording(false);
    setHeld([]);
    setHint(null);
  };

  useEffect(() => {
    if (!recording) return;

    const handleKeyDown = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.repeat) return;

      if (e.code === "Escape") {
        stop();
        return;
      }

      if (MODIFIER_CODES.includes(e.code)) {
        setHint(null);
        setHeld(modifiersOf(e));
        return;
      }

      const key = keyName(e.code);
      const mods = modifiersOf(e);
      const isFunctionKey = /^F\d+$/.test(key ?? "");
      if (!key) {
        setHint("?");
        return;
      }
      if (mods.length === 0 && !isFunctionKey) {
        setHint("Ctrl / Alt / Shift + …");
        return;
      }
      stop();
      onCommitRef.current([...mods, key].join("+"));
    };

    const handleKeyUp = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      setHeld(modifiersOf(e));
    };

    const handlePointerDown = (e: PointerEvent) => {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        stop();
      }
    };

    window.addEventListener("keydown", handleKeyDown, { capture: true });
    window.addEventListener("keyup", handleKeyUp, { capture: true });
    window.addEventListener("pointerdown", handlePointerDown, { capture: true });

    return () => {
      window.removeEventListener("keydown", handleKeyDown, { capture: true });
      window.removeEventListener("keyup", handleKeyUp, { capture: true });
      window.removeEventListener("pointerdown", handlePointerDown, { capture: true });
    };
  }, [recording]);

  return (
    <div ref={containerRef} className="shortcut-recorder">
      <button
        type="button"
        className={`shortcut-input shortcut-record${recording ? " recording" : ""}${recording && hint ? " invalid" : ""}`}
        onClick={() => (recording ? stop() : setRecording(true))}
      >
        {recording
          ? hint ?? (held.length ? `${held.join("+")}+…` : t("shortcut.recording"))
          : value || t("shortcut.none")}
      </button>
      {value && !recording && (
        <button type="button" className="shortcut-clear" onClick={() => onCommit("")} aria-label={t("shortcut.clear")}>
          <X size={12} />
        </button>
      )}
    </div>
  );
}
