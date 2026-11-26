//! Puts a window on the *desktop layer*: above Windows' own wallpaper, below the desktop
//! icons and below every app window. Used for the change animation so it never hides the
//! icons or your windows.
//!
//! Windows 11 24H2+ ("raised desktop"): the icon view (`SHELLDLL_DefView`) is a child of
//! `Progman` and Explorer's wallpaper `WorkerW` is a child below it. A *layered* child of
//! `Progman` inserted directly under the icon view is composited between the two.
//! Older builds: after the `0x052C` message the wallpaper lives in a top-level `WorkerW`
//! sibling that follows the icon host; our window becomes its child.

use std::ffi::c_void;

/// Re-parents the window `hwnd` onto the desktop layer and shows it at the given screen
/// rectangle (physical pixels). Errors leave the window top-level and untouched.
#[cfg(windows)]
pub fn attach(hwnd: *mut c_void, x: i32, y: i32, w: i32, h: i32) -> Result<(), String> {
    imp::attach(hwnd, x, y, w, h)
}

/// Hides the window and turns it back into an ordinary top-level window. Done with raw Win32 calls:
/// the windowing library's own `hide()` re-applies its window style and would undo the attachment.
#[cfg(windows)]
pub fn detach(hwnd: *mut c_void) {
    imp::detach(hwnd)
}

/// Gives Explorer's wallpaper layer back if `attach` had to create it, so the desktop returns to
/// its normal state (the layer is reference-counted by Explorer).
#[cfg(windows)]
pub fn release_layer() {
    imp::release_layer()
}

#[cfg(not(windows))]
pub fn detach(_hwnd: *mut c_void) {}

#[cfg(not(windows))]
pub fn release_layer() {}

#[cfg(not(windows))]
pub fn attach(_hwnd: *mut c_void, _x: i32, _y: i32, _w: i32, _h: i32) -> Result<(), String> {
    Err("the desktop layer is only available on Windows".into())
}

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicBool, Ordering};
    use windows::core::{w, BOOL, PCWSTR};
    use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, RECT, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, FindWindowExW, FindWindowW, GetParent, GetWindowLongPtrW, GetWindowRect,
        IsWindowVisible, SendMessageTimeoutW, SetLayeredWindowAttributes, SetParent,
        SetWindowLongPtrW, SetWindowPos, GWL_EXSTYLE, GWL_STYLE, LWA_ALPHA, SMTO_NORMAL,
        SWP_HIDEWINDOW, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SWP_SHOWWINDOW, WS_CAPTION, WS_CHILD, WS_EX_LAYERED, WS_EX_NOACTIVATE,
        WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, WS_THICKFRAME, WS_VISIBLE,
    };

    /// Explorer message that creates the separate wallpaper layer; `(0xD, 1)` is what Lively sends.
    const SPAWN_WORKERW: u32 = 0x052C;
    /// true while the wallpaper layer exists because *we* asked for it
    static SPAWNED: AtomicBool = AtomicBool::new(false);

    fn icon_view_in(parent: HWND) -> Option<HWND> {
        unsafe { FindWindowExW(Some(parent), None, w!("SHELLDLL_DefView"), PCWSTR::null()).ok() }
    }

    /// Classic layout: the top-level window that hosts the icon view.
    fn icon_host() -> Option<HWND> {
        unsafe extern "system" fn each(hwnd: HWND, lparam: LPARAM) -> BOOL {
            if icon_view_in(hwnd).is_some() {
                unsafe { *(lparam.0 as *mut Option<HWND>) = Some(hwnd) };
                return BOOL(0); // stop
            }
            BOOL(1)
        }
        let mut found: Option<HWND> = None;
        unsafe {
            let _ = EnumWindows(Some(each), LPARAM(&mut found as *mut Option<HWND> as isize));
        }
        found
    }

    fn rect_of(hwnd: HWND) -> Result<RECT, String> {
        let mut r = RECT::default();
        unsafe { GetWindowRect(hwnd, &mut r) }.map_err(|e| e.message())?;
        Ok(r)
    }

    /// WS_POPUP -> WS_CHILD (SetParent on a popup does not behave), no activation, no taskbar button.
    fn make_child_style(hwnd: HWND, layered: bool) -> Result<(), String> {
        unsafe {
            let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
            let style = (style & !(WS_POPUP.0 | WS_CAPTION.0 | WS_THICKFRAME.0)) | WS_CHILD.0 | WS_VISIBLE.0;
            SetWindowLongPtrW(hwnd, GWL_STYLE, style as isize);
            let mut ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
            ex = (ex & !WS_EX_TOPMOST.0) | WS_EX_NOACTIVATE.0 | WS_EX_TOOLWINDOW.0;
            if layered {
                ex |= WS_EX_LAYERED.0; // a plain child of Progman is never composited on 24H2+
            }
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex as isize);
            if layered {
                SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA).map_err(|e| e.message())?;
            }
        }
        Ok(())
    }

    pub fn attach(raw: *mut c_void, x: i32, y: i32, w: i32, h: i32) -> Result<(), String> {
        let hwnd = HWND(raw);
        let progman = unsafe { FindWindowW(w!("Progman"), PCWSTR::null()) }
            .map_err(|_| "desktop window (Progman) not found".to_string())?;

        // Ask Explorer for its wallpaper layer, once: repeating the message can undo it.
        let has_layer = unsafe { FindWindowExW(Some(progman), None, w!("WorkerW"), PCWSTR::null()) }.is_ok()
            || icon_host().is_some_and(|host| host != progman);
        if !has_layer {
            unsafe {
                SendMessageTimeoutW(progman, SPAWN_WORKERW, WPARAM(0xD), LPARAM(1), SMTO_NORMAL, 1000, None);
            }
            SPAWNED.store(true, Ordering::SeqCst);
            std::thread::sleep(std::time::Duration::from_millis(60));
        }

        let (parent, insert_after, layered) = if let Some(view) = icon_view_in(progman) {
            // raised desktop (24H2+): child of Progman, directly under the icon view
            (progman, Some(view), true)
        } else if let Some(host) = icon_host() {
            // classic: child of the wallpaper WorkerW that follows the icon host
            let target = unsafe { FindWindowExW(None, Some(host), w!("WorkerW"), PCWSTR::null()) }
                .map_err(|_| "wallpaper window (WorkerW) not found".to_string())?;
            (target, None, false)
        } else {
            return Err("desktop layout not recognised".into());
        };

        make_child_style(hwnd, layered)?;
        // The return value is the *previous* parent (null for a top-level window), so it says nothing
        // about success: verify with GetParent instead.
        unsafe {
            let _ = SetParent(hwnd, Some(parent));
        }
        if unsafe { GetParent(hwnd) }.ok() != Some(parent) {
            return Err("could not attach to the desktop layer".into());
        }

        // child coordinates are relative to the parent's top-left corner
        let origin = rect_of(parent)?;
        unsafe {
            SetWindowPos(hwnd, insert_after, x - origin.left, y - origin.top, w, h, SWP_NOACTIVATE | SWP_SHOWWINDOW)
                .map_err(|e| e.message())?;
        }
        if unsafe { IsWindowVisible(hwnd) }.as_bool() {
            Ok(())
        } else {
            Err("desktop-layer window is not visible".into())
        }
    }

    pub fn detach(raw: *mut c_void) {
        let hwnd = HWND(raw);
        unsafe {
            let _ = SetWindowPos(
                hwnd,
                None,
                0,
                0,
                0,
                0,
                SWP_HIDEWINDOW | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
            let _ = SetParent(hwnd, None);
            let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
            SetWindowLongPtrW(hwnd, GWL_STYLE, ((style & !WS_CHILD.0) | WS_POPUP.0) as isize);
        }
    }

    pub fn release_layer() {
        if !SPAWNED.swap(false, Ordering::SeqCst) {
            return;
        }
        unsafe {
            if let Ok(progman) = FindWindowW(w!("Progman"), PCWSTR::null()) {
                SendMessageTimeoutW(progman, SPAWN_WORKERW, WPARAM(0xD), LPARAM(0), SMTO_NORMAL, 1000, None);
            }
        }
    }
}
