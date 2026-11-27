export interface SearchApp {
  name: string;
  path: string;
  icon: string | null;
  is_running: boolean;
  hwnd?: number;
  args?: string;
}

export interface FileSearchResult {
  name: string;
  path: string;
  parent_dir: string;
  is_dir: boolean;
  extension: string | null;
}

export interface QuickCommand {
  id: string;
  labelKey: string;
  keywords: string[];
  icon:
    | "settings"
    | "calendar"
    | "mic"
    | "sparkles"
    | "rotate"
    | "activity"
    | "monitor"
    | "lock"
    | "moon"
    | "trash";
  confirmRequired?: boolean;
  run: () => Promise<void> | void;
}

export type WebEngine = "google" | "duckduckgo";

export type LauncherItem =
  | { kind: "calculator"; id: string; expression: string; value: string }
  | { kind: "url"; id: string; url: string; display: string }
  | { kind: "app"; id: string; app: SearchApp }
  | { kind: "command"; id: string; command: QuickCommand; label: string }
  | { kind: "file"; id: string; file: FileSearchResult }
  | { kind: "web"; id: string; query: string; engine: WebEngine; url: string };
