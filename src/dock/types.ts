import type React from 'react';

export interface AppInfo {
  name: string;
  path: string;
  icon: string | null;
  is_running: boolean;
  is_pinned?: boolean;
  hwnd?: number;
  executable?: string;
  all_hwnds?: [number, string][];
  args?: string;
}

export interface AppMediaSession {
  app_id: string;
  title: string;
  artist: string;
  is_playing: boolean;
}

export interface RecycleBinStatus {
  is_empty: boolean;
  item_count: number;
  size_bytes: number;
}

export interface DockDrive {
  letter: string;
  label: string;
  removable: boolean;
}

export type CommActionKind = 'camera' | 'mic' | 'deafen' | 'disconnect';

export interface CommThumbarAction {
  index: number;
  kind: CommActionKind;
  labelKey: string;
  danger?: boolean;
}

export interface CommThumbarProfile {
  matchTokens: string[];
  actions: CommThumbarAction[];
}

export interface CommToggleState {
  cameraOn: boolean;
  micMuted: boolean;
  deafened: boolean;
  micBeforeDeafen: boolean;
}

export interface DockPreviewTooltipProps {
  app: AppInfo;
  previews: { hwnd: number; title: string; image: string }[];
  capturePending?: boolean;
  onClosePreview: (e: React.MouseEvent, hwnd: number) => void;
  onSelectPreview: (hwnd: number) => void;
  renderMedia: (app: AppInfo) => React.ReactNode;
  renderPreviewMedia?: (app: AppInfo) => React.ReactNode;
  onMouseEnter: () => void;
  onMouseLeave: () => void;
}
