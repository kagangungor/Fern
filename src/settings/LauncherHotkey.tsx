import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { AlertTriangle, Keyboard } from "lucide-react";
import { SettingRow } from "./SettingRow";
import { ShortcutRecorder } from "./ShortcutRecorder";
import { useT } from "../i18n";

export function LauncherHotkey() {
  const t = useT();
  const [value, setValue] = useState(() => {
    const saved = localStorage.getItem("fern-launcher-hotkey");
    return saved !== null ? saved : "Alt+Space";
  });
  const [conflict, setConflict] = useState(false);
  const timerRef = useRef<number | null>(null);
  const mountedRef = useRef(true);

  useEffect(() => {
    mountedRef.current = true;
    if (value) {
      invoke<boolean>("get_launcher_hotkey_status")
        .then((ok) => {
          if (mountedRef.current) setConflict(!ok);
        })
        .catch(() => {});
    }
    return () => {
      mountedRef.current = false;
      if (timerRef.current !== null) {
        window.clearTimeout(timerRef.current);
      }
    };
  }, []);

  const commit = (next: string) => {
    setValue(next);
    localStorage.setItem("fern-launcher-hotkey", next);
    if (!next) {
      setConflict(false);
    }
    invoke("save_setting", { key: "fern-launcher-hotkey", value: next })
      .catch(() => {})
      .finally(() => {
        invoke("reload_hotkeys")
          .catch(() => {})
          .finally(() => {
            if (!next) return;
            if (timerRef.current !== null) {
              window.clearTimeout(timerRef.current);
            }
            timerRef.current = window.setTimeout(() => {
              invoke<boolean>("get_launcher_hotkey_status")
                .then((ok) => {
                  if (mountedRef.current) setConflict(!ok);
                })
                .catch(() => {});
            }, 80);
          });
      });
  };

  const showConflict = conflict && Boolean(value);

  return (
    <>
      <SettingRow
        icon={Keyboard}
        label={t("dock.launcherHotkey")}
        desc={t("dock.launcherHotkey.desc")}
        divider={!showConflict}
      >
        <ShortcutRecorder value={value} onCommit={commit} />
      </SettingRow>
      {showConflict && (
        <>
          <div
            style={{
              display: "flex",
              alignItems: "center",
              gap: "6px",
              padding: "0 16px 10px 44px",
              fontSize: "12px",
              color: "#f59e0b",
            }}
          >
            <AlertTriangle size={13} strokeWidth={1.75} style={{ flexShrink: 0 }} />
            <span>{t("dock.launcherHotkey.conflict")}</span>
          </div>
          <div className="setting-divider" />
        </>
      )}
    </>
  );
}
