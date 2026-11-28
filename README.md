<div align="center">

<img src="docs/logo.png" width="128" alt="Fern logo" />

<br />

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/wordmark-dark.png" />
  <img src="docs/wordmark-light.png" height="48" alt="Fern" />
</picture>

<p><strong>A notch and a dock for Windows — physics-animated, reactive, and built to replace your taskbar.</strong><br /><sub>Productivity, media, hardware control, and AI agent awareness in one native layer for Windows 11.</sub></p>

[![Latest release](https://shieldcn.dev/github/release/kagangungor/Fern/stable.svg?logo=lu:Tag)](https://github.com/kagangungor/Fern/releases/latest)
[![Downloads](https://shieldcn.dev/github/downloads/kagangungor/Fern.svg?logo=lu:Download)](https://github.com/kagangungor/Fern/releases)
[![License](https://shieldcn.dev/github/license/kagangungor/Fern.svg?logo=lu:Scale)](LICENSE)
[![Platform](https://shieldcn.dev/badge/platform-Windows.svg?logo=ri:FaWindows)](https://www.microsoft.com/windows)

[**Download**](https://github.com/kagangungor/Fern/releases/latest) &nbsp;·&nbsp; [Features](#features) &nbsp;·&nbsp; [Install](#install) &nbsp;·&nbsp; [Usage](#usage) &nbsp;·&nbsp; [Settings](SETTINGS.md) &nbsp;·&nbsp; [Sponsor](https://buymeacoffee.com/kagangungor) &nbsp;·&nbsp; [Report a bug](https://github.com/kagangungor/Fern/issues/new/choose)

<br />

<img src="docs/screenshots/desktop.webp" width="860" alt="Fern's notch and dock on a Windows desktop" />

</div>

<br />

## Features

| Area | What Fern gives you |
| --- | --- |
| **Notch & Media** | Spring-animated island with **Live Synchronized Lyrics** (click any line to seek), album-art theming, multi-session switching, a drag-to-arrange **Control Center**, Calendar (`Day / Week / Month / Agenda` + read-only `.ics` & CalDAV sync), Timer, Stopwatch, Dual Clock, and a persistent File Shelf. |
| **Quick Launcher** | Built-in, PowerToys-free Spotlight search (`Alt+Space`) for apps, running windows, files (with optional Everything IPC integration), live math calculations, web/URL search, and bilingual Fern & system quick commands. |
| **Notch AI Chat & Agents** | Chat with **Claude**, **ChatGPT**, or **Gemini** (via encrypted API keys or local Claude/Codex/Gemini CLIs) right from the notch with screenshot/image attachments, stoppable streaming, and automatic fallback. **Fernie** also tracks background **Claude Code** & AI agent workflows with audio cues and inline **Allow / Deny** prompts. |
| **Hardware & Audio** | Live **split battery levels** (left bud, right bud, case) for AirPods, Beats, Google Fast Pair earbuds, and Samsung Galaxy Buds, plus USB & 3.5mm jack alerts with custom vector art. Scroll any running **Dock icon** to adjust that app's volume in real time with a floating mini-HUD. |
| **Privacy & Meetings** | **Privacy Aura** glows around the notch whenever your camera or microphone is active, paired with a system-wide **Mic Mute hotkey** (`Ctrl+F1` by default) that works across Zoom, Teams, Meet, and Discord. |
| **Taskbar Replacement Dock** | Mirrors your pinned apps and order with real icons, live **DWM window previews**, attention badges, drag-and-drop hover focus, `Win+1..9` switching, native Windows tray flyout, `End Task`, Show Desktop corner, drives/Recycle Bin, and built-in **Quick Launcher** (or optional Windows Search, PowerToys Run, Raycast, Flow Launcher, Everything, Listary). |
| **Glass, Themes & Performance** | **Solid**, **Frosted Glass**, **Liquid Glass** (with blurred wallpaper & cursor Edge Shine), and **Material You** styles, plus optional **Windows Accent Sync**. **Fullscreen Direct-Flip Protection** (zero FPS/input-lag overhead in games), on-demand Settings/Calendar windows, **Memory Saver**, multi-monitor rules, English & Turkish UI, and live-reloaded `settings.json`. |

<div align="center">
<table>
  <tr>
    <td align="center"><img src="docs/screenshots/island.webp" width="420" alt="The Fern notch with privacy indicator" /></td>
    <td align="center"><img src="docs/screenshots/dock.webp" width="420" alt="The Fern dock" /></td>
  </tr>
  <tr>
    <td align="center"><sub>The notch (with active mic indicator)</sub></td>
    <td align="center"><sub>The dock</sub></td>
  </tr>
</table>
</div>

## Install

Download from the [latest release](https://github.com/kagangungor/Fern/releases/latest):

- **Installer (`Fern-Setup-<version>.exe`)** — Choose **Just me** or **All users** (asks for administrator permission once). Launches with a short splash, then starts the notch and dock together.
- **Portable (`fern-<version>-portable-windows-x64.zip`)** — Extract and run `fern.exe` directly without installing.

Fern updates itself: a green dot on the notch signals a ready update, and **Auto Update** in **Settings › About** installs it for you. Enable **Get pre-releases** there for early builds. Uninstalling from **Windows Settings › Apps** or **Settings › About** automatically restores your Windows taskbar.

<details>
<summary>Download safety</summary>

<br />

Only download Fern binaries from the official [releases page](https://github.com/kagangungor/Fern/releases) of this repository. You can inspect the local file hash at any time with:

```powershell
Get-FileHash .\Fern-Setup-<version>.exe -Algorithm SHA256
```

</details>

## Usage

### Notch panels

Scroll or swipe on the notch to cycle through panels (customizable order in **Settings › Notch**):

| Panel | What it does |
| --- | --- |
| **Music** | Album art, dynamic cover tinting, playback controls, visualizer, multi-app audio switcher, and **Live Synchronized Lyrics** with click-to-seek. |
| **Control Center** | Wi-Fi, Bluetooth, Do Not Disturb, Battery Saver, volume (with output switcher & per-app mixer), and brightness. Drag tiles, buttons, and sliders into your own layout. |
| **Status** | Configurable left/right widgets: weather, battery (auto-hidden on desktops), CPU, RAM, disk, and network speed/status. |
| **Connected Devices** | Live battery readouts and dynamic SVG cards for Bluetooth earbuds (individual left/right/case levels), headsets, and USB peripherals. |
| **Calendar** | Day, week, month, and agenda views synced from `.ics` / `webcal://` links or CalDAV (credentials stored in Windows Credential Manager), plus timer, stopwatch, and Pomodoro. |
| **Shelf** | Drop files or text onto the notch to keep them handy and drag them into any app later. |
| **AI Chat** | Ask Claude, ChatGPT, or Gemini directly from the notch (using DPAPI/keyring-encrypted API keys or local Claude, Codex, and Gemini CLIs), attach screenshots or images, and stream responses with isolated per-provider history. |
| **Agent Companion** | Fernie mascot status card with tool details, click-to-focus window jumping, and **Allow / Deny** buttons for interactive prompts. |

<div align="center">
  <img src="docs/screenshots/notch-lyrics.webp" width="580" alt="Live Synchronized Lyrics inside the Fern music notch" />
  <br />
  <sub>Live Synchronized Lyrics — click any line to seek</sub>
</div>

### Visibility & overlays

The notch and the dock each configure their own visibility mode — and when **Show On** is set to **All Monitors**, you can assign **Per-Monitor Behavior** independently:

| Mode | Behavior |
| --- | --- |
| **Fixed** | Always visible. The dock reserves screen space like a standard taskbar (and supports **Adaptive Mode** to stretch full-width when a window is maximized). |
| **Smart** | Automatically hides when overlapped by another window or fullscreen app and reveals on hover. |
| **Peek** | Stays hidden until the cursor reaches the screen edge or a media/notification event arrives. |

In **Smart** and **Peek**, **Fullscreen Direct-Flip Protection** (enabled by default in **Settings › General**) natively hides notch, dock, and HUD overlay windows over fullscreen games and apps to preserve DWM Direct-Flip (zero FPS/input-lag overhead and intact VRR/G-Sync) while slowing background polling. Fern also includes edge-triggered **Volume** and **Brightness** HUD overlays (with DDC/CI support for external monitors).

### Shortcuts & quick actions

| Action | Trigger |
| --- | --- |
| **Quick Launcher** | `Alt+Space` (customizable under **Settings › Quick Launcher**) |
| **Per-app volume** | Scroll mouse wheel over any running Dock icon (`Middle-click` to mute) |
| **Toggle microphone** | `Ctrl+F1` (customizable under **Settings › Notch › Meeting & Privacy**) |
| **Toggle notch** | Customizable global shortcut under **Settings › Notch › Keyboard Shortcuts** |
| **Open Notch AI Chat** | Customizable global shortcut under **Settings › Notch** |
| **Switch or launch pinned apps** | `Win+1` .. `Win+9` |
| **Drag file to open window** | Drag any file over a Dock icon and hold briefly (`450 ms`) to bring it to front |

<details>
<summary><b>For developers: AI Agent API & Claude Code integration</b></summary>

<br />

When **Claude Code & AI agents** is enabled in **Settings › Notch**, Fern automatically configures Claude Code hooks and starts an authenticated loopback HTTP server described in `%APPDATA%\com.kagangungor.fern\agent.json` (`port` and `token`).

- `POST /activity` — Push states (`working`, `tool`, `needs_input`, `done`, `error`, `clear`) with optional `name`, `project`, `detail`, and `window` HWND.
- `POST /ask` — Display an interactive **Allow / Deny** prompt inside the notch and wait for the user's decision.

</details>

## Troubleshooting

<details>
<summary><b>The dock shows over my game</b></summary>

<br />

Fern natively hides the dock, notch, and HUD overlays over fullscreen windows in **Smart** and **Peek** modes via **Fullscreen Direct-Flip Protection** (**Settings › General**, enabled by default). **Fixed** mode stays visible by design. If something still appears over a game, open an issue with the game name and display mode (fullscreen, borderless, or windowed).

</details>

<details>
<summary><b>The Windows taskbar did not come back</b></summary>

<br />

Start Fern again — it restores the taskbar on launch even after a crash. Uninstalling from **Windows Settings › Apps** or **Settings › About** also brings it back.

</details>

<details>
<summary><b>Memory usage on multi-monitor setups</b></summary>

<br />

Fern loads secondary windows (Settings, Calendar) on demand, and you can enable **Memory Saver** under **Settings › General** to trim working sets and slow background checks while idle or locked. When **Show On** is set to **All Monitors**, each extra display runs its own notch and dock WebViews — switching to **Primary Monitor** reduces memory usage further.

</details>

<details>
<summary><b>Brightness does not change on my external monitor</b></summary>

<br />

External monitors are controlled over DDC/CI. Enable DDC/CI in your monitor's on-screen menu and connect it directly if you use a USB-C hub or dock, as some adapters block DDC/CI commands.

</details>

<details>
<summary><b>Something else is broken</b></summary>

<br />

Open **Settings › Diagnostics › Copy Diagnostics** and paste the output into a [bug report](https://github.com/kagangungor/Fern/issues/new/choose). It includes your app version, monitor layout, and recent logs.

</details>

## Build from source

You need [Rust](https://rustup.rs/) and [Bun](https://bun.sh/) on Windows.

```bash
git clone https://github.com/kagangungor/Fern.git
cd Fern
bun install
bun run tauri dev
```

`bun run tauri build` creates a release build. Before opening a pull request, read [CONTRIBUTING.md](CONTRIBUTING.md).

## Roadmap

### Shipped in v2.0
- [x] **Built-in Search Provider (PowerToys-Free)** — Open-source, standalone Dock/Notch quick launcher and file/app search built in Fern's own style, without relying on PowerToys Run or Command Palette
- [x] **Notch AI Chat** — Ask Claude (API or local Claude Code CLI), ChatGPT, or Gemini directly from the notch with image/screenshot attachments, stoppable streaming, and automatic provider fallback
- [x] **DWM Direct-Flip Protection & Fullscreen Zero-Overhead** — Natively hide notch/dock/overlay windows (`ShowWindow`) over fullscreen games to preserve DWM Direct-Flip (zero FPS/input-lag penalty, intact VRR/G-Sync) and pause Fern's own background polling while a fullscreen game is active

### Upcoming milestones

#### Core Vision & Productivity (Post-2.0)
- [ ] **Pro Clipboard & Smart Inspector** — Fast island clipboard history with search, pinning, and live color token previews (`#hex`, `rgb`, `hsl`)
- [ ] **Dock Notepad & Customizable Dock Menu** — Lightweight dock notebook and customizable dock right-click menu with custom app/file shortcuts
- [ ] **Developer & Terminal Ambient HUD** — Visual notch progress line for long-running CLI tasks (`cargo build`, `npm install`, `docker`, test suites) and active Git branch / dirty-file context when an IDE or terminal is focused

#### Platform & Polish
- [ ] **Linux Support** — Native Linux builds (AppImage & `.deb`) with X11/GTK desktop integration and hardware controls (`nmcli`, `bluetoothctl`, `pactl`, `playerctl`)
- [ ] **Custom Installer & Automated RAM/CPU Benchmarking** — Dedicated modern installer app and repeatable memory/CPU benchmark scripts to track and minimize idle footprint
- [ ] **Settings & UI Polish** — Inline row-level settings search (`SettingsSearchBox`)

See the [open issues](https://github.com/kagangungor/Fern/issues) for feature requests and known bugs.

## Built with

[![Tauri](https://img.shields.io/badge/Tauri-24C8D8?style=for-the-badge&logo=tauri&logoColor=white)](https://tauri.app/)
[![Rust](https://img.shields.io/badge/Rust-000000?style=for-the-badge&logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![React](https://img.shields.io/badge/React-20232A?style=for-the-badge&logo=react&logoColor=61DAFB)](https://react.dev/)
[![TypeScript](https://img.shields.io/badge/TypeScript-3178C6?style=for-the-badge&logo=typescript&logoColor=white)](https://www.typescriptlang.org/)
[![Vite](https://img.shields.io/badge/Vite-646CFF?style=for-the-badge&logo=vite&logoColor=white)](https://vitejs.dev/)

## Support

Fern is free and open source. If it makes your desktop nicer, you can support its development:

[![Buy Me a Coffee](https://img.shields.io/badge/Buy%20Me%20a%20Coffee-FFDD00?style=for-the-badge&logo=buymeacoffee&logoColor=black)](https://buymeacoffee.com/kagangungor)

## License

Fern is open source under the [GPLv3](LICENSE).

## Credits

Fern is a fork of [Nectar](https://github.com/kua8/Nectar) by kua8, which began as a fork of [Bloom](https://github.com/SehajveerSingh2005/bloom) by Sehajveer Singh. Fern split from Nectar in 2026, has been modified extensively since, and is distributed under the GPLv3.
