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
How it works: a click-through always-on-top window per monitor shows the old picture, animates to the new one (GPU:
transform/opacity/clip-path), the real wallpaper is set underneath, then the windows are destroyed (nothing stays resident).
Order of events: overlay shows the old picture → Windows is told to change the wallpaper underneath → animation plays →
the overlay stays until Windows has finished applying the new image (`transition_hold_ms` in `config.json`, default 1800 ms
counted from the change; raise it if the old wallpaper ever flashes back at the end, e.g. with very large images).
The overlay covers each monitor's work area, so the taskbar is never covered.
Limitation: the overlay sits above your windows, so they are hidden for the length of the animation. Pictures that cannot
be read, or a monitor setup where Windows reports no current wallpaper, fall back to the plain Windows fade.

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
src-tauri/src/transition.rs  per-monitor animation windows + flow
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
- Animations: all 13 verified frame-by-frame in a browser harness, and the full flow (hotkey → overlay windows → animation → cleanup) ran in the real app on Linux. The Windows-specific parts (overlay stacking above the taskbar, DPI on mixed-scale monitors) are untested.
- Idle RAM of the hidden WebView2 was not measured.
