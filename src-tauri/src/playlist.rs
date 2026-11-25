//! Cycle logic: sequential, shuffle and no-repeat (shuffle-bag) modes.
//!
//! Pure std, no Win32 — fully unit-tested on any OS. Memory use is one `PathBuf`
//! per image plus a small index bag.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

const HISTORY_MAX: usize = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Natural filename order (img2 before img10), wrapping around.
    Sequential,
    /// Random pick every time; never the same image twice in a row.
    Shuffle,
    /// Shuffle-bag: every image is shown once before any repeats.
    NoRepeat,
}

#[allow(dead_code)] // label/key/ALL/parse are exercised by tests and kept for tray/CLI use
impl Mode {
    pub const ALL: [Mode; 3] = [Mode::Sequential, Mode::Shuffle, Mode::NoRepeat];

    pub fn label(self) -> &'static str {
        match self {
            Mode::Sequential => "Sequential",
            Mode::Shuffle => "Shuffle",
            Mode::NoRepeat => "No-repeat shuffle",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Mode::Sequential => "sequential",
            Mode::Shuffle => "shuffle",
            Mode::NoRepeat => "norepeat",
        }
    }

    pub fn parse(s: &str) -> Option<Mode> {
        let k: String = s
            .to_ascii_lowercase()
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect();
        match k.as_str() {
            "sequential" | "seq" | "ordered" | "order" => Some(Mode::Sequential),
            "shuffle" | "random" => Some(Mode::Shuffle),
            "norepeat" | "shufflenorepeat" | "bag" => Some(Mode::NoRepeat),
            _ => None,
        }
    }
}

/// xorshift64* — tiny, fast, good enough for picking wallpapers.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        // splitmix64 scramble so similar seeds give unrelated streams
        let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        Rng(if z == 0 { 0x2545_F491_4F6C_DD1D } else { z })
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform in `0..n` (n > 0).
    pub fn below(&mut self, n: usize) -> usize {
        (((self.next_u64() >> 11) as u128 * n as u128) >> 53) as usize
    }
}

fn shuffle<T>(v: &mut [T], rng: &mut Rng) {
    for i in (1..v.len()).rev() {
        let j = rng.below(i + 1);
        v.swap(i, j);
    }
}

/// Case-insensitive compare where digit runs compare numerically.
fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let (mut ai, mut bi) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (ai.peek().copied(), bi.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let mut na = String::new();
                while let Some(c) = ai.peek().copied().filter(char::is_ascii_digit) {
                    na.push(c);
                    ai.next();
                }
                let mut nb = String::new();
                while let Some(c) = bi.peek().copied().filter(char::is_ascii_digit) {
                    nb.push(c);
                    bi.next();
                }
                let (ta, tb) = (na.trim_start_matches('0'), nb.trim_start_matches('0'));
                let ord = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            (Some(x), Some(y)) => {
                if x != y {
                    return x.cmp(&y);
                }
                ai.next();
                bi.next();
            }
        }
    }
}

pub fn sort_natural(files: &mut Vec<PathBuf>) {
    let mut keyed: Vec<(String, PathBuf)> = files
        .drain(..)
        .map(|p| (p.to_string_lossy().to_lowercase(), p))
        .collect();
    keyed.sort_by(|a, b| natural_cmp(&a.0, &b.0).then_with(|| a.1.cmp(&b.1)));
    files.extend(keyed.into_iter().map(|(_, p)| p));
}

pub struct Playlist {
    files: Vec<PathBuf>,
    mode: Mode,
    rng: Rng,
    current: Option<PathBuf>,
    /// Indices not yet shown in the current no-repeat cycle (popped from the end).
    bag: Vec<usize>,
    /// Shown images for "previous" in the random modes.
    history: Vec<PathBuf>,
    back: usize,
    /// Set when a cycle ended — the caller should re-scan the folder.
    wrapped: bool,
    picks_since_wrap: usize,
}

impl Playlist {
    pub fn new(mode: Mode, seed: u64) -> Self {
        Self {
            files: Vec::new(),
            mode,
            rng: Rng::new(seed),
            current: None,
            bag: Vec::new(),
            history: Vec::new(),
            back: 0,
            wrapped: false,
            picks_since_wrap: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn files(&self) -> &[PathBuf] {
        &self.files
    }

    pub fn current(&self) -> Option<&Path> {
        self.current.as_deref()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// True once per finished cycle; the worker uses it to pick up new files.
    pub fn take_wrapped(&mut self) -> bool {
        std::mem::take(&mut self.wrapped)
    }

    pub fn set_mode(&mut self, mode: Mode) {
        if mode == self.mode {
            return;
        }
        self.mode = mode;
        self.bag.clear();
        self.history.clear();
        self.back = 0;
        self.picks_since_wrap = 0;
        self.seed_history();
    }

    fn seed_history(&mut self) {
        if self.mode != Mode::Sequential {
            if let Some(c) = &self.current {
                self.history.push(c.clone());
            }
        }
    }

    /// Replace everything (new folder).
    pub fn reset(&mut self, mut files: Vec<PathBuf>) {
        sort_natural(&mut files);
        self.files = files;
        self.current = None;
        self.bag.clear();
        self.history.clear();
        self.back = 0;
        self.wrapped = false;
        self.picks_since_wrap = 0;
    }

    /// Re-scan of the same folder: keep position, history and the unseen part of
    /// the no-repeat bag; brand-new files are added to the bag.
    pub fn refresh(&mut self, mut files: Vec<PathBuf>) {
        sort_natural(&mut files);
        let old_files: HashSet<&PathBuf> = self.files.iter().collect();
        let old_bag: HashSet<&PathBuf> =
            self.bag.iter().filter_map(|&i| self.files.get(i)).collect();
        let mut bag: Vec<usize> = files
            .iter()
            .enumerate()
            .filter(|(_, p)| old_bag.contains(p) || !old_files.contains(p))
            .map(|(i, _)| i)
            .collect();
        shuffle(&mut bag, &mut self.rng);
        self.bag = bag;
        self.files = files;
        self.wrapped = false;
        self.picks_since_wrap = 0;
    }

    /// Tell the playlist which wallpaper is on screen right now (Windows keeps
    /// it between runs), so Sequential continues from there.
    pub fn resume_after(&mut self, path: &Path) {
        let key = path.to_string_lossy().to_lowercase();
        let found = self
            .files
            .iter()
            .find(|f| f.to_string_lossy().to_lowercase() == key)
            .cloned();
        self.current = Some(found.unwrap_or_else(|| path.to_path_buf()));
        self.history.clear();
        self.back = 0;
        self.seed_history();
    }

    fn cursor(&self) -> Option<usize> {
        let cur = self.current.as_ref()?;
        self.files.iter().position(|f| f == cur)
    }

    fn pick_random(&mut self) -> usize {
        let n = self.files.len();
        if n == 1 {
            return 0;
        }
        let r = self.rng.below(n - 1);
        match self.cursor() {
            Some(c) if r >= c => r + 1,
            _ => r,
        }
    }

    fn refill_bag(&mut self) {
        let n = self.files.len();
        self.bag = (0..n).collect();
        shuffle(&mut self.bag, &mut self.rng);
        // avoid showing the same image twice across the cycle boundary
        if n > 1 && self.bag.last().copied() == self.cursor() {
            self.bag.swap(n - 1, 0);
        }
    }

    fn pick_bag(&mut self) -> usize {
        if self.bag.is_empty() {
            self.refill_bag();
            self.wrapped = true;
        }
        self.bag.pop().expect("bag refilled from non-empty list")
    }

    fn push_history(&mut self, i: usize) -> PathBuf {
        let p = self.files[i].clone();
        if self.history.len() >= HISTORY_MAX {
            self.history.remove(0);
        }
        self.history.push(p.clone());
        self.current = Some(p.clone());
        p
    }

    pub fn next(&mut self) -> Option<PathBuf> {
        let n = self.files.len();
        if n == 0 {
            return None;
        }
        match self.mode {
            Mode::Sequential => {
                let i = self.cursor().map_or(0, |c| (c + 1) % n);
                if i == 0 {
                    self.wrapped = true;
                }
                let p = self.files[i].clone();
                self.current = Some(p.clone());
                Some(p)
            }
            Mode::Shuffle | Mode::NoRepeat => {
                if self.back > 0 {
                    self.back -= 1;
                    let p = self.history[self.history.len() - 1 - self.back].clone();
                    self.current = Some(p.clone());
                    return Some(p);
                }
                let i = if self.mode == Mode::Shuffle {
                    self.picks_since_wrap += 1;
                    if self.picks_since_wrap >= n {
                        self.picks_since_wrap = 0;
                        self.wrapped = true;
                    }
                    self.pick_random()
                } else {
                    self.pick_bag()
                };
                Some(self.push_history(i))
            }
        }
    }

    pub fn prev(&mut self) -> Option<PathBuf> {
        let n = self.files.len();
        if n == 0 {
            return None;
        }
        match self.mode {
            Mode::Sequential => {
                let i = self.cursor().map_or(n - 1, |c| (c + n - 1) % n);
                let p = self.files[i].clone();
                self.current = Some(p.clone());
                Some(p)
            }
            Mode::Shuffle | Mode::NoRepeat => {
                if self.back + 1 < self.history.len() {
                    self.back += 1;
                    let p = self.history[self.history.len() - 1 - self.back].clone();
                    self.current = Some(p.clone());
                    Some(p)
                } else {
                    None
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(n: usize) -> Vec<PathBuf> {
        (1..=n)
            .map(|i| PathBuf::from(format!("w{i}.jpg")))
            .collect()
    }

    #[test]
    fn natural_order() {
        let mut v: Vec<PathBuf> = ["img10.jpg", "img2.jpg", "IMG1.jpg", "img02b.jpg"]
            .iter()
            .map(PathBuf::from)
            .collect();
        sort_natural(&mut v);
        let s: Vec<_> = v.iter().map(|p| p.to_str().unwrap()).collect();
        assert_eq!(s, ["IMG1.jpg", "img2.jpg", "img02b.jpg", "img10.jpg"]);
    }

    #[test]
    fn sequential_wraps_and_goes_back() {
        let mut p = Playlist::new(Mode::Sequential, 1);
        p.reset(files(3));
        let got: Vec<_> = (0..5).map(|_| p.next().unwrap()).collect();
        assert_eq!(got[0], PathBuf::from("w1.jpg"));
        assert_eq!(got[2], PathBuf::from("w3.jpg"));
        assert_eq!(got[3], PathBuf::from("w1.jpg"));
        assert!(p.take_wrapped());
        assert_eq!(
            p.prev().unwrap(),
            PathBuf::from("w1.jpg")
                .clone()
                .with_file_name("w1.jpg")
                .clone()
        );
        assert_eq!(p.prev().unwrap(), PathBuf::from("w3.jpg"));
    }

    #[test]
    fn sequential_resumes_after_current() {
        let mut p = Playlist::new(Mode::Sequential, 1);
        p.reset(files(4));
        p.resume_after(Path::new("W2.JPG")); // case-insensitive like Windows
        assert_eq!(p.next().unwrap(), PathBuf::from("w3.jpg"));
    }

    #[test]
    fn no_repeat_shows_everything_each_cycle() {
        let mut p = Playlist::new(Mode::NoRepeat, 42);
        p.reset(files(7));
        let mut all = Vec::new();
        for _ in 0..7 * 6 {
            all.push(p.next().unwrap());
        }
        for cycle in all.chunks(7) {
            let set: HashSet<_> = cycle.iter().collect();
            assert_eq!(set.len(), 7, "a cycle repeated an image");
        }
        for w in all.windows(2) {
            assert_ne!(w[0], w[1], "same image twice in a row");
        }
    }

    #[test]
    fn shuffle_never_repeats_back_to_back() {
        let mut p = Playlist::new(Mode::Shuffle, 7);
        p.reset(files(3));
        let mut seen = HashSet::new();
        let mut last = None;
        for _ in 0..300 {
            let n = p.next().unwrap();
            assert_ne!(Some(&n), last.as_ref());
            seen.insert(n.clone());
            last = Some(n);
        }
        assert_eq!(seen.len(), 3);
    }

    #[test]
    fn single_file_is_fine_in_every_mode() {
        for m in Mode::ALL {
            let mut p = Playlist::new(m, 3);
            p.reset(files(1));
            for _ in 0..5 {
                assert_eq!(p.next().unwrap(), PathBuf::from("w1.jpg"));
            }
        }
    }

    #[test]
    fn previous_uses_history_in_random_modes() {
        let mut p = Playlist::new(Mode::NoRepeat, 9);
        p.reset(files(10));
        p.resume_after(Path::new("original.png"));
        let a = p.next().unwrap();
        let b = p.next().unwrap();
        assert_eq!(p.prev().unwrap(), a);
        assert_eq!(p.prev().unwrap(), PathBuf::from("original.png"));
        assert_eq!(p.prev(), None);
        assert_eq!(p.next().unwrap(), a); // forward replays history
        assert_eq!(p.next().unwrap(), b);
    }

    #[test]
    fn refresh_keeps_unseen_bag_and_adds_new_files() {
        let mut p = Playlist::new(Mode::NoRepeat, 5);
        p.reset(files(5));
        let played: HashSet<_> = (0..2).map(|_| p.next().unwrap()).collect();
        let mut more = files(5);
        more.push(PathBuf::from("w6.jpg"));
        p.refresh(more);
        let rest: Vec<_> = (0..4).map(|_| p.next().unwrap()).collect();
        let rest_set: HashSet<_> = rest.iter().cloned().collect();
        assert_eq!(rest_set.len(), 4);
        assert!(rest_set.is_disjoint(&played));
        assert!(rest_set.contains(&PathBuf::from("w6.jpg")));
    }

    #[test]
    fn mode_parse() {
        assert_eq!(Mode::parse("No-Repeat"), Some(Mode::NoRepeat));
        assert_eq!(Mode::parse("random"), Some(Mode::Shuffle));
        assert_eq!(Mode::parse("Sequential"), Some(Mode::Sequential));
        assert_eq!(Mode::parse("nope"), None);
    }
}
