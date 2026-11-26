//! Settings, stored as JSON in the app config dir (`%APPDATA%\com.wallflip.app\config.json`).

use crate::playlist::Mode;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::Path;

pub const DEFAULT_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "jpe", "jfif", "png", "bmp", "dib", "gif", "tif", "tiff", "webp", "jxr",
];

/// How the picture is fitted to the screen (maps 1:1 to `DESKTOP_WALLPAPER_POSITION`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[allow(clippy::enum_variant_names)] // names mirror Windows' DESKTOP_WALLPAPER_POSITION
pub enum Fit {
    Fill,
    Fit,
    Stretch,
    Tile,
    Center,
    Span,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StyleSetting {
    Carousel,
    Depth,
    Fan,
    Wall,
    Random,
}

/// Animation played when the wallpaper changes.
/// `Native` = Windows' own cross-fade (no overlay window); `Random` picks one of the others each time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Transition {
    Native,
    Fade,
    Slide,
    Push,
    Wipe,
    Diagonal,
    Circle,
    Diamond,
    Zoom,
    Zoomout,
    Blur,
    Blinds,
    Tiles,
    Flip,
    Random,
}

impl Transition {
    /// Every animation `Random` can choose from.
    pub const ANIMATED: [Transition; 13] = [
        Transition::Fade,
        Transition::Slide,
        Transition::Push,
        Transition::Wipe,
        Transition::Diagonal,
        Transition::Circle,
        Transition::Diamond,
        Transition::Zoom,
        Transition::Zoomout,
        Transition::Blur,
        Transition::Blinds,
        Transition::Tiles,
        Transition::Flip,
    ];

    /// Name used by the transition page (same as the serde name).
    pub fn key(self) -> String {
        serde_json::to_value(self).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default()
    }
}

/// Where the animation is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Layer {
    /// On the desktop, behind the icons and all windows (recommended; Windows 11 24H2+ and older).
    Desktop,
    /// Above everything (hides icons and windows while it plays). Fallback if `Desktop` shows nothing.
    Top,
}

pub const MAX_HOLD_MS: u32 = 6000;
pub const MIN_TRANSITION_MS: u32 = 200;
pub const MAX_TRANSITION_MS: u32 = 3000;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub folder: Option<String>,
    pub recursive: bool,
    pub style: StyleSetting,
    /// Opens/closes the gallery overlay.
    pub gallery_hotkey: String,
    /// Instantly applies the next wallpaper (uses `mode`).
    pub cycle_hotkey: String,
    pub mode: Mode,
    pub fit: Fit,
    /// Animation for changes you trigger (gallery, quick-cycle). Auto-change always uses `Native`.
    pub transition: Transition,
    /// Length of the animation in milliseconds.
    pub transition_ms: u32,
    pub transition_layer: Layer,
    /// How long the animation overlay stays up after Windows is told to change the wallpaper, so the
    /// desktop has finished applying it. Raise it if the old wallpaper ever flashes back at the end.
    pub transition_hold_ms: u32,
    /// Auto-change interval in minutes; 0 = off.
    pub interval_min: u32,
    /// Lower-case extensions without the dot.
    pub extensions: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            folder: None,
            recursive: false,
            style: StyleSetting::Carousel,
            gallery_hotkey: "Alt+W".into(),
            cycle_hotkey: "Alt+Shift+W".into(),
            mode: Mode::NoRepeat,
            fit: Fit::Fill,
            transition: Transition::Circle,
            transition_ms: 900,
            transition_layer: Layer::Desktop,
            transition_hold_ms: 1800,
            interval_min: 0,
            extensions: DEFAULT_EXTENSIONS.iter().map(|s| s.to_string()).collect(),
        }
    }
}

impl Config {
    /// Cleans user-supplied values; returns an error message for unusable ones.
    pub fn normalize(&mut self) -> Result<(), String> {
        if self.interval_min > 24 * 60 {
            return Err("Auto-change interval must be at most 24 hours.".into());
        }
        self.transition_ms = self.transition_ms.clamp(MIN_TRANSITION_MS, MAX_TRANSITION_MS);
        self.transition_hold_ms = self.transition_hold_ms.min(MAX_HOLD_MS);
        self.extensions = self
            .extensions
            .iter()
            .map(|e| e.trim().trim_start_matches('.').to_ascii_lowercase())
            .filter(|e| !e.is_empty())
            .collect();
        if self.extensions.is_empty() {
            self.extensions = DEFAULT_EXTENSIONS.iter().map(|s| s.to_string()).collect();
        }
        if let Some(f) = &self.folder {
            let f = f.trim().trim_matches('"');
            self.folder = if f.is_empty() { None } else { Some(f.to_string()) };
        }
        Ok(())
    }

    /// Never fails: a missing file gives defaults; a broken one gives defaults + a warning.
    pub fn load(path: &Path) -> (Config, Option<String>) {
        match fs::read_to_string(path) {
            Ok(text) => match serde_json::from_str::<Config>(text.trim_start_matches('\u{feff}')) {
                Ok(mut c) => {
                    let _ = c.normalize();
                    (c, None)
                }
                Err(e) => (Config::default(), Some(format!("config.json is invalid ({e}); using defaults"))),
            },
            Err(e) if e.kind() == io::ErrorKind::NotFound => (Config::default(), None),
            Err(e) => (Config::default(), Some(format!("cannot read config: {e}"))),
        }
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(self).map_err(io::Error::other)?)?;
        fs::rename(&tmp, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_shape_matches_the_typescript_side() {
        let v = serde_json::to_value(Config::default()).unwrap();
        assert_eq!(v["style"], "carousel");
        assert_eq!(v["mode"], "norepeat");
        assert_eq!(v["fit"], "fill");
        assert_eq!(v["transition"], "circle");
        assert_eq!(v["transition_ms"], 900);
        assert_eq!(v["transition_layer"], "desktop");
        assert_eq!(serde_json::to_value(Transition::Zoomout).unwrap(), "zoomout");
        assert_eq!(Transition::Flip.key(), "flip");
        assert_eq!(v["gallery_hotkey"], "Alt+W");
        assert_eq!(v["cycle_hotkey"], "Alt+Shift+W");
        assert!(v["folder"].is_null());
    }

    #[test]
    fn partial_json_fills_defaults_and_roundtrips() {
        let c: Config = serde_json::from_str(r#"{"folder":"D:\\Pics","style":"fan","interval_min":15}"#).unwrap();
        assert_eq!(c.folder.as_deref(), Some("D:\\Pics"));
        assert_eq!(c.style, StyleSetting::Fan);
        assert_eq!(c.mode, Mode::NoRepeat);
        let back: Config = serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
        assert_eq!(back, c);
    }

    #[test]
    fn normalize_cleans_values() {
        let mut c = Config { extensions: vec![" .PNG".into(), "".into()], folder: Some("  \"C:\\x\" ".into()), ..Config::default() };
        c.normalize().unwrap();
        assert_eq!(c.extensions, ["png"]);
        assert_eq!(c.folder.as_deref(), Some("C:\\x"));
        let mut slow = Config { transition_ms: 60_000, ..Config::default() };
        slow.normalize().unwrap();
        assert_eq!(slow.transition_ms, MAX_TRANSITION_MS);
        let mut bad = Config { interval_min: 99_999, ..Config::default() };
        assert!(bad.normalize().is_err());
    }

    #[test]
    fn save_load_and_bad_file() {
        let dir = std::env::temp_dir().join(format!("wallflip-cfg-{}", std::process::id()));
        let path = dir.join("config.json");
        let c = Config { folder: Some("/some/dir".into()), ..Config::default() };
        c.save(&path).unwrap();
        assert_eq!(Config::load(&path), (c, None));
        fs::write(&path, "{not json").unwrap();
        let (d, warn) = Config::load(&path);
        assert_eq!(d, Config::default());
        assert!(warn.is_some());
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(Config::load(&dir.join("nope.json")), (Config::default(), None));
    }
}
