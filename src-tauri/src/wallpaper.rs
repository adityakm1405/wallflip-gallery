//! Setting and reading the desktop wallpaper.
//!
//! `IDesktopWallpaper` gives Windows' native cross-fade and honours Fill/Fit/Span.
//! `SystemParametersInfo` is used only when the COM object cannot be created at
//! all — NOT when one image fails, because SPI can blank the desktop on a bad
//! file whereas IDesktopWallpaper simply returns an error.

use crate::config::Fit;
use std::path::{Path, PathBuf};

#[cfg(windows)]
pub fn set(path: &Path, fit: Fit) -> Result<(), String> {
    imp::set(path, fit)
}

#[cfg(windows)]
pub fn current() -> Option<PathBuf> {
    imp::current()
}

#[cfg(not(windows))]
pub fn set(_path: &Path, _fit: Fit) -> Result<(), String> {
    Err("Setting the wallpaper is only supported on Windows".into())
}

#[cfg(not(windows))]
pub fn current() -> Option<PathBuf> {
    None
}

#[cfg(windows)]
mod imp {
    use super::{Fit, Path, PathBuf};
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use windows::core::{w, PCWSTR};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL,
        COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE,
        REG_OPTION_NON_VOLATILE, REG_SZ,
    };
    use windows::Win32::UI::Shell::{
        DesktopWallpaper, IDesktopWallpaper, DESKTOP_WALLPAPER_POSITION, DWPOS_CENTER, DWPOS_FILL,
        DWPOS_FIT, DWPOS_SPAN, DWPOS_STRETCH, DWPOS_TILE,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        SystemParametersInfoW, SPIF_SENDCHANGE, SPIF_UPDATEINIFILE, SPI_SETDESKWALLPAPER,
    };

    /// Initialises COM on the calling thread for the lifetime of the guard.
    struct ComGuard(bool);
    impl ComGuard {
        fn new() -> Self {
            // S_OK / S_FALSE => we must balance with CoUninitialize.
            // RPC_E_CHANGED_MODE => thread already has COM in another mode: usable, don't uninit.
            ComGuard(unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok())
        }
    }
    impl Drop for ComGuard {
        fn drop(&mut self) {
            if self.0 {
                unsafe { CoUninitialize() };
            }
        }
    }

    fn dwpos(fit: Fit) -> DESKTOP_WALLPAPER_POSITION {
        match fit {
            Fit::Fill => DWPOS_FILL,
            Fit::Fit => DWPOS_FIT,
            Fit::Stretch => DWPOS_STRETCH,
            Fit::Tile => DWPOS_TILE,
            Fit::Center => DWPOS_CENTER,
            Fit::Span => DWPOS_SPAN,
        }
    }

    /// NUL-terminated UTF-16 with `/` normalised to `\`.
    fn path_wide(p: &Path) -> Vec<u16> {
        p.as_os_str()
            .encode_wide()
            .map(|c| if c == b'/' as u16 { b'\\' as u16 } else { c })
            .chain(std::iter::once(0))
            .collect()
    }

    pub fn set(path: &Path, fit: Fit) -> Result<(), String> {
        let _com = ComGuard::new();
        let wpath = path_wide(path);
        let dw: IDesktopWallpaper = match unsafe { CoCreateInstance(&DesktopWallpaper, None, CLSCTX_ALL) } {
            Ok(dw) => dw,
            Err(_) => return spi_fallback(&wpath, fit),
        };
        unsafe {
            // SetPosition returns S_FALSE (still success) when nothing changes.
            let _ = dw.SetPosition(dwpos(fit));
            dw.SetWallpaper(PCWSTR::null(), PCWSTR(wpath.as_ptr()))
        }
        .map_err(|e| e.message())
    }

    /// Wallpaper currently shown; `None` when monitors differ or none is set.
    pub fn current() -> Option<PathBuf> {
        let _com = ComGuard::new();
        unsafe {
            let dw: IDesktopWallpaper = CoCreateInstance(&DesktopWallpaper, None, CLSCTX_ALL).ok()?;
            let p = dw.GetWallpaper(PCWSTR::null()).ok()?;
            let mut len = 0;
            while *p.0.add(len) != 0 {
                len += 1;
            }
            let path = PathBuf::from(std::ffi::OsString::from_wide(std::slice::from_raw_parts(p.0, len)));
            CoTaskMemFree(Some(p.0 as *const _));
            if path.as_os_str().is_empty() {
                None
            } else {
                Some(path)
            }
        }
    }

    fn reg_set_string(subkey: PCWSTR, name: PCWSTR, value: &str) {
        unsafe {
            let mut key = HKEY::default();
            let rc = RegCreateKeyExW(
                HKEY_CURRENT_USER,
                subkey,
                None,
                PCWSTR::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_SET_VALUE,
                None,
                &mut key,
                None,
            );
            if rc.is_err() {
                return;
            }
            let data: Vec<u16> = value.encode_utf16().chain(std::iter::once(0)).collect();
            let bytes = std::slice::from_raw_parts(data.as_ptr() as *const u8, data.len() * 2);
            let _ = RegSetValueExW(key, name, None, REG_SZ, Some(bytes));
            let _ = RegCloseKey(key);
        }
    }

    fn spi_fallback(wpath: &[u16], fit: Fit) -> Result<(), String> {
        let (style, tile) = match fit {
            Fit::Fill => ("10", "0"),
            Fit::Fit => ("6", "0"),
            Fit::Stretch => ("2", "0"),
            Fit::Tile => ("0", "1"),
            Fit::Center => ("0", "0"),
            Fit::Span => ("22", "0"),
        };
        reg_set_string(w!("Control Panel\\Desktop"), w!("WallpaperStyle"), style);
        reg_set_string(w!("Control Panel\\Desktop"), w!("TileWallpaper"), tile);
        unsafe {
            SystemParametersInfoW(
                SPI_SETDESKWALLPAPER,
                0,
                Some(wpath.as_ptr() as *mut _),
                SPIF_UPDATEINIFILE | SPIF_SENDCHANGE,
            )
        }
        .map_err(|e| e.message())
    }
}
