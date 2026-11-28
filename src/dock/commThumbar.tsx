import { Video, VideoOff, Mic, MicOff, Headphones, HeadphoneOff, PhoneOff } from 'lucide-react';
import type { AppInfo, CommActionKind, CommThumbarProfile, CommToggleState } from './types';

export const COMM_THUMBAR_PROFILES: CommThumbarProfile[] = [
  {
    matchTokens: ['discord', 'vesktop'],
    actions: [
      { index: 0, kind: 'camera', labelKey: 'dockpreview.camera' },
      { index: 1, kind: 'mic', labelKey: 'dockpreview.mic' },
      { index: 2, kind: 'deafen', labelKey: 'dockpreview.deafen' },
      { index: 3, kind: 'disconnect', labelKey: 'dockpreview.disconnect', danger: true },
    ],
  },
];

export function commProfileForApp(app: AppInfo): CommThumbarProfile | null {
  const haystack = `${app.executable ?? ''} ${app.path ?? ''} ${app.name ?? ''}`.toLowerCase();
  return (
    COMM_THUMBAR_PROFILES.find((profile) =>
      profile.matchTokens.some((token) => haystack.includes(token))
    ) ?? null
  );
}

export const DEFAULT_COMM_TOGGLE: CommToggleState = {
  cameraOn: false,
  micMuted: false,
  deafened: false,
  micBeforeDeafen: false,
};

export const commToggleStateByHwnd = new Map<number, CommToggleState>();

export function nextCommToggleState(prev: CommToggleState, kind: CommActionKind): CommToggleState {
  if (kind === 'camera') {
    return { ...prev, cameraOn: !prev.cameraOn };
  }
  if (kind === 'mic') {
    if (prev.deafened) {
      return { ...prev, micMuted: false, deafened: false, micBeforeDeafen: false };
    }
    const nextMuted = !prev.micMuted;
    return { ...prev, micMuted: nextMuted, deafened: false, micBeforeDeafen: nextMuted };
  }
  if (kind === 'deafen') {
    if (!prev.deafened) {
      return { ...prev, micMuted: true, deafened: true, micBeforeDeafen: prev.micMuted };
    }
    return { ...prev, micMuted: prev.micBeforeDeafen, deafened: false, micBeforeDeafen: false };
  }
  if (kind === 'disconnect') {
    return { ...DEFAULT_COMM_TOGGLE };
  }
  return prev;
}

export function CommActionIcon({ kind, toggled }: { kind: CommActionKind; toggled: boolean }) {
  switch (kind) {
    case 'camera':
      return toggled ? <Video size={14} /> : <VideoOff size={14} />;
    case 'mic':
      return toggled ? <MicOff size={14} /> : <Mic size={14} />;
    case 'deafen':
      return toggled ? <HeadphoneOff size={14} /> : <Headphones size={14} />;
    case 'disconnect':
      return <PhoneOff size={14} />;
  }
}
