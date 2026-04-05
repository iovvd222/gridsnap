# GridSnap

**A cross-platform window management utility powered by grid resolution**

OS-native snap features (half/quarter splits) are too coarse. PowerToys FancyZones requires pre-defining zones, leading to combinatorial explosion when you need different layouts.

GridSnap defines just **one parameter — grid resolution** — and lets each window freely occupy any number of grid cells at runtime. It compresses the zone-definition combinatorial explosion into a single grid resolution setting.

![GridSnap demo](assets/gridsnap.gif)

---

## Features

- **Auto-place new windows** — Detects window creation and automatically places it on the appropriate grid cell. Manage default positions declaratively with per-app rules (`app_rules`)
- **Per-App Capture** — Press a hotkey to remember the current position of the active app. The setting is persisted immediately and applied to future windows of that app
- **Resize snap** — After dragging a window edge, the window snaps to the nearest grid line on drop. Hold `Shift` while dragging to disable snap (fine-tuning mode)
- **Move snap** — After dragging a title bar, the window snaps to the nearest grid intersection on drop. `Shift+drag` disables snap as well
- **Idle CPU 0% · Memory ≤ 5 MB** — Fully event-driven. No polling

---

## Supported Platforms

- **Windows 10 / 11**
- **macOS** (Sequoia and later)

---

## Installation

### Windows

1. Download the latest `gridsnap.exe` from [Releases](https://github.com/iovvd222/gridsnap/releases/download/v1.0.0/gridsnap.exe)
2. Run the `.exe` and follow the installer prompts
3. GridSnap launches automatically after installation and stays in the system tray

GridSnap registers itself to start automatically on Windows logon.

### macOS

1. Download the latest `gridsnap.dmg` from [Releases](https://github.com/iovvd222/gridsnap/releases/download/v1.0.0/gridsnap.dmg)
2. Open the `.dmg` and drag GridSnap to the Applications folder
3. On first launch, grant accessibility permission in **System Settings → Privacy & Security → Accessibility**

### Build from Source

```bash
git clone https://github.com/<your-username>/gridsnap.git
cd gridsnap
cargo build --release
```

Build requirement: [Rust](https://www.rust-lang.org/tools/install) (edition 2021)

---

## Usage

### Basic Operation

1. GridSnap runs in the system tray after installation
2. Drag a window → it snaps to the grid on drop
3. Hold `Shift` while dragging → snap is disabled for free positioning

### Grid Settings

Right-click the system tray icon to change the number of columns and rows. Changes take effect immediately.

### Per-App Capture

1. Place a window at your preferred position
2. Select "Capture position for this app" from the tray menu or press the hotkey
3. From now on, new windows of that app will be automatically placed at the captured position

### Batch Relocate

Press the hotkey to relocate all visible windows according to their captured rules at once.

---

## Architecture

```
┌──────────────────────────────────┐
│          GridSnap                │
│  ┌───────────┐ ┌──────────────┐  │
│  │ Key Hook  │ │ Config       │  │
│  │(Shift det)│ │(Grid defs)   │  │
│  └─────┬─────┘ └──────┬───────┘  │
│        │               │         │
│  ┌─────▼───────────────▼──────┐  │
│  │   Snap Handler             │  │
│  │  (Grid snap calculation)   │  │
│  └─────────────┬──────────────┘  │
│                │                 │
│  ┌─────────────▼──────────────┐  │
│  │   Overlay Renderer         │  │
│  │  (Grid lines / title bar)  │  │
│  └────────────────────────────┘  │
└──────────────────────────────────┘
```

- **Windows**: `SetWinEventHook` + `EVENT_SYSTEM_MOVESIZEEND` for post-drop correction. No DLL injection required
- **macOS**: `AXObserver` + `kAXWindowMovedNotification` / `kAXWindowResizedNotification` for equivalent behavior

---

## Performance

| Metric | GridSnap | PowerToys (all) | FancyZones |
| --- | --- | --- | --- |
| Memory | 1–5 MB | 200–400 MB | 30–50 MB |
| CPU (idle) | 0% | 1–3% | ~0% |

---

## Known Limitations

- **Elevated apps** — Window operations return `ACCESS_DENIED` if the target runs as administrator. Running GridSnap as administrator resolves this
- **Some UWP apps** — Certain Windows built-in apps (e.g. Calculator) have non-standard window structures and may not snap correctly
- **Exclusive fullscreen** — Games in Exclusive Fullscreen mode are out of scope
- **In-app UI** — Controlling internal layouts such as tab bars or ribbons is out of scope

---

## Tech Stack

- **Language:** Rust
- **Windows:** Direct Win32 API calls via `windows-rs`
- **macOS:** Accessibility API / CoreGraphics / Cocoa via `objc` + `cocoa` crates
- **Config:** TOML (internal format, managed through tray UI)

---

## License

MIT
