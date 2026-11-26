# WallFlip Gallery

Lightweight Windows 11 wallpaper switcher with a full-screen animated gallery.
Tauri v2 (Rust backend) + React/TypeScript overlay. Original implementation.

| Hotkey (default) | Action |
|---|---|
| **Alt+W** | Open/close the gallery overlay |
| **Alt+Shift+W** | Instantly apply the next wallpaper (Sequential / Shuffle / No-repeat) |

Gallery keys: ←/→ (or H/L), mouse wheel, PageUp/PageDown, Home/End to browse · **Enter** or click to apply ·
**Tab** cycle style · **R** reload folder · **S** settings · **Esc** close.
Styles: Carousel, Depth, Fan, Wall (or Random). Tray: left-click toggles gallery; menu has Next, Settings, Quit.

## Change animations
Settings → **Change animation**: Circle reveal, Wipe, Diagonal, Diamond, Fade, Slide, Push, Zoom in, Burst, Blur,
Blinds, Tiles, Flip, Random, or Windows fade (no overlay). Duration 0.2–3 s. **Try it** applies the next wallpaper with the
animation selected (before saving). Plays for gallery picks and the quick-cycle shortcut; timed auto-change always uses the
Windows fade.

How it works
- One borderless page per monitor draws the animation (GPU: transform/opacity/clip-path).
- **Draw animation → Behind icons and windows** (default): the window lives on the desktop layer, between Windows' own
  wallpaper and the icons (Windows 11 24H2+ "raised desktop" and older layouts are both handled), so icons, windows and the
  taskbar stay untouched. **On top of everything** is a fallback that hides icons/windows while it plays; use it if the
  default shows no animation (`"transition_layer": "top"` in config.json).
- The real wallpaper is changed *underneath* the animation. The overlay then stays (invisibly) for `transition_hold_ms`
  (default 1800 ms from the change) so the old picture can never flash back; this never delays the next change.
- Teardown: the animation window is detached with raw Win32 calls (the windowing library's own `hide()` would undo the
  desktop attachment), Explorer's wallpaper layer is released if we created it, and the wallpaper is re-applied once so the
  desktop repaints. Opening the gallery prepares the windows in the background and hands the keyboard focus back to the gallery.
- Keyboard focus: while an animation lingers on the desktop layer, our input state is tied to Explorer's. Opening the
  gallery therefore takes the animation down at once, and the gallery takes the foreground with the documented
  thread-input attach (needed when only the desktop is showing and no app window has the focus).
- Alt+W and the system menu: when the gallery is already the active window as Alt is released, Windows treats the lone
  Alt as a menu-key tap and enters menu mode, in which arrows/wheel go to the window's system menu (Up shows
  Minimize/Maximize/Close). The hotkey handler therefore sends an unassigned key (VK 0xE8) while Alt is still down, and
  the gallery window's system menu is removed.
- Speed: the animation windows are prepared when the gallery opens and kept for 2 minutes after the last change, so a
  change starts at once. They are destroyed when idle (about 100 MB per monitor while alive).
- A picture that cannot be read, or Windows reporting no current wallpaper (different pictures per monitor), falls back to
  the plain Windows fade.

Wallpapers are applied through `IDesktopWallpaper` (native Windows cross-fade, Fill/Fit/Stretch/Tile/Center/Span);
`SystemParametersInfo` is used only if the COM object cannot be created.
The gallery shows cached 960 px JPEG thumbnails, never full-size images, and only animates `transform`/`opacity` (GPU-composited).

## Structure
```
package.json, vite.config.ts, index.html (overlay), settings.html
src/lib/api.ts            typed Tauri command wrappers
src/overlay/              gallery UI (Overlay, Card, Wall, layouts, css)
src/settings/             settings window (folder, hotkey recorder, mode, fit, interval, autostart)
src-tauri/Cargo.toml, tauri.conf.json, capabilities/, icons/
src-tauri/src/main.rs     app state, tray, hotkeys, overlay window, thumb:// protocol, commands
src-tauri/src/wallpaper.rs  IDesktopWallpaper + SPI fallback
src-tauri/src/playlist.rs   sequential / shuffle / no-repeat engine
src-tauri/src/thumbs.rs     thumbnail generation + disk cache
src-tauri/src/transition.rs  animation windows (kept warm), job flow
src-tauri/src/desktop.rs     attach a window to the desktop layer (Windows)
src/transition/             animation page (13 effects, Web Animations API)
src-tauri/src/scan.rs, config.rs
```

## Prerequisites (Windows 11)
1. Rust (stable, MSVC): https://rustup.rs
2. Visual Studio Build Tools with "Desktop development with C++"
3. Node.js 20+ (WebView2 runtime is already part of Windows 11)

## Build / run
```
npm install
npm run tauri dev          # development
npm run tauri build        # installer: src-tauri\target\release\bundle\nsis\*.exe
```
The portable exe is `src-tauri\target\release\wallflip.exe`.

## Data locations
- Config: `%APPDATA%\com.wallflip.app\config.json`
- Thumbnail cache: `%LOCALAPPDATA%\com.wallflip.app\thumbs` (pruned after 60 days)

## Verification status (honest notes)
- Compiled and tested on Linux against real Tauri 2: 19 Rust unit tests pass; frontend builds; end-to-end smoke test
  (hotkey toggle, Esc, quick-cycle, IPC, thumbnail protocol) passed under Xvfb.
- **Not compiled for Windows in the authoring environment.** The Windows-only code (`wallpaper.rs`) was checked against
  the `windows` 0.62 crate sources for symbol/feature names only. If `cargo` reports an error there, send it over.
- Animations: all 13 effects verified frame-by-frame in a browser harness (13 jobs in one persistent page), and the full flow (hotkey → windows → animation → idle cleanup) ran in the real app on Linux (top layer). The Windows-only parts are **untested**: `desktop.rs` (desktop-layer attach, written from published 24H2+ techniques and checked against the `windows` crate API), taskbar/DPI edges. If no animation appears, set `transition_layer` to `top` and report back.
- Idle RAM of the hidden WebView2 was not measured.
