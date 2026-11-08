# Fern v1.3.0 / Nectar v1.9.3 Feature Sync Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Integrate the upstream Nectar v1.9.1 and v1.9.3 architectural improvements, performance enhancements, and user features into Fern while preserving Fern's branding and stability.

**Architecture:** 
1. Replace Tauri's registry-based autostart with a native Windows Task Scheduler logon task (`schtasks`), adding startup benchmark timing and dev-build safety guards.
2. Extend system background workers (`services.rs`) with media artwork retry loop, Game Pass launcher filter, and optional console window visibility.
3. Enhance the UI with week start day configuration, low-battery notch flip animation, and automatic agent hooks restoration.

**Tech Stack:** Rust (Tauri v2, Windows Win32 API, `schtasks`), TypeScript, React 19, Framer Motion, Tailwind CSS / Vanilla CSS.

## Global Constraints
- Product identity must remain **Fern** (use `fern-` prefix for all settings keys and `Fern` for task/process names).
- Touch only what is strictly necessary (Karpathy guideline: surgical changes).
- All changes must compile cleanly (`bun run build` and `cargo check`).
- Maintain full Turkish (`tr.json`) and English (`en.json`) localization coverage.

---

### Task 1: Native Windows Logon Task & Startup Benchmark (`startup.rs`)

**Files:**
- Create: `src-tauri/src/startup.rs`
- Modify: `src-tauri/src/main.rs`
- Modify: `src-tauri/src/commands.rs`
- Modify: `src/App.tsx`
- Modify: `src/settings/useSettings.ts`

**Interfaces:**
- Produces: `startup::begin()`, `startup::mark(label: &str)`, `startup::settle(reason: &str)`, `startup::migrate(app: &AppHandle)`, Tauri commands `get_autostart`, `set_autostart`.

- [ ] **Step 1: Create `src-tauri/src/startup.rs`**
  Implement Windows Task Scheduler logon task management via `schtasks`, priority management (`ABOVE_NORMAL_PRIORITY_CLASS` with 20s timeout and settle to `NORMAL_PRIORITY_CLASS`), benchmark logging via `crate::diagnostics::log`, and v1.9.3 guards (`is_build_output()` and `task_command()` path migration).
- [ ] **Step 2: Wire up `startup` in `src-tauri/src/main.rs`**
  Add `mod startup;`. Call `startup::begin()` at start of `main()`. Call `startup::migrate(&app.handle())` in Tauri setup hook. Expose `get_autostart` and `set_autostart` in `generate_handler!`.
- [ ] **Step 3: Add benchmark settle points in `src-tauri/src/commands.rs`**
  In `init_dock`: record `crate::startup::mark("dock shown")` and `crate::startup::settle("dock shown")`.
- [ ] **Step 4: Update frontend autostart calls**
  Update `src/App.tsx` (first run check) and `src/settings/useSettings.ts` (`toggleAutostart`, `checkAutostart`) to use `invoke("get_autostart")` and `invoke("set_autostart", { enabled })`.
- [ ] **Step 5: Verify Rust compilation and autostart IPC**
  Run `cargo check --manifest-path src-tauri/Cargo.toml` and verify no errors.

---

### Task 2: Claude Code / AI Agent Kancalarını Açılışta Otomatik Doğrulama

**Files:**
- Modify: `src-tauri/src/agent_api.rs:364-370`

**Interfaces:**
- Consumes: `set_hooks(true)` in `agent_api.rs`.

- [ ] **Step 1: Add background hook check on startup**
  In `agent_api::start(app: AppHandle)`: immediately after validating `enabled == true`, spawn a background thread: `std::thread::spawn(|| set_hooks(true));`.
- [ ] **Step 2: Verify compilation**
  Run `cargo check --manifest-path src-tauri/Cargo.toml`.

---

### Task 3: Background Service Filters & Artwork Retry Loop (`services.rs` & `utils.rs`)

**Files:**
- Modify: `src-tauri/src/utils.rs`
- Modify: `src-tauri/src/services.rs`

**Interfaces:**
- Produces: `setting_flag(key: &str) -> bool` in `utils.rs`.
- Consumes: Setting key `fern-dock-console-windows`.

- [ ] **Step 1: Add `setting_flag` helper in `src-tauri/src/utils.rs`**
  Read boolean setting from `SETTINGS_CACHE` safely without locks panicking.
- [ ] **Step 2: Implement Artwork Retry Loop in `services.rs`**
  In `setup_system_worker`: track `art_attempts: (u64, u32)`. When playback starts or track changes, if artwork is missing, retry every poll for first 8 polls, then every 15 polls.
- [ ] **Step 3: Filter Xbox Game Pass helper window (`gamingservicesui.exe`)**
  In `enum_windows_proc`: add `|| lowercase_path.ends_with("gamingservicesui.exe")` to exclusion list.
- [ ] **Step 4: Allow console windows when `fern-dock-console-windows` is enabled**
  In `enum_windows_proc`: allow `conhost.exe` if `window_class == "ConsoleWindowClass" && crate::utils::setting_flag("fern-dock-console-windows")`. Set app name to window `title` for conhost.
- [ ] **Step 5: Verify Rust backend compilation**
  Run `cargo check --manifest-path src-tauri/Cargo.toml`.

---

### Task 4: Console Windows Setting in Dock Settings

**Files:**
- Modify: `src/settings/useSettings.ts`
- Modify: `src/settings/DockTab.tsx`
- Modify: `src/locales/en.json`
- Modify: `src/locales/tr.json`

**Interfaces:**
- Produces: `dockConsoleWindows: boolean`, `toggleDockConsoleWindows: () => void`.

- [ ] **Step 1: Add state and toggle in `src/settings/useSettings.ts`**
  Add `dockConsoleWindows` with localStorage key `fern-dock-console-windows`, sync via `useSettingsSync`.
- [ ] **Step 2: Add setting row in `src/settings/DockTab.tsx`**
  Add toggle row with `Terminal` icon from `lucide-react`, title `t("dock.consoleWindows")`, description `t("dock.consoleWindows.desc")`.
- [ ] **Step 3: Add localization keys**
  Add `"dock.consoleWindows"` and `"dock.consoleWindows.desc"` to `en.json` and `tr.json`.
- [ ] **Step 4: Verify frontend type check**
  Run `bun run build` or `npx tsc --noEmit`.

---

### Task 5: Çentikte Kırmızı Düşük Pil Flip Uyarısı

**Files:**
- Modify: `src/App.tsx`
- Modify: `src/App.css`

**Interfaces:**
- Consumes: `hasBattery`, `isCharging`, `batteryLevel`, `lowBatteryThreshold`, `t("battery.low")`.

- [ ] **Step 1: Define `batteryLow` condition in `src/App.tsx`**
  `const batteryLow = hasBattery && !isCharging && batteryLevel <= lowBatteryThreshold;`
- [ ] **Step 2: Add animated flip state in `time-flip-container`**
  Inside `<AnimatePresence initial={false}>` in `src/App.tsx`: when `batteryLow` is true, render `<motion.span key="low-battery" className="time low-battery-text">` with 3D flip animation and `{t("battery.low")}`.
- [ ] **Step 3: Add `.time.low-battery-text` CSS in `src/App.css`**
  Add color `#FF453A`, `font-weight: 700`, `letter-spacing: 0.06em`, `text-transform: uppercase`, `white-space: nowrap`.
- [ ] **Step 4: Verify styles and transitions**
  Run `npx tsc --noEmit`.

---

### Task 6: Haftanın İlk Günü Seçimi (`weekStart.ts` & Takvimler)

**Files:**
- Create: `src/weekStart.ts`
- Modify: `src/i18n.ts`
- Modify: `src/components/NotchCalendar.tsx`
- Modify: `src/Calendar.tsx`
- Modify: `src/settings/useSettings.ts`
- Modify: `src/settings/GeneralTab.tsx`
- Modify: `src/locales/en.json`
- Modify: `src/locales/tr.json`

**Interfaces:**
- Produces: `resolveWeekStart`, `daysSinceWeekStart`, `useWeekStart`, setting key `fern-week-start`.

- [ ] **Step 1: Create `src/weekStart.ts`**
  Implement `WEEK_START_KEY = "fern-week-start"`, `resolveWeekStart`, `daysSinceWeekStart`, and `useWeekStart` hook with `useSettingsSync`.
- [ ] **Step 2: Update `weekdayNames` in `src/i18n.ts`**
  Allow optional `startDay: number` parameter in `weekdayNames(width, startDay)`.
- [ ] **Step 3: Connect `NotchCalendar.tsx` and `Calendar.tsx`**
  Use `daysSinceWeekStart(anchor, weekStart)` to calculate grid offsets and pass `weekStart` to `weekdayNames`.
- [ ] **Step 4: Add week start dropdown in `src/settings/GeneralTab.tsx`**
  Add dropdown with `auto`, `sunday`, `monday` options and localize.
- [ ] **Step 5: Add localization keys**
  Add `general.weekStart`, `general.weekStart.desc`, `general.weekStart.auto`, `general.weekStart.sunday`, `general.weekStart.monday` to `en.json` and `tr.json`.
- [ ] **Step 6: Verify build**
  Run `bun run build`.

---

### Task 7: Tam Sistem Doğrulaması ve Entegrasyon Testi

**Files:**
- Entire repository

- [ ] **Step 1: Compile Rust backend**
  Execute `cargo check` and verify zero errors/warnings.
- [ ] **Step 2: Compile TypeScript frontend**
  Execute `npm run build` / `bun run build` and ensure complete build output without bundle errors.
- [ ] **Step 3: Git status & sanity check**
  Review `git diff` to ensure surgical changes without any stray formatting or regression.
