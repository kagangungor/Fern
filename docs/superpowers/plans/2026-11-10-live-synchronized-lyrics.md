# Live Synchronized Lyrics (Karaoke Mode) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Enable real-time synchronized lyrics (Karaoke Mode) inside Fern's Music Notch, matching the playing track from Windows GSMTC (Spotify, Apple Music, YouTube Music, web browsers) with smooth line-by-line auto-scrolling, click-to-seek, and full zero-regression backward compatibility.

**Architecture:** 
1. **Rust Backend (`src-tauri/src/lyrics.rs`):** An asynchronous, cached HTTP client querying the free [LRCLIB](https://lrclib.net/) API. Includes title/artist sanitization (removing tags like `(feat. ...)`, `[Remastered]`, `Official Audio`), LRC parser producing millisecond timestamps, and disk/memory caching in the app cache directory.
2. **Tauri IPC (`src-tauri/src/commands.rs` & `main.rs`):** `get_lyrics` command exposed to frontend.
3. **Frontend Lyrics Engine (`src/components/SyncedLyricsView.tsx`):** A glassmorphic karaoke view displaying live synchronized lines. Highlights the active line with accent color and full opacity, softly dims adjacent lines, auto-scrolls smoothly to keep the current singing line centered, and supports click-to-seek.
4. **Music Player Integration (`src/CompactMediaPlayer.tsx` & `src/App.tsx`):** A toggle button in the player controls/header. Smooth height/width spring expansion when lyrics are opened, with instantaneous restoration of the original layout when closed.
5. **Settings & Localization:** Toggle in `src/settings/NotchTab.tsx`, localized in English (`en.json`) and Turkish (`tr.json`).

**Tech Stack:** Rust (Tauri v2, `reqwest` with native TLS, `serde`), React 19, TypeScript, Framer Motion, Lucide React icons, Vanilla CSS.

---

## Global Constraints & Non-Negotiables
- **Zero Regression:** When lyrics mode is closed or disabled, all existing music player controls, album art animations, progress bar, audio visualizer, volume slider, and notch dimensions MUST behave exactly as they currently do.
- **Offline & Fallback Safety:** If a track has no synced lyrics or network is offline, display a clean, non-intrusive status ("No lyrics found" / "Plain lyrics available") without breaking player responsiveness or causing UI hangs.
- **Thread Safety & Performance:** Network requests must run on async background tasks (`tokio::spawn` or `tauri::async_runtime::spawn_blocking`), never blocking the main UI thread, audio polling loop, or DWM compositor.
- **Karpathy Principles:** Surgical changes, zero unnecessary dependencies (use existing `reqwest` already in `Cargo.toml`), verifiable success criteria at each step.

---

## Tasks

### Task 1: Rust Backend Lyrics Service (`src-tauri/src/lyrics.rs`)

**Files:**
- Create: `src-tauri/src/lyrics.rs`
- Modify: `src-tauri/src/main.rs`
- Modify: `src-tauri/src/commands.rs`

**Interfaces:**
- Produces:
  ```rust
  #[derive(Serialize, Deserialize, Clone, Debug)]
  pub struct LyricLine {
      pub time_ms: u64,
      pub text: String,
  }

  #[derive(Serialize, Deserialize, Clone, Debug)]
  pub struct LyricsPayload {
      pub synced: bool,
      pub lines: Vec<LyricLine>,
      pub plain: Option<String>,
      pub source: String,
  }

  pub async fn fetch_lyrics(app: &AppHandle, title: &str, artist: &str, duration_ms: Option<u64>) -> Result<LyricsPayload, String>;
  ```
- Tauri Command:
  ```rust
  #[tauri::command]
  pub async fn get_lyrics(app: AppHandle, title: String, artist: String, duration_ms: Option<u64>) -> Result<Option<LyricsPayload>, String>;
  ```

- [x] **Step 1: Implement `src-tauri/src/lyrics.rs`**
  - Implement regex / string cleaner for `title`:
    - Strip parentheses containing `feat.`, `ft.`, `Remastered`, `Live`, `Bonus Track`, `Official Video`, `Radio Edit`.
  - Cache lookup: Check local disk cache in `{app_cache_dir}/lyrics_cache/{sanitized_hash}.json`. If present and valid, return immediately.
  - Query LRCLIB:
    - Primary endpoint: `GET https://lrclib.net/api/get?artist_name={artist}&track_name={cleaned_title}&duration={duration_secs}`.
    - Fallback endpoint (if 404): `GET https://lrclib.net/api/search?track_name={cleaned_title}&artist_name={artist}`.
  - Parse `syncedLyrics` LRC format:
    - Parse lines matching `^\[(\d{2}):(\d{2})\.(\d{2,3})\](.*)$` into `time_ms = (min * 60 + sec) * 1000 + ms` and `text`.
    - If `syncedLyrics` is null/empty but `plainLyrics` exists, set `synced: false` and populate `plain`.
  - Cache result to disk and return `LyricsPayload`.
- [x] **Step 2: Wire `get_lyrics` command in `commands.rs` and `main.rs`**
  - Register `get_lyrics` in `generate_handler![..., get_lyrics]`.
- [x] **Step 3: Verification**
  - Run `cargo check --manifest-path src-tauri/Cargo.toml` and verify compilation succeeds.

---

### Task 2: Synchronized Lyrics UI Component (`src/components/SyncedLyricsView.tsx` & `.css`)

**Files:**
- Create: `src/components/SyncedLyricsView.tsx`
- Create: `src/components/SyncedLyricsView.css`

**Interfaces:**
- Props:
  ```ts
  interface SyncedLyricsViewProps {
    lines: Array<{ time_ms: number; text: string }>;
    currentTimeMs: number;
    synced: boolean;
    plain?: string;
    loading: boolean;
    onSeek?: (timeMs: number) => void;
    onClose?: () => void;
  }
  ```

- [x] **Step 1: Implement `src/components/SyncedLyricsView.tsx`**
  - Binary search or `findLastIndex` for `currentTimeMs`:
    - Find the active line where `lines[i].time_ms <= currentTimeMs < lines[i + 1]?.time_ms`.
  - Scroll container with `useRef<HTMLDivElement>`:
    - Automatically center the active line using `activeEl.scrollIntoView({ behavior: 'smooth', block: 'center' })` (with a flag to disable auto-scroll temporarily if the user manually scrolls).
  - Click-to-seek:
    - Clicking on any line calls `onSeek(line.time_ms)`.
  - Empty / Loading states:
    - Graceful pulsing skeleton loader while loading.
    - "No lyrics found" with subtle music note icon if empty.
    - Plain lyrics scrollable text if `synced === false`.
- [x] **Step 2: Create styling in `SyncedLyricsView.css`**
  - Glassmorphic backdrop with soft blur and gentle gradient masks at top and bottom (`mask-image: linear-gradient(...)`).
  - Active line: 100% opacity, larger font weight (600), subtle scale (`scale: 1.02`), accent color or bright white text with soft text-shadow.
  - Inactive lines: 35-45% opacity, smooth font transition.
  - Hover states: cursor pointer, slight highlight on hover to encourage click-to-seek.
- [x] **Step 3: Verification**
  - Verify TypeScript types compile cleanly with `bun run build`.

---

### Task 3: Music Player Integration (`src/CompactMediaPlayer.tsx` & `src/App.tsx`)

**Files:**
- Modify: `src/CompactMediaPlayer.tsx`
- Modify: `src/App.tsx`
- Modify: `src/App.css`

- [x] **Step 1: Integrate Lyrics Toggle in `src/CompactMediaPlayer.tsx`**
  - Add lyrics toggle button in Row 1 (beside Calendar/Source buttons) or Row 3 using `ListMusic` or `Quote` icon from `lucide-react`.
  - Tooltip localized: `t("media.lyrics")`.
  - Manage state: `isLyricsOpen` (persisted in `localStorage.getItem("fern-notch-lyrics-open")`).
  - When `isLyricsOpen`:
    - Fetch lyrics on track change via `invoke('get_lyrics', { title: mediaInfo.title, artist: mediaInfo.artist, durationMs: mediaInfo.duration_ms })`.
    - Cache in React state so re-toggling or seeking does not refetch.
    - Render `<SyncedLyricsView />` smoothly inside the player body.
  - Propagate `isLyricsOpen` to parent `App.tsx` via `onLyricsToggle?: (open: boolean) => void`.
- [x] **Step 2: Dynamic Notch Dimension Expansion in `src/App.tsx`**
  - In `getDynamicWidth()`:
    - When `isMusicMode && isHovered && isLyricsOpen`: Expand width to `380px` (or `400px`) for spacious, elegant reading without text clipping.
  - In `getDynamicHeight()`:
    - When `isMusicMode && isHovered && isLyricsOpen`: Expand height to `240px` (accommodating 3-4 visible karaoke lines + controls).
  - When `isLyricsOpen === false`: Dimensions revert strictly to current values (`300px` / `132px`).
- [x] **Step 3: Verification**
  - Verify no layout shift when opening and closing lyrics.
  - Verify all standard media controls (play, pause, next, prev, volume) continue operating normally while lyrics are active.

---

### Task 4: Settings & Internationalization

**Files:**
- Modify: `src/settings/NotchTab.tsx`
- Modify: `src/locales/en.json`
- Modify: `src/locales/tr.json`

- [x] **Step 1: Add Settings Toggle in `src/settings/NotchTab.tsx`**
  - Under the Music Settings card:
    - Toggle: "Live Synchronized Lyrics (Karaoke)" (`fern-lyrics-enabled`, default: `true`).
    - Sub-toggle: "Click lyrics to seek" (`fern-lyrics-click-to-seek`, default: `true`).
- [x] **Step 2: Add Localized Strings**
  - `en.json`:
    - `"media.lyrics": "Lyrics"`
    - `"media.lyricsLoading": "Searching lyrics..."`
    - `"media.noLyrics": "No lyrics available for this track"`
    - `"settings.notch.lyrics": "Live Synchronized Lyrics"`
    - `"settings.notch.lyricsDesc": "Stream real-time karaoke lyrics in the music notch with click-to-seek"`
  - `tr.json`:
    - `"media.lyrics": "Şarkı Sözleri"`
    - `"media.lyricsLoading": "Şarkı sözleri aranıyor..."`
    - `"media.noLyrics": "Bu parça için şarkı sözü bulunamadı"`
    - `"settings.notch.lyrics": "Canlı Senkronize Şarkı Sözleri"`
    - `"settings.notch.lyricsDesc": "Müzik çentiğinde anlık akan karaoke şarkı sözleri ve tıklayarak parçayı sarma"`
- [x] **Step 3: Verification**
  - Verify settings toggle saves and reads from `localStorage`.
  - Verify switching between English and Turkish updates all labels correctly.

---

### Task 5: End-to-End Build & Functional Verification

- [x] **Step 1: Full Rust and Frontend Build**
  - Run `cargo check --manifest-path src-tauri/Cargo.toml`
  - Run `bun run build`
- [x] **Step 2: Playback & Sync Testing**
  - Verify active line highlights as playback position advances.
  - Click on a line in the future or past: verify playback position seeks cleanly.
  - Close lyrics view: verify notch returns smoothly to original compact music view.
