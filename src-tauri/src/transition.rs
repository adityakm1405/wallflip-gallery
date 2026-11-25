//! Animated wallpaper changes.
//!
//! Windows can only cross-fade, so every other animation is drawn by us: one
//! click-through, always-on-top window per monitor (covering the work area, never the
//! taskbar) shows the old picture. The real wallpaper is changed underneath it while the
//! animation plays, and the overlay stays up until Windows has finished applying the new
//! picture (it can take a second or two for a 4K image). Then the windows are destroyed,
//! so nothing stays resident between changes.

use crate::config::{Fit, Transition};
use crate::{wallpaper, AppState};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindowBuilder};

/// Messages from the transition pages (see `transition_*` commands in main.rs).
pub enum Msg {
    Ready,
    Painted,
    Failed,
    Done,
}

/// Window labels must be unique: a window destroyed a moment ago may still be registered.
static RUN: AtomicU32 = AtomicU32::new(0);

/// How long a page may take to load and decode both pictures.
const READY_TIMEOUT: Duration = Duration::from_secs(5);
/// How long a shown window may take to paint both pictures once before we start anyway.
const PAINT_TIMEOUT: Duration = Duration::from_millis(1500);

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

/// Sets `path` as the wallpaper, animating from the current one when possible.
/// Blocking (about `ms` long): call it from a worker thread, never the UI thread.
pub fn apply(app: &AppHandle, path: &Path, fit: Fit, kind: Transition, ms: u32) -> Result<(), String> {
    let st = app.state::<AppState>();
    // one change at a time: a second hotkey press waits for the running animation
    let _turn = st.apply_lock.lock().unwrap_or_else(|e| e.into_inner());

    let kind = resolve(kind);
    let hold = Duration::from_millis(u64::from(st.cfg.lock().unwrap().transition_hold_ms));
    if kind != Transition::Native {
        if let Some(old) = current_picture(app).filter(|o| !same(o, path)) {
            match play(app, &old, path, fit, kind, ms, hold) {
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
fn play(
    app: &AppHandle,
    old: &Path,
    new: &Path,
    fit: Fit,
    kind: Transition,
    ms: u32,
    hold: Duration,
) -> Result<Result<(), String>, String> {
    let monitors = app.available_monitors().map_err(|e| e.to_string())?;
    if monitors.is_empty() {
        return Err("no monitors".into());
    }
    let (tx, rx) = mpsc::channel();
    *app.state::<AppState>().trans_tx.lock().unwrap() = Some(tx);

    let run = RUN.fetch_add(1, Ordering::Relaxed);
    let mut labels: Vec<String> = Vec::new();
    let outcome = (|| -> Result<Result<(), String>, String> {
        for (i, m) in monitors.iter().enumerate() {
            let label = format!("tr{run}x{i}");
            let (mw, mh) = (m.size().width, m.size().height);
            let work = m.work_area(); // the screen minus the taskbar: the overlay must not cover it
            let (ox, oy) = (m.position().x - work.position.x, m.position().y - work.position.y);
            // centre/tile must keep native pixel size, everything else is shrunk to the screen
            let (qw, qh) = if matches!(fit, Fit::Center | Fit::Tile) { (0, 0) } else { (mw, mh) };
            let url = format!(
                "transition.html?from={}&to={}&kind={}&ms={ms}&fit={}&w={qw}&h={qh}&mw={mw}&mh={mh}&ox={ox}&oy={oy}&label={label}",
                hex(old),
                hex(new),
                kind.key(),
                fit_key(fit),
            );
            let win = WebviewWindowBuilder::new(app, &label, WebviewUrl::App(url.into()))
                .title("WallFlip transition")
                .decorations(false)
                .resizable(false)
                .skip_taskbar(true)
                .always_on_top(true)
                .shadow(false)
                .focusable(false)
                .focused(false)
                .visible(false)
                .build()
                .map_err(|e| e.to_string())?;
            labels.push(label);
            let _ = win.set_position(PhysicalPosition::new(work.position.x, work.position.y));
            let _ = win.set_size(PhysicalSize::new(work.size.width, work.size.height));
        }

        // 1. every page has both pictures decoded
        let deadline = Instant::now() + READY_TIMEOUT;
        let mut ready = 0;
        while ready < labels.len() {
            match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(Msg::Ready) => ready += 1,
                Ok(Msg::Failed) => return Err("a picture could not be loaded".into()),
                Ok(_) => {}
                Err(_) => return Err("transition page did not start in time".into()),
            }
        }
        // 2. show and go
        for l in &labels {
            if let Some(w) = app.get_webview_window(l) {
                let _ = w.show();
                // after show(): the GTK backend has no native window to configure before that
                let _ = w.set_ignore_cursor_events(true);
            }
        }
        // 2b. every page has painted both pictures once (so the animation itself never stutters)
        for l in &labels {
            let _ = app.emit_to(l.as_str(), "transition-show", ());
        }
        let deadline = Instant::now() + PAINT_TIMEOUT;
        let mut painted = 0;
        while painted < labels.len() {
            match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(Msg::Painted) => painted += 1,
                Ok(_) => {}
                Err(_) => break, // start anyway
            }
        }

        // 3. change the real wallpaper NOW, hidden under the overlay (which still shows the old
        //    picture), while the animation plays on top. Windows needs a moment to decode and
        //    fade; that time is spent under cover instead of after the animation.
        let started = Instant::now();
        let result = std::thread::scope(|scope| {
            let setter = scope.spawn(|| wallpaper::set(new, fit));
            for l in &labels {
                let _ = app.emit_to(l.as_str(), "transition-go", ());
            }
            // animation finished everywhere (a hung page only delays us, it cannot block us)
            let deadline = Instant::now() + Duration::from_millis(u64::from(ms)) + Duration::from_millis(2500);
            let mut done = 0;
            while done < labels.len() {
                match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                    Ok(Msg::Done) => done += 1,
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
            setter.join().unwrap_or_else(|_| Err("wallpaper thread crashed".into()))
        });
        // 4. keep covering until Windows has had time to apply the new picture
        if result.is_ok() {
            std::thread::sleep(hold.saturating_sub(started.elapsed()));
        } else {
            app.state::<AppState>().animated_failure.store(true, Ordering::SeqCst);
        }
        Ok(result)
    })();

    *app.state::<AppState>().trans_tx.lock().unwrap() = None;
    for l in &labels {
        if let Some(w) = app.get_webview_window(l) {
            let _ = w.destroy();
        }
    }
    outcome
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
