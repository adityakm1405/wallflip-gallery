# WallFlip Gallery

A lightweight Windows 11 wallpaper switcher with a full-screen animated gallery. WallFlip uses **Tauri v2 (Rust)** for its native backend and **React + TypeScript** for the gallery and settings interfaces.

## Features

- Browse wallpapers in a full-screen gallery.
- Switch wallpapers with sequential, shuffle, or no-repeat playback.
- Choose from Carousel, Depth, Fan, and Wall gallery layouts, or use Random.
- Apply animated transitions when changing wallpapers.
- Configure wallpaper folder, hotkeys, display mode, interval, and autostart in Settings.
- Use the system tray to open the gallery, move to the next wallpaper, open Settings, or quit.

## Keyboard shortcuts

| Shortcut | Action |
| --- | --- |
| **Alt + W** | Open or close the gallery |
| **Alt + Shift + W** | Apply the next wallpaper immediately |

Inside the gallery:

| Key | Action |
| --- | --- |
| `←` / `→` or `H` / `L` | Browse wallpapers |
| Mouse wheel | Browse wallpapers |
| `PageUp` / `PageDown` | Browse by page |
| `Home` / `End` | Jump to first / last wallpaper |
| `Enter` or click | Apply selected wallpaper |
| `Tab` | Cycle gallery style |
| `R` | Reload wallpaper folder |
| `S` | Open Settings |
| `Esc` | Close gallery |

## Change animations

Open **Settings → Change animation** to select an effect:

Circle reveal, Wipe, Diagonal, Diamond, Fade, Slide, Push, Zoom in, Burst, Blur, Blinds, Tiles, Flip, Random, or Windows fade (without an overlay).

Animation duration can be set from **0.2 to 3 seconds**. **Try it** applies the next wallpaper with the selected animation before saving.

The animation flow uses a click-through, always-on-top window per monitor to show the previous wallpaper while Windows applies the new one. It then animates the overlay and destroys the windows. The overlay covers each monitor's work area, so the taskbar is not covered. Because the overlay sits above other windows, those windows are hidden for the duration of the animation.

Wallpapers are applied through the native `IDesktopWallpaper` API (with Fill, Fit, Stretch, Tile, Center, and Span modes). `SystemParametersInfo` is used only if the COM object cannot be created.

The gallery displays cached 960 px JPEG thumbnails rather than full-size images. Animation uses GPU-composited transforms and opacity.

## Project structure

```text
.
├── index.html                 # Gallery overlay entry
├── settings.html              # Settings window entry
├── transition.html            # Transition animation entry
├── src/
│   ├── lib/                   # Typed Tauri command wrappers
│   ├── overlay/               # Gallery UI, cards, layouts, and styles
│   ├── settings/              # Settings UI and hotkey recorder
│   └── transition/            # Animation page and effects
└── src-tauri/
    ├── capabilities/
    ├── icons/
    └── src/                   # Rust application and native services
```

## Requirements

This project targets **Windows 11**.

- [Node.js 20 or later](https://nodejs.org/)
- [Rust (stable, MSVC toolchain)](https://rustup.rs/)
- Visual Studio Build Tools with **Desktop development with C++**
- WebView2 Runtime (included with Windows 11)

## Development

Install JavaScript dependencies:

```bash
npm install
```

Run the Tauri development app:

```bash
npm run tauri dev
```

Build the frontend:

```bash
npm run build
```

Build the Windows installer:

```bash
npm run tauri build
```

The NSIS installer is generated under:

```text
src-tauri/target/release/bundle/nsis/
```

The portable executable is generated at:

```text
src-tauri/target/release/wallflip.exe
```

## Data locations

- Configuration: `%APPDATA%\\com.wallflip.app\\config.json`
- Thumbnail cache: `%LOCALAPPDATA%\\com.wallflip.app\\thumbs` (pruned after 60 days)

## Verification notes

The supplied project README reports that:

- 19 Rust unit tests passed in the author's Linux environment.
- The frontend build and an end-to-end smoke test passed under Xvfb.
- All 13 animation effects were verified in a browser harness.
- The Windows-specific code was not compiled on Windows in the authoring environment. Overlay stacking above the taskbar and DPI behavior on mixed-scale monitors remain untested.
- Idle RAM usage of the hidden WebView2 was not measured.

These notes describe the verification reported with the supplied source; they are not an independent test run for this repository.
