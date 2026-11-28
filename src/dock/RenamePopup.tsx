import type React from 'react';
import { useState, useEffect, useRef } from 'react';
import { motion } from 'framer-motion';
import { invoke } from '@tauri-apps/api/core';
import { useT, localizeAppName } from '../i18n';
import type { AppInfo } from './types';

export function RenamePopup({
  app,
  current,
  containerRef,
  scale,
  bottom,
  onSave,
  onClose,
}: {
  app: AppInfo;
  current: string;
  containerRef: React.RefObject<HTMLDivElement | null>;
  scale: number;
  bottom: number;
  onSave: (name: string) => void;
  onClose: () => void;
}) {
  const t = useT();
  const [value, setValue] = useState(current || localizeAppName(app.name));
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    invoke('set_window_keyboard_focus', { enabled: true })
      .catch(() => {})
      .finally(() => {
        inputRef.current?.focus();
        inputRef.current?.select();
      });
    return () => {
      invoke('set_window_keyboard_focus', { enabled: false }).catch(() => {});
    };
  }, []);

  return (
    <div className="add-popup-anchor" style={{ zoom: scale, bottom }}>
      <motion.div
        ref={containerRef}
        className="add-app-popup rename-popup"
        style={{ transformOrigin: "bottom center" }}
        initial={{ opacity: 0, scaleY: 0 }}
        animate={{ opacity: 1, scaleY: 1 }}
        exit={{ opacity: 0, scaleY: 0, transition: { duration: 0.16, ease: "easeIn" } }}
        transition={{ opacity: { duration: 0.15 }, scaleY: { type: "spring", stiffness: 300, damping: 28, mass: 0.9 } }}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="popup-search-row">
          <input
            ref={inputRef}
            type="text"
            className="popup-search-input"
            value={value}
            maxLength={40}
            onChange={(e) => setValue(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") onSave(value);
              if (e.key === "Escape") onClose();
            }}
          />
        </div>
        <div className="rename-actions">
          {current && (
            <button type="button" className="rename-btn" onClick={() => onSave("")}>
              {t("dockmenu.renameReset")}
            </button>
          )}
          <button type="button" className="rename-btn primary" onClick={() => onSave(value)}>
            {t("dockmenu.renameSave")}
          </button>
        </div>
      </motion.div>
    </div>
  );
}
