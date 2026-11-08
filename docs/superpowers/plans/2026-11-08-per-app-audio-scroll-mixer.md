# Per-App Audio Scroll Mixer Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Enable users to seamlessly adjust individual application volumes (e.g. Spotify, Discord, Chrome, games) by scrolling the mouse wheel directly over their dock icons, complete with a reactive dynamic mini-HUD indicator.

**Architecture:** 
- A dedicated native Windows Core Audio module in Rust (`src-tauri/src/app_audio.rs`) using `IAudioSessionManager2` and `ISimpleAudioVolume` to enumerate and control per-process audio sessions.
- Process-matching logic that handles multi-process apps (like Chrome, Spotify, Discord) by matching both PID and process image name (`.exe`).
- Tauri IPC commands exposed in `src-tauri/src/commands.rs`.
- Frontend event handling in `src/Dock.tsx` (`onWheel` and middle-click mute `onAuxClick`) with optimistic volume updates and trailing debounce.
- A floating glassmorphic mini-HUD badge (`.app-volume-hud`) in `src/Dock.css` showing the current volume level (`Volume2` / `VolumeX` icon, progress bar, percentage).
- Toggle switch and step options in Settings under `src/settings/DockTab.tsx` with full i18n support (`tr.json`, `en.json`).

**Tech Stack:** 
- Rust (Tauri v2, `windows` crate `Win32_Media_Audio`, COM API)
- React 19, TypeScript, Framer Motion
- Lucide React icons, Tailwind/CSS glassmorphism

---

## Global Constraints

- Never break existing Dock drag-and-drop, window preview hover, or click behaviors.
- Wheel events must `preventDefault` and `stopPropagation` only when adjusting volume to avoid interfering with parent scroll containers.
- Windows audio COM calls must be thread-safe (`CoInitializeEx` / MTA or Apartment threaded guard).
- All new user-facing copy must be localized in both English (`src/locales/en.json`) and Turkish (`src/locales/tr.json`).
- Adhere to Andrej Karpathy guidelines: surgical changes, zero speculative code, verifiable at each step.

---

## Tasks

### Task 1: Windows Core Audio Rust Module (`src-tauri/src/app_audio.rs`)

**Files:**
- Create: `src-tauri/src/app_audio.rs`
- Modify: `src-tauri/src/main.rs`
- Modify: `src-tauri/src/commands.rs`

**Interfaces:**
- Produces:
  - `pub struct AppVolumeState { pub volume: f32, pub muted: bool }`
  - `pub fn adjust_process_volume(target_exe: &str, delta: f32) -> Result<AppVolumeState, String>`
  - `pub fn get_process_volume(target_exe: &str) -> Result<Option<AppVolumeState>, String>`
  - `pub fn set_process_volume(target_exe: &str, volume: f32) -> Result<AppVolumeState, String>`
  - `pub fn toggle_process_mute(target_exe: &str) -> Result<AppVolumeState, String>`
  - Tauri commands: `adjust_app_volume`, `get_app_volume`, `set_app_volume`, `toggle_app_mute`

- [x] **Step 1: Implement `src-tauri/src/app_audio.rs`**
  - Implement Windows Core Audio COM initialization.
  - Enumerate sessions via `IMMDeviceEnumerator -> GetDefaultAudioEndpoint -> Activate::<IAudioSessionManager2> -> GetSessionEnumerator`.
  - For each session, inspect `IAudioSessionControl2::GetProcessId()`.
  - Match PID against the requested executable name (e.g. `spotify.exe` case-insensitively).
  - Cast to `ISimpleAudioVolume` to read/write master volume and mute state.
  - Support multi-session / multi-process apps: apply changes across all matching sessions of that executable.

- [x] **Step 2: Wire commands in `commands.rs` and `main.rs`**
  - Expose `#[tauri::command] pub fn adjust_target_app_volume(target: String, delta: f32, pids: Option<Vec<u32>>) -> Result<AppVolumeState, String>`
  - Expose `#[tauri::command] pub fn toggle_target_app_mute(target: String, pids: Option<Vec<u32>>) -> Result<AppVolumeState, String>`
  - Expose `#[tauri::command] pub fn get_target_app_volume(target: String, pids: Option<Vec<u32>>) -> Result<Option<AppVolumeState>, String>`
  - Expose `#[tauri::command] pub fn set_target_app_volume(target: String, volume: f32, pids: Option<Vec<u32>>) -> Result<AppVolumeState, String>`
  - Register in `tauri::generate_handler![...]` in `src-tauri/src/main.rs`.

- [x] **Step 3: Verification**
  - Run `cargo check --manifest-path src-tauri/Cargo.toml`.
  - Ensure zero warnings/errors.

---

### Task 2: Frontend Dock Volume Wheel & Mini-HUD (`src/Dock.tsx` & `src/Dock.css`)

**Files:**
- Modify: `src/Dock.tsx`
- Modify: `src/Dock.css`

**Interfaces:**
- Consumes: `adjust_target_app_volume`, `toggle_target_app_mute`, `get_target_app_volume`, `set_target_app_volume` from Tauri invoke.
- State: `volumeHuds: { [appKey: string]: { currentVol: number; targetVol: number; muted: boolean; visible: boolean } }`

- [x] **Step 1: Add fluid volume wheel listener & LERP engine on Dock App icons**
  - In `src/Dock.tsx`, add `onWheel` handler on dock app item container:
    - If `fern-dock-app-volume-scroll` is disabled, do nothing.
    - If app is running (`app.is_running`) and has an executable or path:
      - Continuous & Fluid Ramping: Eliminate discrete %5 blocks.
      - Calculate continuous delta from `e.deltaY` with momentum/velocity support (slow scrolling gives 1% precision, fast scroll accelerates smoothly).
      - Run a 60/120 FPS `requestAnimationFrame` LERP loop (`current += (target - current) * 0.22`) that drives both the visual progress bar and the audio volume.
      - Trailing throttle for Tauri IPC calls (~20ms) to ensure butter-smooth Windows Core Audio volume ramping without IPC flooding.
      - Show floating mini-HUD above the dock icon with fluidly moving level bar.
      - Auto-hide HUD after 1200ms of inactivity.

- [x] **Step 2: Style the Floating Mini-HUD in `src/Dock.css`**
  - Create `.dock-app-volume-hud`:
    - Positioned directly above the app icon (`bottom: calc(100% + 12px)`).
    - Compact glassmorphism pill (`background: rgba(18, 18, 22, 0.88)`, blur, border).
    - Contains: Volume icon (`Volume2` / `Volume1` / `VolumeX`), mini progress bar (height 3px, rounded), and percentage text.
    - Smooth entrance/exit animation with Framer Motion.

- [x] **Step 3: Verification**
  - Run `npm run build` to verify TypeScript and CSS compilation.

---

### Task 3: Settings UI & Localization (`src/settings/DockTab.tsx`, `useSettings.ts`, `locales/`)

**Files:**
- Modify: `src/settings/DockTab.tsx`
- Modify: `src/settings/useSettings.ts`
- Modify: `src/locales/en.json`
- Modify: `src/locales/tr.json`

- [x] **Step 1: Add settings keys & hook state**
  - `fern-dock-app-volume-scroll`: boolean (default `true`) - Enable per-app volume scroll.
  - Add getters, setters, and `useSettingsSync` handlers in `src/settings/useSettings.ts`.

- [x] **Step 2: Add Settings Row in DockTab.tsx**
  - Add a new `SettingRow` under Dock Settings with `Volume2` icon:
    - Title: "Uygulama Başına Ses Mikseri" / "Per-app volume scroll"
    - Description: "Dock simgeleri üzerinde fare tekerleğini kaydırarak uygulamanın sesini akıcı şekilde ayarlayın"
    - Toggle switch.

- [x] **Step 3: Add localization entries**
  - In `src/locales/tr.json`:
    - `"dock.appVolumeScroll": "Uygulama başına ses mikseri"`
    - `"dock.appVolumeScroll.desc": "Dock simgeleri üzerinde fare tekerleğini kaydırarak uygulamanın sesini akıcı şekilde ayarlayın"`
  - In `src/locales/en.json`:
    - `"dock.appVolumeScroll": "Per-app volume scroll"`
    - `"dock.appVolumeScroll.desc": "Scroll mouse wheel over running dock apps to fluidly adjust their individual volume"`

- [x] **Step 4: End-to-end Verification**
  - Run `cargo check --manifest-path src-tauri/Cargo.toml`.
  - Run `npm run build`.
  - Verify all builds pass cleanly.
