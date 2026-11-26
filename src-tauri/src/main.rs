//! WallFlip — Alt+W opens an animated gallery of your wallpapers.
//!
//! Rust owns everything heavy (scanning, thumbnails, wallpaper, hotkeys, tray);
//! the WebView only draws the gallery. While idle nothing runs: the OS delivers
//! hotkey/tray events, the optional auto-change thread sleeps on a channel, and
//! the hidden WebView is not painting.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod config;
mod desktop;
mod focus;
mod playlist;
mod scan;
mod thumbs;
mod transition;
mod wallpaper;

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::http::Response;
use tauri::menu::MenuBuilder;
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{
    AppHandle, Emitter, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder, WindowEvent,
};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt as AutostartExt};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use config::{Config, StyleSetting, Transition};
use playlist::Playlist;

/// How many unreadable images to skip before giving up on one key press.
const MAX_SKIPS: usize = 6;
/// Ignore focus-loss for this long after showing (Windows needs a moment to hand over focus).
const BLUR_GRACE: Duration = Duration::from_millis(350);
/// A shortcut press this soon after click-away closed the gallery means "close".
const TOGGLE_GRACE: Duration = Duration::from_millis(250);

// ───────────────────────────── state ─────────────────────────────

struct AppState {
    cfg: Mutex<Config>,
    cfg_path: PathBuf,
    cache_dir: PathBuf,
    playlist: Mutex<Playlist>,
    /// thumbnail id -> file path for the current folder listing
    index: Mutex<HashMap<String, PathBuf>>,
    /// Our own record of whether the gallery is open (asking the OS from other threads is unreliable).
    overlay_open: AtomicBool,
    shown_at: Mutex<Instant>,
    hidden_at: Mutex<Instant>,
    interval_tx: Mutex<mpsc::Sender<u32>>,
    /// serialises wallpaper changes so animations never overlap
    apply_lock: Mutex<()>,
    /// where the transition pages report progress while an animation runs
    trans_tx: Mutex<Option<mpsc::Sender<transition::Msg>>>,
    /// set when the real wallpaper change failed *after* an animation played
    animated_failure: AtomicBool,
    /// the kept-alive animation windows
    anim: Mutex<Option<transition::Pool>>,
    /// small private pool: thumbnails never starve the rest of the system
    pool: rayon::ThreadPool,
}

fn seed() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let t = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64);
    t ^ ((std::process::id() as u64) << 32)
}

fn lower(p: &Path) -> String {
    p.to_string_lossy().to_lowercase()
}

// ───────────────────────────── folder & playlist ─────────────────────────────

enum Rescan {
    /// New folder/settings: forget position and history.
    Reset,
    /// Same folder: keep position, history and the unseen part of the no-repeat bag.
    Refresh,
}

fn rescan(app: &AppHandle, how: Rescan) {
    let st = app.state::<AppState>();
    let (folder, recursive, exts) = {
        let c = st.cfg.lock().unwrap();
        (c.folder.clone(), c.recursive, c.extensions.clone())
    };
    let files = match &folder {
        Some(f) => scan::scan(Path::new(f), recursive, &exts),
        None => Vec::new(),
    };
    let index: HashMap<String, PathBuf> = {
        let mut pl = st.playlist.lock().unwrap();
        match how {
            Rescan::Reset => pl.reset(files),
            Rescan::Refresh => pl.refresh(files),
        }
        pl.files().iter().map(|p| (thumbs::id_for(p), p.clone())).collect()
    };
    *st.index.lock().unwrap() = index;
}

/// Tells the playlist which wallpaper Windows is showing right now (it may have
/// been changed elsewhere), so Sequential continues from it.
fn sync_current(st: &AppState) -> Option<PathBuf> {
    let cur = wallpaper::current()?;
    let mut pl = st.playlist.lock().unwrap();
    if pl.current().map(lower) != Some(lower(&cur)) {
        pl.resume_after(&cur);
    }
    Some(cur)
}

/// Pre-builds thumbnails around the current wallpaper on one pool thread, so the
/// first gallery open is instant.
fn prewarm(app: &AppHandle) {
    let st = app.state::<AppState>();
    let files: Vec<PathBuf> = {
        let pl = st.playlist.lock().unwrap();
        let all = pl.files();
        let center = pl
            .current()
            .and_then(|c| all.iter().position(|f| lower(f) == lower(c)))
            .unwrap_or(0);
        let from = center.saturating_sub(12);
        all.iter().skip(from).take(25).cloned().collect()
    };
    let cache = st.cache_dir.clone();
    st.pool.spawn(move || {
        for f in files {
            let _ = thumbs::thumbnail(&f, &cache);
        }
    });
}

#[derive(Serialize)]
struct Item {
    id: String,
    name: String,
}

#[derive(Serialize)]
struct Gallery {
    items: Vec<Item>,
    current: Option<String>,
    folder: Option<String>,
    style: StyleSetting,
}

fn build_gallery(app: &AppHandle) -> Gallery {
    rescan(app, Rescan::Refresh); // cheap; picks up files added since last time
    let st = app.state::<AppState>();
    let current = sync_current(&st).map(|c| thumbs::id_for(&c));
    let (folder, style) = {
        let c = st.cfg.lock().unwrap();
        (c.folder.clone(), c.style)
    };
    let items: Vec<Item> = st
        .playlist
        .lock()
        .unwrap()
        .files()
        .iter()
        .map(|p| Item {
            id: thumbs::id_for(p),
            name: p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        })
        .collect();
    let current = current.filter(|id| items.iter().any(|i| &i.id == id));
    Gallery { items, current, folder, style }
}

/// Instantly applies the next/previous wallpaper using the configured cycle mode.
/// `over` replaces the configured animation (auto-change and the settings preview use it).
fn cycle(app: &AppHandle, forward: bool, over: Option<(Transition, u32)>) -> Result<(), String> {
    let st = app.state::<AppState>();
    let (fit, has_folder, (kind, ms)) = {
        let c = st.cfg.lock().unwrap();
        (c.fit, c.folder.is_some(), over.unwrap_or((c.transition, c.transition_ms)))
    };
    if !has_folder {
        open_settings(app);
        return Err("Choose a wallpaper folder first.".into());
    }
    if st.playlist.lock().unwrap().is_empty() {
        rescan(app, Rescan::Refresh);
    }
    let n = st.playlist.lock().unwrap().len();
    if n == 0 {
        return Err("No images found in the wallpaper folder.".into());
    }
    sync_current(&st);

    let mut failure = None;
    for _ in 0..MAX_SKIPS.min(n) {
        let pick = {
            let mut pl = st.playlist.lock().unwrap();
            if forward { pl.next() } else { pl.prev() }
        };
        let Some(path) = pick else { return Ok(()) }; // start of history
        match transition::apply(app, &path, fit, kind, ms) {
            Ok(()) => {
                failure = None;
                break;
            }
            Err(e) => {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                failure = Some(format!("Could not set \"{name}\": {e}"));
                // an animation already played for this file: don't replay six more for the next ones
                if st.animated_failure.swap(false, Ordering::SeqCst) {
                    break;
                }
            }
        }
    }
    if let Some(e) = failure {
        return Err(e);
    }
    // end of a cycle: pick up files added to / removed from the folder
    if forward && st.playlist.lock().unwrap().take_wrapped() {
        rescan(app, Rescan::Refresh);
    }
    Ok(())
}

/// Runs a cycle off the UI thread and surfaces failures in the tray tooltip.
fn cycle_async(app: &AppHandle, forward: bool, over: Option<(Transition, u32)>) {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let tip = match cycle(&app, forward, over) {
            Ok(()) => tooltip(&app, None),
            Err(e) => {
                #[cfg(debug_assertions)]
                eprintln!("wallflip: {e}");
                tooltip(&app, Some(&e))
            }
        };
        if let Some(tray) = app.tray_by_id("tray") {
            let _ = tray.set_tooltip(Some(tip));
        }
    });
}

fn tooltip(app: &AppHandle, error: Option<&str>) -> String {
    let c = app.state::<AppState>().cfg.lock().unwrap().clone();
    match error {
        Some(e) => format!("WallFlip — {e}"),
        None => format!("WallFlip — {} gallery · {} next", c.gallery_hotkey, c.cycle_hotkey),
    }
}

// ───────────────────────────── windows ─────────────────────────────

fn show_overlay(app: &AppHandle) {
    let Some(win) = app.get_webview_window("main") else { return };
    // cover the monitor the mouse is on
    if let Ok(pos) = app.cursor_position() {
        if let Ok(Some(m)) = app.monitor_from_point(pos.x, pos.y) {
            let _ = win.set_position(*m.position());
            let _ = win.set_size(*m.size());
        }
    }
    let st = app.state::<AppState>();
    *st.shown_at.lock().unwrap() = Instant::now();
    st.overlay_open.store(true, Ordering::SeqCst);
    // a finished animation may still be attached to the desktop (it lingers while Windows applies the
    // picture); while it is, our input state is tied to Explorer's and keystrokes would go to the desktop
    transition::release_now(app);
    let _ = win.show();
    focus_window(app, win.clone());
    let _ = app.emit_to("main", "overlay-shown", ());
    transition::warm(app);
    // Windows sometimes ignores the first focus request
    std::thread::spawn(move || {
        for ms in [80u64, 200, 400] {
            std::thread::sleep(Duration::from_millis(ms));
            if !st_open(&win) || win.is_focused().unwrap_or(true) {
                return;
            }
            focus_window(&win.app_handle().clone(), win.clone());
        }
    });
}

/// Gives the keyboard focus back to the gallery if it is open (something else may have taken it).
pub(crate) fn refocus_overlay(app: &AppHandle) {
    if app.state::<AppState>().overlay_open.load(Ordering::SeqCst) {
        if let Some(win) = app.get_webview_window("main") {
            if !win.is_focused().unwrap_or(false) {
                focus_window(app, win);
            }
        }
    }
}

/// Takes the keyboard focus for `win`, forcing it past Windows' foreground rules (on the UI thread,
/// where the window lives). Plain `set_focus` is not always enough when only the desktop is showing.
fn focus_window(app: &AppHandle, win: tauri::WebviewWindow) {
    let _ = app.run_on_main_thread(move || {
        #[cfg(windows)]
        if let Ok(h) = win.hwnd() {
            // no system menu => nothing for Alt / Up to open (the window is frameless anyway)
            focus::strip_system_menu(h.0 as *mut std::ffi::c_void);
            focus::force_foreground(h.0 as *mut std::ffi::c_void);
        }
        let _ = win.set_focus();
    });
}

/// Whether the gallery is (still) supposed to be open.
fn st_open(win: &tauri::WebviewWindow) -> bool {
    win.app_handle().state::<AppState>().overlay_open.load(Ordering::SeqCst)
}

fn hide_overlay_now(app: &AppHandle) {
    let st = app.state::<AppState>();
    if st.overlay_open.swap(false, Ordering::SeqCst) {
        *st.hidden_at.lock().unwrap() = Instant::now();
        if let Some(win) = app.get_webview_window("main") {
            let _ = win.hide();
        }
        let _ = app.emit_to("main", "overlay-hidden", ());
    }
}

fn toggle_overlay(app: &AppHandle) {
    let st = app.state::<AppState>();
    if st.overlay_open.load(Ordering::SeqCst) {
        hide_overlay_now(app);
    } else if st.hidden_at.lock().unwrap().elapsed() < TOGGLE_GRACE {
        // The press itself took focus away and click-away just closed the gallery
        // (seen with X11 key grabs): count the press as "close", don't reopen.
    } else {
        show_overlay(app);
    }
}

/// Settings window is created on demand and destroyed on close, so it costs no memory while closed.
fn open_settings(app: &AppHandle) {
    hide_overlay_now(app);
    let app = app.clone();
    // building windows must not happen on the event-loop thread on Windows
    tauri::async_runtime::spawn(async move {
        if let Some(w) = app.get_webview_window("settings") {
            let _ = w.show();
            let _ = w.set_focus();
            return;
        }
        let _ = WebviewWindowBuilder::new(&app, "settings", WebviewUrl::App("settings.html".into()))
            .title("WallFlip Settings")
            .inner_size(560.0, 800.0)
            .min_inner_size(520.0, 620.0)
            .center()
            .build();
    });
}

// ───────────────────────────── shortcuts ─────────────────────────────

fn register_shortcuts(app: &AppHandle, cfg: &Config) -> Result<(), String> {
    let gs = app.global_shortcut();
    let gallery: Shortcut = cfg
        .gallery_hotkey
        .parse()
        .map_err(|_| format!("'{}' is not a valid shortcut.", cfg.gallery_hotkey))?;
    let quick: Shortcut = cfg
        .cycle_hotkey
        .parse()
        .map_err(|_| format!("'{}' is not a valid shortcut.", cfg.cycle_hotkey))?;
    if gallery == quick {
        return Err("The two shortcuts must be different.".into());
    }
    gs.unregister_all().map_err(|e| e.to_string())?;
    gs.on_shortcut(gallery, |app, _, ev| {
        if ev.state() == ShortcutState::Pressed {
            focus::mask_alt_tap();
            toggle_overlay(app);
        }
    })
    .map_err(|_| format!("{} is already used by another program.", cfg.gallery_hotkey))?;
    gs.on_shortcut(quick, |app, _, ev| {
        if ev.state() == ShortcutState::Pressed {
            focus::mask_alt_tap();
            cycle_async(app, true, None);
        }
    })
    .map_err(|_| format!("{} is already used by another program.", cfg.cycle_hotkey))?;
    Ok(())
}

// ───────────────────────────── auto-change timer ─────────────────────────────

/// Sleeps on a channel (0 CPU). A message with a new interval restarts the countdown.
fn start_timer(app: AppHandle, rx: mpsc::Receiver<u32>, initial_minutes: u32) {
    let _ = std::thread::Builder::new().name("auto-change".into()).spawn(move || {
        let mut minutes = initial_minutes;
        loop {
            let msg = if minutes == 0 {
                rx.recv().map_err(|_| mpsc::RecvTimeoutError::Disconnected)
            } else {
                rx.recv_timeout(Duration::from_secs(u64::from(minutes) * 60))
            };
            match msg {
                Ok(m) => minutes = m,
                // unattended changes never cover your windows: Windows' own fade only
                Err(mpsc::RecvTimeoutError::Timeout) => cycle_async(&app, true, Some((Transition::Native, 0))),
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
    });
}

// ───────────────────────────── commands ─────────────────────────────

#[tauri::command]
async fn get_gallery(app: AppHandle) -> Gallery {
    build_gallery(&app)
}

#[tauri::command]
async fn reload_gallery(app: AppHandle) -> Gallery {
    build_gallery(&app)
}

#[tauri::command]
async fn apply_wallpaper(app: AppHandle, id: String) -> Result<(), String> {
    let (path, fit, kind, ms) = {
        let st = app.state::<AppState>();
        let path = st.index.lock().unwrap().get(&id).cloned();
        let c = st.cfg.lock().unwrap();
        (path, c.fit, c.transition, c.transition_ms)
    };
    let path = path.ok_or_else(|| "That image is gone — press R to reload.".to_string())?;
    // close the gallery first so the animation plays on a clear screen
    hide_overlay_now(&app);
    let p = path.clone();
    let worker = app.clone();
    let res = tauri::async_runtime::spawn_blocking(move || transition::apply(&worker, &p, fit, kind, ms))
        .await
        .map_err(|e| e.to_string())?;
    match res {
        Ok(()) => {
            app.state::<AppState>().playlist.lock().unwrap().resume_after(&path);
            Ok(())
        }
        Err(e) => {
            show_overlay(&app);
            Err(format!("Could not set wallpaper: {e}"))
        }
    }
}

/// Settings "Try it": really changes to the next wallpaper, using the (unsaved) animation chosen there.
#[tauri::command]
async fn preview_transition(app: AppHandle, kind: Transition, ms: u32) -> Result<(), String> {
    let ms = ms.clamp(config::MIN_TRANSITION_MS, config::MAX_TRANSITION_MS);
    tauri::async_runtime::spawn_blocking(move || cycle(&app, true, Some((kind, ms))))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
fn transition_up(app: AppHandle) {
    transition::page_up(&app);
}

#[tauri::command]
fn transition_ready(app: AppHandle) {
    transition::page_ready(&app);
}

#[tauri::command]
fn transition_painted(app: AppHandle) {
    transition::page_painted(&app);
}

#[tauri::command]
fn transition_failed(app: AppHandle) {
    transition::page_failed(&app);
}

#[tauri::command]
fn transition_done(app: AppHandle) {
    transition::page_done(&app);
}

#[tauri::command]
fn hide_overlay(app: AppHandle) {
    hide_overlay_now(&app);
}

#[tauri::command]
fn open_settings_cmd(app: AppHandle) {
    open_settings(&app);
}

#[tauri::command]
fn get_config(app: AppHandle) -> Config {
    app.state::<AppState>().cfg.lock().unwrap().clone()
}

#[tauri::command]
async fn save_config(app: AppHandle, config: Config) -> Result<(), String> {
    let mut new = config;
    new.normalize()?;
    if let Some(f) = &new.folder {
        if !Path::new(f).is_dir() {
            return Err(format!("Folder not found: {f}"));
        }
    }
    let old = app.state::<AppState>().cfg.lock().unwrap().clone();

    if new.gallery_hotkey != old.gallery_hotkey || new.cycle_hotkey != old.cycle_hotkey {
        if let Err(e) = register_shortcuts(&app, &new) {
            let _ = register_shortcuts(&app, &old); // roll back
            return Err(e);
        }
    }
    {
        let st = app.state::<AppState>();
        new.save(&st.cfg_path).map_err(|e| format!("Could not save settings: {e}"))?;
        *st.cfg.lock().unwrap() = new.clone();
        st.playlist.lock().unwrap().set_mode(new.mode);
        if new.interval_min != old.interval_min {
            let _ = st.interval_tx.lock().unwrap().send(new.interval_min);
        }
    }
    if new.folder != old.folder || new.recursive != old.recursive || new.extensions != old.extensions {
        rescan(&app, Rescan::Reset);
        prewarm(&app);
    }
    if let Some(tray) = app.tray_by_id("tray") {
        let _ = tray.set_tooltip(Some(tooltip(&app, None)));
    }
    let _ = app.emit("config-changed", ());
    Ok(())
}

#[tauri::command]
async fn choose_folder(app: AppHandle) -> Option<String> {
    let mut dialog = app.dialog().file();
    if let Some(w) = app.get_webview_window("settings") {
        dialog = dialog.set_parent(&w);
    }
    let start = app.state::<AppState>().cfg.lock().unwrap().folder.clone();
    if let Some(s) = start {
        dialog = dialog.set_directory(s);
    }
    dialog
        .blocking_pick_folder()
        .and_then(|p| p.into_path().ok())
        .map(|p| p.to_string_lossy().into_owned())
}

#[tauri::command]
fn get_autostart(app: AppHandle) -> bool {
    app.autolaunch().is_enabled().unwrap_or(false)
}

#[tauri::command]
fn set_autostart(app: AppHandle, enabled: bool) -> Result<(), String> {
    let al = app.autolaunch();
    if enabled { al.enable() } else { al.disable() }.map_err(|e| e.to_string())
}

#[tauri::command]
fn quit_app(app: AppHandle) {
    app.exit(0);
}

// ───────────────────────────── tray ─────────────────────────────

fn build_tray(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let menu = MenuBuilder::new(app)
        .text("toggle", "Open gallery")
        .text("next", "Next wallpaper")
        .text("prev", "Previous wallpaper")
        .separator()
        .text("settings", "Settings…")
        .separator()
        .text("quit", "Quit WallFlip")
        .build()?;
    let icon = app.default_window_icon().cloned().ok_or("the app has no icon")?;
    TrayIconBuilder::with_id("tray")
        .icon(icon)
        .tooltip(tooltip(app, None))
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "toggle" => show_overlay(app),
            "next" => cycle_async(app, true, None),
            "prev" => cycle_async(app, false, None),
            "settings" => open_settings(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                let app = tray.app_handle();
                // clicking the tray steals focus, which already closed the overlay: don't reopen it
                let just_closed = app.state::<AppState>().hidden_at.lock().unwrap().elapsed() < BLUR_GRACE;
                if !just_closed {
                    toggle_overlay(app);
                }
            }
        })
        .build(app)?;
    Ok(())
}

// ───────────────────────────── main ─────────────────────────────

fn main() {
    tauri::Builder::default()
        // must be first: a second launch just opens the gallery of the running instance
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| show_overlay(app)))
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, Some(vec![])))
        .register_asynchronous_uri_scheme_protocol("thumb", |ctx, request, responder| {
            let app = ctx.app_handle().clone();
            let id = request.uri().path().trim_start_matches('/').to_string();
            let worker = app.clone();
            app.state::<AppState>().pool.spawn(move || {
                let st = worker.state::<AppState>();
                let path = st.index.lock().unwrap().get(&id).cloned();
                let bytes = path
                    .ok_or_else(|| "unknown image".to_string())
                    .and_then(|p| thumbs::thumbnail(&p, &st.cache_dir));
                let response = match bytes {
                    Ok(b) => Response::builder()
                        .status(200)
                        .header("Content-Type", "image/jpeg")
                        .header("Access-Control-Allow-Origin", "*")
                        .header("Cache-Control", "no-cache")
                        .body(Cow::Owned(b)),
                    Err(_) => Response::builder()
                        .status(404)
                        .header("Access-Control-Allow-Origin", "*")
                        .body(Cow::Owned(Vec::new())),
                };
                if let Ok(r) = response {
                    responder.respond(r);
                }
            });
        })
        // screen-sized pictures for the transition pages: wall://<hex path>?w=..&h=..
        .register_asynchronous_uri_scheme_protocol("wall", |ctx, request, responder| {
            let app = ctx.app_handle().clone();
            let uri = request.uri();
            let path = transition::unhex(uri.path().trim_start_matches('/'));
            let q = uri.query().unwrap_or("").to_string();
            let num = move |k: &str| -> u32 {
                q.split('&')
                    .find_map(|kv| kv.strip_prefix(k).and_then(|v| v.strip_prefix('=')))
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0)
            };
            let (w, h) = (num("w"), num("h"));
            let worker = app.clone();
            app.state::<AppState>().pool.spawn(move || {
                let _ = &worker;
                let bytes = path
                    .ok_or_else(|| "bad path".to_string())
                    .and_then(|p| thumbs::fullsize(&p, w, h));
                let response = match bytes {
                    Ok(b) => Response::builder()
                        .status(200)
                        .header("Content-Type", "image/jpeg")
                        .header("Access-Control-Allow-Origin", "*")
                        .header("Cache-Control", "max-age=120")
                        .body(Cow::Owned(b)),
                    Err(_) => Response::builder()
                        .status(404)
                        .header("Access-Control-Allow-Origin", "*")
                        .body(Cow::Owned(Vec::new())),
                };
                if let Ok(r) = response {
                    responder.respond(r);
                }
            });
        })
        .invoke_handler(tauri::generate_handler![
            get_gallery,
            reload_gallery,
            apply_wallpaper,
            hide_overlay,
            preview_transition,
            transition_up,
            transition_ready,
            transition_painted,
            transition_failed,
            transition_done,
            open_settings_cmd,
            get_config,
            save_config,
            choose_folder,
            get_autostart,
            set_autostart,
            quit_app,
        ])
        .on_window_event(|window, event| {
            // click-away closes the gallery
            if window.label() == "main" {
                if let WindowEvent::Focused(false) = event {
                    let app = window.app_handle();
                    let age = app.state::<AppState>().shown_at.lock().unwrap().elapsed();
                    if age > BLUR_GRACE {
                        hide_overlay_now(app);
                    }
                }
            }
        })
        .setup(|app| {
            let handle = app.handle().clone();
            let cfg_path = app.path().app_config_dir()?.join("config.json");
            let cache_dir = app.path().app_local_data_dir()?.join("thumbs");
            let (cfg, warning) = Config::load(&cfg_path);

            let threads = std::thread::available_parallelism().map_or(2, |n| n.get() / 2).clamp(2, 4);
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .thread_name(|i| format!("thumb-{i}"))
                .build()?;
            let (tx, rx) = mpsc::channel();

            app.manage(AppState {
                cfg: Mutex::new(cfg.clone()),
                cfg_path,
                cache_dir: cache_dir.clone(),
                playlist: Mutex::new(Playlist::new(cfg.mode, seed())),
                index: Mutex::new(HashMap::new()),
                overlay_open: AtomicBool::new(false),
                shown_at: Mutex::new(Instant::now()),
                hidden_at: Mutex::new(Instant::now() - Duration::from_secs(60)),
                interval_tx: Mutex::new(tx),
                apply_lock: Mutex::new(()),
                trans_tx: Mutex::new(None),
                animated_failure: AtomicBool::new(false),
                anim: Mutex::new(None),
                pool,
            });

            start_timer(handle.clone(), rx, cfg.interval_min);
            transition::start_reaper(handle.clone());
            build_tray(&handle)?;
            let shortcut_error = register_shortcuts(&handle, &cfg).err();

            rescan(&handle, Rescan::Reset);
            if let Some(cur) = wallpaper::current() {
                handle.state::<AppState>().playlist.lock().unwrap().resume_after(&cur);
            }
            prewarm(&handle);
            handle.state::<AppState>().pool.spawn(move || thumbs::prune(&cache_dir));

            // first run, broken config or a shortcut clash: open settings so the user can fix it
            if cfg.folder.is_none() || warning.is_some() || shortcut_error.is_some() {
                if let Some(e) = warning.or(shortcut_error) {
                    if let Some(tray) = handle.tray_by_id("tray") {
                        let _ = tray.set_tooltip(Some(format!("WallFlip — {e}")));
                    }
                }
                open_settings(&handle);
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to build WallFlip")
        .run(|_app, event| {
            // closing the settings window must not quit the app (it lives in the tray)
            if let RunEvent::ExitRequested { api, code, .. } = event {
                if code.is_none() {
                    api.prevent_exit();
                }
            }
        });
}
