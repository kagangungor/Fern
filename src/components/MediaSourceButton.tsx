import { invoke } from "@tauri-apps/api/core";
import { useT } from "../i18n";

export interface MediaSession {
  id: string;
  name: string;
  title: string;
  is_playing: boolean;
  active: boolean;
}

/** Shows the app the player follows; a click moves on to the next app with a media session. */
export function MediaSourceButton({ sessions }: { sessions: MediaSession[] }) {
  const t = useT();
  if (sessions.length < 2) return null;
  const index = Math.max(0, sessions.findIndex((s) => s.active));
  const current = sessions[index];
  const next = sessions[(index + 1) % sessions.length];
  return (
    <button
      className="media-source-btn"
      title={`${current.name} (${index + 1}/${sessions.length}) → ${next.name}`}
      aria-label={t("media.switchSource", { name: next.name })}
      onClick={(e) => {
        e.stopPropagation();
        invoke("select_media_session", { id: next.id }).catch(() => {});
      }}
    >
      {current.is_playing && <span className="media-source-dot" />}
      <span className="media-source-name">{current.name}</span>
    </button>
  );
}
