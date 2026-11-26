//! Animated wallpaper changes.
//!
//! Windows can only cross-fade, so every other animation is drawn by us. One borderless
//! window per monitor (a small web page) shows the old picture and animates to the new
//! one. By default the window lives on the *desktop layer* (see `desktop.rs`): below the
//! icons and every app, so nothing on your screen disappears while it plays.
//!
//! Speed: the windows are created once and kept for `IDLE` after the last use (and are
//! prepared as soon as the gallery opens), so a change starts immediately instead of
//! waiting for a browser view to boot. The real wallpaper is changed *under* the
//! animation; the overlay then lingers invisibly until Windows has applied it, without
//! blocking the next change.

use crate::config::{Fit, Layer, Transition};
use crate::{desktop, wallpaper, AppState};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

/// Messages from the transition pages (see the `transition_*` commands in main.rs).
pub enum Msg {
    Up,
    Ready,
    Painted,
    Failed,
    Done,
}

/// Window labels must be unique: a window destroyed a moment ago may still be registered.
static RUN: AtomicU32 = AtomicU32::new(0);

/// How long a page may take to start / to decode both pictures.
const READY_TIMEOUT: Duration = Duration::from_secs(6);
/// How long a shown window may take to paint both pictures once before we start anyway.
const PAINT_TIMEOUT: Duration = Duration::from_millis(1500);
/// Windows stay alive this long after their last use, then are destroyed to free memory.
const IDLE: Duration = Duration::from_secs(120);

#[derive(Clone, Copy, PartialEq, Debug)]
struct Rect {
    x: i32,
    y: i32,
    w: u32,
    h: u32,
}

#[derive(Clone)]
struct Win {
    label: String,
    mon: Rect,
    work: Rect,
}

/// The kept-alive animation windows.
pub struct Pool {
    wins: Vec<Win>,
    layer: Layer,
    last_used: Instant,
    /// an animation (or its invisible hold) is in progress: the reaper must not touch the windows
    busy: bool,
    /// identifies the latest job, so an older job's cleanup cannot hide a newer job's windows
    generation: u64,
    /// picture to re-apply once the desktop layer is gone (Explorer may leave its layer blank)
    refresh: Option<(PathBuf, Fit)>,
}

#[derive(Serialize)]
struct Job<'a> {
    from: String,
    to: String,
    kind: &'a str,
    ms: u32,
    fit: &'a str,
    /// picture size cap (physical px); 0 = keep native size
    w: u32,
    h: u32,
    /// the whole monitor (physical px) and the window's offset inside it
    mw: u32,
    mh: u32,
    ox: i32,
    oy: i32,
}

pub fn hex(p: &Path) -> String {
    p.to_string_lossy().bytes().map(|b| format!("{b:02x}")).collect()
}

pub fn unhex(s: &str) -> Option<PathBuf> {
    if s.is_empty() || s.len() % 2 != 0 {
        return None;
    }
    let bytes: Option<Vec<u8>> = (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect();
    String::from_utf8(bytes?).ok().map(PathBuf::from)
}

fn fit_key(f: Fit) -> &'static str {
    match f {
        Fit::Fill => "fill",
        Fit::Fit => "fit",
        Fit::Stretch => "stretch",
        Fit::Tile => "tile",
        Fit::Center => "center",
        Fit::Span => "span",
    }
}

fn resolve(t: Transition) -> Transition {
    if t == Transition::Random {
        let n = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.subsec_nanos()) as usize;
        Transition::ANIMATED[n % Transition::ANIMATED.len()]
    } else {
        t
    }
}

fn same(a: &Path, b: &Path) -> bool {
    a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
}

/// The picture on screen right now: Windows' answer, else the last one we applied.
fn current_picture(app: &AppHandle) -> Option<PathBuf> {
    if let Some(p) = wallpaper::current() {
        return Some(p);
    }
    #[cfg(debug_assertions)]
    if let Some(p) = std::env::var_os("WALLFLIP_TEST_OLD") {
        return Some(PathBuf::from(p)); // lets the Linux smoke test exercise the animation
    }
    app.state::<AppState>().playlist.lock().unwrap().current().map(Path::to_path_buf)
}

fn send(app: &AppHandle, m: Msg) {
    if let Some(tx) = app.state::<AppState>().trans_tx.lock().unwrap().as_ref() {
        let _ = tx.send(m);
    }
}
pub fn page_up(app: &AppHandle) {
    send(app, Msg::Up);
}
pub fn page_ready(app: &AppHandle) {
    send(app, Msg::Ready);
}
pub fn page_painted(app: &AppHandle) {
    send(app, Msg::Painted);
}
pub fn page_failed(app: &AppHandle) {
    send(app, Msg::Failed);
}
pub fn page_done(app: &AppHandle) {
    send(app, Msg::Done);
}

fn monitors(app: &AppHandle) -> Result<Vec<(Rect, Rect)>, String> {
    let list = app.available_monitors().map_err(|e| e.to_string())?;
    if list.is_empty() {
        return Err("no monitors".into());
    }
    Ok(list
        .iter()
        .map(|m| {
            let work = m.work_area(); // the screen minus the taskbar
            (
                Rect { x: m.position().x, y: m.position().y, w: m.size().width, h: m.size().height },
                Rect { x: work.position.x, y: work.position.y, w: work.size.width, h: work.size.height },
            )
        })
        .collect())
}

/// Blocks until `n` messages matching `want` arrived or the deadline passed.
fn wait_for(rx: &mpsc::Receiver<Msg>, n: usize, timeout: Duration, want: fn(&Msg) -> bool) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    let mut got = 0;
    while got < n {
        match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(Msg::Failed) => return Err("a picture could not be loaded".into()),
            Ok(m) if want(&m) => got += 1,
            Ok(_) => {}
            Err(_) => return Err("timed out".into()),
        }
    }
    Ok(())
}

fn destroy_pool(app: &AppHandle) {
    let old = app.state::<AppState>().anim.lock().unwrap().take();
    if let Some(p) = old {
        for w in &p.wins {
            if let Some(win) = app.get_webview_window(&w.label) {
                let _ = win.destroy();
            }
        }
    }
}

/// Returns the (possibly just created) windows. `rx` receives the pages' "up" messages.
fn ensure_pool(app: &AppHandle, rx: &mpsc::Receiver<Msg>, layer: Layer) -> Result<(Vec<Win>, bool), String> {
    let st = app.state::<AppState>();
    let mons = monitors(app)?;
    if let Some(p) = st.anim.lock().unwrap().as_ref() {
        let alive = p.wins.iter().all(|w| app.get_webview_window(&w.label).is_some());
        let same_layout = p.wins.len() == mons.len() && p.wins.iter().zip(&mons).all(|(w, (m, k))| w.mon == *m && w.work == *k);
        if p.layer == layer && alive && same_layout {
            return Ok((p.wins.clone(), false));
        }
    }
    destroy_pool(app); // monitors, layer or windows changed

    let run = RUN.fetch_add(1, Ordering::Relaxed);
    let mut wins = Vec::new();
    for (i, (mon, work)) in mons.iter().enumerate() {
        let label = format!("tr{run}x{i}");
        let built = WebviewWindowBuilder::new(app, &label, WebviewUrl::App(format!("transition.html?label={label}").into()))
            .title("WallFlip transition")
            .decorations(false)
            .resizable(false)
            .skip_taskbar(true)
            .always_on_top(layer == Layer::Top)
            .shadow(false)
            .focusable(false)
            .focused(false)
            .visible(false)
            .build();
        wins.push(Win { label, mon: *mon, work: *work });
        if let Err(e) = built {
            for w in &wins {
                if let Some(win) = app.get_webview_window(&w.label) {
                    let _ = win.destroy();
                }
            }
            return Err(e.to_string());
        }
    }
    let register = |wins: Vec<Win>| {
        *st.anim.lock().unwrap() = Some(Pool { wins, layer, last_used: Instant::now(), busy: false, generation: 0, refresh: None });
    };
    register(wins.clone());
    if let Err(e) = wait_for(rx, wins.len(), READY_TIMEOUT, |m| matches!(m, Msg::Up)) {
        destroy_pool(app);
        return Err(format!("transition pages did not start: {e}"));
    }
    Ok((wins, true))
}

#[cfg(windows)]
fn raw_hwnd(win: &WebviewWindow) -> Result<*mut std::ffi::c_void, String> {
    win.hwnd().map(|h| h.0 as *mut std::ffi::c_void).map_err(|e| e.to_string())
}
#[cfg(not(windows))]
fn raw_hwnd(_win: &WebviewWindow) -> Result<*mut std::ffi::c_void, String> {
    Ok(std::ptr::null_mut())
}

/// Moves a ready window into place and shows it.
fn place(app: &AppHandle, w: &Win, layer: Layer) -> Result<(), String> {
    let win = app.get_webview_window(&w.label).ok_or("transition window vanished")?;
    match layer {
        Layer::Desktop => desktop::attach(raw_hwnd(&win)?, w.mon.x, w.mon.y, w.mon.w as i32, w.mon.h as i32),
        Layer::Top => {
            win.set_position(PhysicalPosition::new(w.work.x, w.work.y)).map_err(|e| e.to_string())?;
            win.set_size(PhysicalSize::new(w.work.w, w.work.h)).map_err(|e| e.to_string())?;
            win.show().map_err(|e| e.to_string())?;
            // after show(): the GTK backend has no native window to configure before that
            let _ = win.set_ignore_cursor_events(true);
            Ok(())
        }
    }
}

/// Removes the animation from the screen. Desktop layer: raw Win32 detach (see `desktop::detach`) and
/// give Explorer's wallpaper layer back; top layer: ordinary hide. The page frees its textures afterwards.
fn hide_all(app: &AppHandle, wins: &[Win], layer: Layer) {
    for w in wins {
        if let Some(win) = app.get_webview_window(&w.label) {
            match layer {
                Layer::Desktop => {
                    if let Ok(raw) = raw_hwnd(&win) {
                        desktop::detach(raw);
                    }
                }
                Layer::Top => {
                    let _ = win.hide();
                }
            }
        }
        let _ = app.emit_to(w.label.as_str(), "transition-reset", ());
    }
    if layer == Layer::Desktop {
        desktop::release_layer();
    }
}

/// Called when the gallery opens. A finished animation lingers (attached to the desktop) while Windows
/// applies the picture; in that state our input is tied to Explorer's and the gallery cannot get the
/// keyboard. Take the animation down right away; the picture is re-applied a moment later instead.
pub fn release_now(app: &AppHandle) {
    let st = app.state::<AppState>();
    let Ok(_turn) = st.apply_lock.try_lock() else { return }; // an animation is running: leave it
    let taken = {
        let mut g = st.anim.lock().unwrap();
        match g.as_mut().filter(|p| p.busy) {
            Some(p) => {
                p.busy = false;
                p.generation += 1; // cancels the pending cleanup of that job
                p.last_used = Instant::now();
                Some((p.wins.clone(), p.layer, p.refresh.take(), p.generation))
            }
            None => None,
        }
    };
    drop(_turn);
    let Some((wins, layer, refresh, generation)) = taken else { return };
    hide_all(app, &wins, layer);
    if let Some((path, fit)) = refresh {
        let app = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(250));
            let st = app.state::<AppState>();
            let _turn = st.apply_lock.lock().unwrap_or_else(|e| e.into_inner());
            // only if nothing else changed the wallpaper meanwhile
            if st.anim.lock().unwrap().as_ref().is_some_and(|p| p.generation == generation) {
                let _ = wallpaper::set(&path, fit);
            }
        });
    }
}

/// Windows steals the keyboard focus while it boots a new browser view; give it back to the gallery.
fn refocus_gallery(app: &AppHandle) {
    for delay in [0u64, 150, 400] {
        std::thread::sleep(Duration::from_millis(delay));
        crate::refocus_overlay(app);
    }
}

/// Prepares the windows ahead of time (called when the gallery opens). No-op when they exist.
pub fn warm(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let st = app.state::<AppState>();
        let (kind, layer) = {
            let c = st.cfg.lock().unwrap();
            (c.transition, c.transition_layer)
        };
        if kind == Transition::Native {
            return;
        }
        let _turn = st.apply_lock.lock().unwrap_or_else(|e| e.into_inner());
        let (tx, rx) = mpsc::channel();
        *st.trans_tx.lock().unwrap() = Some(tx);
        let created = match ensure_pool(&app, &rx, layer) {
            Ok((_, created)) => {
                if let Some(p) = st.anim.lock().unwrap().as_mut() {
                    p.last_used = Instant::now();
                }
                created
            }
            Err(_) => false,
        };
        *st.trans_tx.lock().unwrap() = None;
        if created {
            refocus_gallery(&app);
        }
    });
}

/// Background janitor: destroys the windows once they have been idle for `IDLE`.
pub fn start_reaper(app: AppHandle) {
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(15));
        let st = app.state::<AppState>();
        let expired = st.anim.lock().unwrap().as_ref().is_some_and(|p| !p.busy && p.last_used.elapsed() > IDLE);
        if expired {
            let _turn = st.apply_lock.lock().unwrap_or_else(|e| e.into_inner());
            let still = st.anim.lock().unwrap().as_ref().is_some_and(|p| !p.busy && p.last_used.elapsed() > IDLE);
            if still {
                destroy_pool(&app);
            }
        }
    });
}

/// Sets `path` as the wallpaper, animating from the current one when possible.
/// Blocking (about `ms` long): call it from a worker thread, never the UI thread.
pub fn apply(app: &AppHandle, path: &Path, fit: Fit, kind: Transition, ms: u32) -> Result<(), String> {
    let st = app.state::<AppState>();
    // one change at a time: a second hotkey press waits for the running animation
    let _turn = st.apply_lock.lock().unwrap_or_else(|e| e.into_inner());

    let kind = resolve(kind);
    let (hold, layer) = {
        let c = st.cfg.lock().unwrap();
        (Duration::from_millis(u64::from(c.transition_hold_ms)), c.transition_layer)
    };
    if kind != Transition::Native {
        if let Some(old) = current_picture(app).filter(|o| !same(o, path)) {
            match play(app, &old, path, fit, kind, ms, hold, layer) {
                Ok(result) => return result,
                Err(_e) => {
                    // could not animate: fall back to the plain change below
                    #[cfg(debug_assertions)]
                    eprintln!("wallflip: animation skipped: {_e}");
                }
            }
        }
    }
    wallpaper::set(path, fit)
}

/// Outer `Err` = the animation could not run (wallpaper untouched); inner = result of setting it.
#[allow(clippy::too_many_arguments)]
fn play(
    app: &AppHandle,
    old: &Path,
    new: &Path,
    fit: Fit,
    kind: Transition,
    ms: u32,
    hold: Duration,
    layer: Layer,
) -> Result<Result<(), String>, String> {
    let st = app.state::<AppState>();
    let (tx, rx) = mpsc::channel();
    *st.trans_tx.lock().unwrap() = Some(tx);

    let outcome = (|| -> Result<(Vec<Win>, Result<(), String>, Instant, u64), String> {
        let (wins, _) = ensure_pool(app, &rx, layer)?;
        let generation = {
            let mut g = st.anim.lock().unwrap();
            let p = g.as_mut().ok_or("transition windows missing")?;
            p.generation += 1;
            p.busy = true;
            p.last_used = Instant::now();
            p.generation
        };

        // 1. hand every page its job; wait until both pictures are decoded
        for w in &wins {
            // centre/tile must keep native pixel size, everything else is shrunk to the screen
            let (qw, qh) = if matches!(fit, Fit::Center | Fit::Tile) { (0, 0) } else { (w.mon.w, w.mon.h) };
            // desktop layer: the window covers the monitor; top layer: only the work area (not the taskbar)
            let area = if layer == Layer::Desktop { w.mon } else { w.work };
            let job = Job {
                from: hex(old),
                to: hex(new),
                kind: &kind.key(),
                ms,
                fit: fit_key(fit),
                w: qw,
                h: qh,
                mw: w.mon.w,
                mh: w.mon.h,
                ox: w.mon.x - area.x,
                oy: w.mon.y - area.y,
            };
            app.emit_to(w.label.as_str(), "transition-load", &job).map_err(|e| e.to_string())?;
        }
        wait_for(&rx, wins.len(), READY_TIMEOUT, |m| matches!(m, Msg::Ready))?;

        // 2. put the windows in place, let them paint both pictures once
        for w in &wins {
            place(app, w, layer)?;
        }
        for w in &wins {
            let _ = app.emit_to(w.label.as_str(), "transition-show", ());
        }
        let _ = wait_for(&rx, wins.len(), PAINT_TIMEOUT, |m| matches!(m, Msg::Painted)); // start anyway

        // 3. change the real wallpaper NOW, hidden under the animation: Windows needs a moment to
        //    decode and fade, and that time is spent under cover instead of after the animation
        let started = Instant::now();
        let result = std::thread::scope(|scope| {
            let setter = scope.spawn(|| wallpaper::set(new, fit));
            for w in &wins {
                let _ = app.emit_to(w.label.as_str(), "transition-go", ());
            }
            // a hung page only delays us, it cannot block us
            let _ = wait_for(&rx, wins.len(), Duration::from_millis(u64::from(ms)) + Duration::from_millis(2500), |m| {
                matches!(m, Msg::Done)
            });
            setter.join().unwrap_or_else(|_| Err("wallpaper thread crashed".into()))
        });
        Ok((wins, result, started, generation))
    })();

    *st.trans_tx.lock().unwrap() = None;

    match outcome {
        Err(e) => {
            // nothing was changed: remove whatever is showing
            let wins = st.anim.lock().unwrap().as_mut().map(|p| {
                p.busy = false;
                p.wins.clone()
            });
            if let Some(wins) = wins {
                hide_all(app, &wins, layer);
            }
            Err(e)
        }
        Ok((wins, result, started, generation)) => {
            if result.is_err() {
                st.animated_failure.store(true, Ordering::SeqCst);
            } else if layer == Layer::Desktop {
                if let Some(p) = st.anim.lock().unwrap().as_mut().filter(|p| p.generation == generation) {
                    p.refresh = Some((new.to_path_buf(), fit));
                }
            }
            // Keep covering until Windows has applied the new picture -- but in the background, so the
            // next change is not delayed. A newer job takes over the windows and cancels this cleanup.
            let app = app.clone();
            let ok = result.is_ok();
            let wait = if ok { hold.saturating_sub(started.elapsed()) } else { Duration::ZERO };
            std::thread::spawn(move || {
                std::thread::sleep(wait);
                let st = app.state::<AppState>();
                // no new change may start while we take the animation down
                let _turn = st.apply_lock.lock().unwrap_or_else(|e| e.into_inner());
                let refresh = {
                    let mut g = st.anim.lock().unwrap();
                    match g.as_mut().filter(|p| p.generation == generation) {
                        Some(p) => {
                            p.busy = false;
                            p.last_used = Instant::now();
                            Some(p.refresh.take())
                        }
                        None => None, // a newer change (or the gallery) owns the windows now
                    }
                };
                if let Some(refresh) = refresh {
                    hide_all(&app, &wins, layer);
                    if let (true, Some((path, fit))) = (ok, refresh) {
                        // Explorer may leave its wallpaper layer blank after the animation layer is
                        // removed: ask it to paint the (already applied) picture once more
                        std::thread::sleep(Duration::from_millis(200));
                        let _ = wallpaper::set(&path, fit);
                    }
                }
            });
            Ok(result)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_roundtrip_including_unicode() {
        for s in ["C:\\Wall papers\\a b.jpg", "/tmp/x/ünï-壁紙.png"] {
            assert_eq!(unhex(&hex(Path::new(s))).as_deref(), Some(Path::new(s)));
        }
        assert!(unhex("").is_none());
        assert!(unhex("abc").is_none());
        assert!(unhex("zz").is_none());
    }

    #[test]
    fn random_never_picks_native_or_random() {
        for _ in 0..50 {
            let t = resolve(Transition::Random);
            assert!(t != Transition::Random && t != Transition::Native);
        }
        assert_eq!(resolve(Transition::Wipe), Transition::Wipe);
    }
}
