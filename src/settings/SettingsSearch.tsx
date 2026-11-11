import { useState, useRef, useEffect, useCallback } from "react";
import { Search, X, ChevronRight } from "lucide-react";
import { searchSettings, type SearchResult } from "./searchEngine";
import type { SettingsTab } from "./types";
import { useT } from "../i18n";

interface SettingsSearchProps {
  onNavigate: (tab: SettingsTab, sectionId?: string) => void;
}

export function SettingsSearch({ onNavigate }: SettingsSearchProps) {
  const t = useT();
  const [query, setQuery] = useState("");
  const [isOpen, setIsOpen] = useState(false);
  const [selectedIndex, setSelectedIndex] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);

  const results = searchSettings(query, t);

  const handleSelect = useCallback(
    (res: SearchResult) => {
      onNavigate(res.setting.tab, res.setting.sectionId);
      setIsOpen(false);
      setQuery("");
      inputRef.current?.blur();
    },
    [onNavigate]
  );

  // Global Ctrl + F / Cmd + F handler
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "f") {
        e.preventDefault();
        inputRef.current?.focus();
        setIsOpen(true);
      } else if (e.key === "Escape" && isOpen) {
        setIsOpen(false);
        inputRef.current?.blur();
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [isOpen]);

  // Outside click handler
  useEffect(() => {
    const handleClickOutside = (e: MouseEvent) => {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        setIsOpen(false);
      }
    };
    document.addEventListener("mousedown", handleClickOutside);
    return () => document.removeEventListener("mousedown", handleClickOutside);
  }, []);

  // Keyboard navigation inside dropdown
  const onInputKeyDown = (e: React.KeyboardEvent) => {
    if (!isOpen || results.length === 0) return;

    if (e.key === "ArrowDown") {
      e.preventDefault();
      setSelectedIndex((prev) => (prev + 1) % results.length);
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setSelectedIndex((prev) => (prev - 1 + results.length) % results.length);
    } else if (e.key === "Enter") {
      e.preventDefault();
      if (results[selectedIndex]) {
        handleSelect(results[selectedIndex]);
      }
    }
  };

  return (
    <div className="settings-search-container" ref={containerRef}>
      <div className="settings-search-box">
        <Search size={14} className="settings-search-icon" />
        <input
          ref={inputRef}
          type="text"
          className="settings-search-input"
          placeholder={t("settings.searchPlaceholder")}
          value={query}
          onChange={(e) => {
            setQuery(e.target.value);
            setIsOpen(true);
            setSelectedIndex(0);
          }}
          onFocus={() => {
            if (query.trim().length > 0) setIsOpen(true);
          }}
          onKeyDown={onInputKeyDown}
        />
        {query ? (
          <button
            className="settings-search-clear"
            onClick={() => {
              setQuery("");
              setIsOpen(false);
              inputRef.current?.focus();
            }}
          >
            <X size={12} />
          </button>
        ) : (
          <span className="settings-search-shortcut">Ctrl+F</span>
        )}
      </div>

      {isOpen && query.trim().length > 0 && (
        <div className="settings-search-dropdown">
          {results.length > 0 ? (
            results.map((res, idx) => (
              <div
                key={res.setting.id}
                className={`settings-search-result-item ${idx === selectedIndex ? "active" : ""}`}
                onClick={() => handleSelect(res)}
                onMouseEnter={() => setSelectedIndex(idx)}
              >
                <div className="settings-search-result-info">
                  <div className="settings-search-result-header">
                    <span className="settings-search-result-title">{res.title}</span>
                    <span className="settings-search-result-tab">{t(`tabs.${res.setting.tab}`)}</span>
                  </div>
                  {res.desc && <div className="settings-search-result-desc">{res.desc}</div>}
                </div>
                <ChevronRight size={14} className="settings-search-result-arrow" />
              </div>
            ))
          ) : (
            <div className="settings-search-empty">{t("settings.searchEmpty")}</div>
          )}
        </div>
      )}
    </div>
  );
}
