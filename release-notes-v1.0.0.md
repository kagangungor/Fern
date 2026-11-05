## What's New in Fern v1.0.0 🌿

Welcome to the official initial release of **Fern** — a physics-animated notch and reactive dock for Windows, built to seamlessly replace your traditional taskbar.

### 🎧 Connected Devices & Hardware Detection
- **Live Battery & Status Indicators:** Real-time battery levels for Bluetooth earbuds (including AirPods, Samsung Galaxy Buds case & individual earbuds), wireless headsets, and USB peripherals.
- **Custom Dynamic Vector Art:** Handcrafted SVG illustrations for connected devices that accurately reflect their visual profile.
- **Audio Jack & USB Awareness:** Instant detection when headphones or external peripherals are plugged in.

### 🦊 Fernie Desktop Companion & Agent Status
- **Interactive Companion:** Fernie the fox mascot comes with multiple expressive states: idle, working, listening to music, sleeping, and input alert.
- **AI Agent Integration:** Built-in HTTP and CLI Agent API allowing developer tools (like Claude Code and background tasks) to report their operational status.
- **Audio Cues:** Subtle, high-quality audio notifications (`agent-done`, `agent-error`, `agent-needs_input`) when background tasks finish or require attention.

### ⚡ Phase 1 Performance Core & On-Demand Windows
- **On-Demand Window Lifecycle:** Secondary windows (Settings, Calendar) are instantiated only when requested and cleanly released, eliminating background WebView bloat.
- **Memory Saver Engine:** Idle WebViews are automatically suspended to keep idle RAM and CPU consumption minimal.
- **Window Region Clipping:** Crisp click-through hit testing and precise boundary clipping preventing phantom window overlays.

### 🎨 Refined Surfaces, Dock & Theming
- **Dynamic Surface Architecture:** Smooth physics-based spring animations, adaptive backdrop blur, and custom accent tinting.
- **Dock Superpowers:** Win+1 through Win+9 keyboard shortcuts to switch or launch pinned dock apps, window attention badges, and hover previews.
- **Native System Tray Integration:** Directly access the real Windows system tray flyout under both the dock and the notch.
- **Full Localization:** Complete English and Turkish language support out of the box.
