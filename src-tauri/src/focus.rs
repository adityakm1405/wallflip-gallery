//! Reliable "make this window the active one" for the gallery.
//!
//! Windows only lets a process take the foreground in a few situations; when the desktop itself
//! has the focus (no app window open) a plain request can be refused and keystrokes then go to the
//! desktop. Briefly attaching to the foreground thread's input state is the documented way around it.

use std::ffi::c_void;

#[cfg(windows)]
pub fn force_foreground(hwnd: *mut c_void) {
    imp::force_foreground(hwnd)
}

#[cfg(not(windows))]
#[allow(dead_code)]
pub fn force_foreground(_hwnd: *mut c_void) {}

/// Alt+W is released as "W up, then Alt up". A lone Alt press-and-release makes Windows enter menu mode in
/// whichever window is active, and in that mode the arrow keys and the wheel go to the window's system menu
/// (Up shows Minimize/Maximize/Close) instead of the page. Sending an unassigned key while Alt is still down
/// marks the Alt as "used", so no menu mode starts. (Same trick AutoHotkey uses.)
#[cfg(windows)]
pub fn mask_alt_tap() {
    imp::mask_alt_tap()
}

#[cfg(not(windows))]
#[allow(dead_code)]
pub fn mask_alt_tap() {}

/// Removes the system menu from the gallery window so there is no menu for Alt/Up to open.
#[cfg(windows)]
pub fn strip_system_menu(hwnd: *mut c_void) {
    imp::strip_system_menu(hwnd)
}

#[cfg(not(windows))]
#[allow(dead_code)]
pub fn strip_system_menu(_hwnd: *mut c_void) {}

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, SetFocus, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
        VIRTUAL_KEY,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        BringWindowToTop, GetForegroundWindow, GetWindowLongPtrW, GetWindowThreadProcessId, SetForegroundWindow,
        SetWindowLongPtrW, GWL_STYLE, WS_SYSMENU,
    };

    pub fn mask_alt_tap() {
        unsafe {
            let key = |flags| INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VIRTUAL_KEY(0xE8), // unassigned virtual key
                        wScan: 0,
                        dwFlags: flags,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            };
            let inputs = [key(KEYBD_EVENT_FLAGS(0)), key(KEYEVENTF_KEYUP)];
            SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
        }
    }

    pub fn strip_system_menu(raw: *mut c_void) {
        unsafe {
            let hwnd = HWND(raw);
            let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
            SetWindowLongPtrW(hwnd, GWL_STYLE, style & !(WS_SYSMENU.0 as isize));
        }
    }

    /// Must run on the thread that owns the window (the UI thread).
    pub fn force_foreground(raw: *mut c_void) {
        let hwnd = HWND(raw);
        unsafe {
            let fg = GetForegroundWindow();
            let me = GetCurrentThreadId();
            let theirs = if fg.is_invalid() { 0 } else { GetWindowThreadProcessId(fg, None) };
            let attached = theirs != 0 && theirs != me && AttachThreadInput(me, theirs, true).as_bool();
            let _ = BringWindowToTop(hwnd);
            let _ = SetForegroundWindow(hwnd);
            let _ = SetFocus(Some(hwnd));
            if attached {
                let _ = AttachThreadInput(me, theirs, false);
            }
        }
    }
}
