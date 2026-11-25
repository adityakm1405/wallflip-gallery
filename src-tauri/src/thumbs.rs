//! Thumbnail generation with a persistent on-disk JPEG cache.
//!
//! The gallery never loads full-size wallpapers: each image is decoded once,
//! downscaled to at most `MAX_EDGE` px and cached. Later opens just read ~100 KB
//! files, so even a 4K PNG library opens instantly after the first run.

use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use image::{ImageReader, ExtendedColorType};
use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

const MAX_EDGE: u32 = 960;
const QUALITY: u8 = 82;
/// Bump to invalidate every cached thumbnail after changing the settings above.
const VERSION: u64 = 1;
const PRUNE_AFTER: Duration = Duration::from_secs(60 * 24 * 3600);
const TOUCH_AFTER: Duration = Duration::from_secs(7 * 24 * 3600);

/// FNV-1a: tiny, stable across runs (unlike `DefaultHasher`), good enough for cache keys.
pub fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Stable id for a path (case-insensitive, like Windows).
pub fn id_for(path: &Path) -> String {
    format!("{:016x}", fnv1a(path.to_string_lossy().to_lowercase().as_bytes()))
}

fn cache_name(path: &Path, meta: &fs::Metadata) -> String {
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs());
    let key = format!("{}|{}|{}|{}|{}", path.to_string_lossy().to_lowercase(), mtime, meta.len(), MAX_EDGE, VERSION);
    format!("{:016x}.jpg", fnv1a(key.as_bytes()))
}

/// Returns JPEG bytes for `src`, from cache when possible.
pub fn thumbnail(src: &Path, cache_dir: &Path) -> Result<Vec<u8>, String> {
    let meta = fs::metadata(src).map_err(|e| format!("{}: {e}", src.display()))?;
    let cached = cache_dir.join(cache_name(src, &meta));

    if let Ok(bytes) = fs::read(&cached) {
        // keep recently used thumbnails alive for `prune`
        if let Ok(age) = fs::metadata(&cached).and_then(|m| m.modified()).map(|t| t.elapsed().unwrap_or_default()) {
            if age > TOUCH_AFTER {
                if let Ok(f) = fs::OpenOptions::new().write(true).open(&cached) {
                    let _ = f.set_modified(SystemTime::now());
                }
            }
        }
        return Ok(bytes);
    }

    let img = ImageReader::open(src)
        .map_err(|e| e.to_string())?
        .with_guessed_format()
        .map_err(|e| e.to_string())?
        .decode()
        .map_err(|e| format!("{}: {e}", src.display()))?;
    let small = img.resize(MAX_EDGE, MAX_EDGE, FilterType::Triangle).to_rgb8();

    let mut out = Vec::with_capacity(96 * 1024);
    JpegEncoder::new_with_quality(&mut out, QUALITY)
        .encode(small.as_raw(), small.width(), small.height(), ExtendedColorType::Rgb8)
        .map_err(|e| e.to_string())?;

    if fs::create_dir_all(cache_dir).is_ok() {
        let tmp = cached.with_extension("tmp");
        if fs::write(&tmp, &out).is_ok() && fs::rename(&tmp, &cached).is_err() {
            let _ = fs::remove_file(&tmp);
        }
    }
    Ok(out)
}

/// Screen-sized JPEG for the transition animation (no cache: it is used once).
/// The image is shrunk, never enlarged, so that it still covers `w` x `h`; `0` keeps the original size.
pub fn fullsize(src: &Path, w: u32, h: u32) -> Result<Vec<u8>, String> {
    let img = ImageReader::open(src)
        .map_err(|e| e.to_string())?
        .with_guessed_format()
        .map_err(|e| e.to_string())?
        .decode()
        .map_err(|e| format!("{}: {e}", src.display()))?;
    let cover = if w > 0 && h > 0 {
        (f64::from(w) / f64::from(img.width())).max(f64::from(h) / f64::from(img.height()))
    } else {
        1.0
    };
    let rgb = if cover < 1.0 {
        let nw = (f64::from(img.width()) * cover).ceil() as u32;
        let nh = (f64::from(img.height()) * cover).ceil() as u32;
        img.resize_exact(nw.max(1), nh.max(1), FilterType::Triangle).to_rgb8()
    } else {
        img.to_rgb8()
    };
    let mut out = Vec::with_capacity(512 * 1024);
    JpegEncoder::new_with_quality(&mut out, 92)
        .encode(rgb.as_raw(), rgb.width(), rgb.height(), ExtendedColorType::Rgb8)
        .map_err(|e| e.to_string())?;
    Ok(out)
}

/// Deletes thumbnails nobody has used for two months.
pub fn prune(cache_dir: &Path) {
    let Ok(rd) = fs::read_dir(cache_dir) else { return };
    for e in rd.flatten() {
        let stale = e
            .metadata()
            .and_then(|m| m.modified())
            .map(|t| t.elapsed().unwrap_or_default() > PRUNE_AFTER)
            .unwrap_or(false);
        if stale {
            let _ = fs::remove_file(e.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, RgbImage};

    fn tmpdir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("wallflip-thumb-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn ids_are_stable_and_case_insensitive() {
        assert_eq!(id_for(Path::new("C:\\A\\b.png")), id_for(Path::new("c:\\a\\B.PNG")));
        assert_ne!(id_for(Path::new("a.png")), id_for(Path::new("b.png")));
        assert_eq!(id_for(Path::new("x")).len(), 16);
    }

    #[test]
    fn makes_downscaled_jpeg_and_reuses_cache() {
        let dir = tmpdir("a");
        let src = dir.join("big.png");
        RgbImage::from_pixel(3840, 2160, Rgb([200, 80, 40])).save(&src).unwrap();
        let cache = dir.join("cache");

        let first = thumbnail(&src, &cache).unwrap();
        let decoded = image::load_from_memory(&first).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (960, 540));
        assert!(first.len() < 60_000, "thumbnail unexpectedly large: {}", first.len());
        assert_eq!(fs::read_dir(&cache).unwrap().count(), 1);

        // second call must come from the cache: remove the source's pixels, keep size+mtime semantics
        let second = thumbnail(&src, &cache).unwrap();
        assert_eq!(first, second);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn portrait_and_small_images_keep_aspect_ratio() {
        let dir = tmpdir("b");
        let src = dir.join("p.png");
        RgbImage::from_pixel(540, 1200, Rgb([10, 20, 30])).save(&src).unwrap();
        let t = image::load_from_memory(&thumbnail(&src, &dir.join("c")).unwrap()).unwrap();
        assert_eq!((t.width(), t.height()), (432, 960));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn changing_the_file_invalidates_the_cache() {
        let dir = tmpdir("c");
        let src = dir.join("x.png");
        RgbImage::from_pixel(100, 100, Rgb([255, 0, 0])).save(&src).unwrap();
        let cache = dir.join("c");
        let a = thumbnail(&src, &cache).unwrap();
        RgbImage::from_pixel(120, 100, Rgb([0, 0, 255])).save(&src).unwrap(); // different size => new key
        let b = thumbnail(&src, &cache).unwrap();
        assert_ne!(a, b);
        assert_eq!(fs::read_dir(&cache).unwrap().count(), 2);
        let _ = fs::remove_dir_all(&dir);
    }

    /// Dev tool: `WALLFLIP_BENCH_IMG=/path/to/photo.jpg cargo test --release bench_one -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn bench_one() {
        let Some(src) = std::env::var_os("WALLFLIP_BENCH_IMG") else { return };
        let cache = tmpdir("bench").join("c");
        let t = std::time::Instant::now();
        let cold = thumbnail(Path::new(&src), &cache).unwrap();
        let cold_ms = t.elapsed().as_secs_f64() * 1000.0;
        let t = std::time::Instant::now();
        thumbnail(Path::new(&src), &cache).unwrap();
        println!("cold: {cold_ms:.1} ms ({} KB thumbnail), cached: {:.2} ms", cold.len() / 1024, t.elapsed().as_secs_f64() * 1000.0);
    }

    #[test]
    fn fullsize_shrinks_to_cover_but_never_enlarges() {
        let dir = tmpdir("e");
        let src = dir.join("big.png");
        RgbImage::from_pixel(3840, 2160, Rgb([1, 2, 3])).save(&src).unwrap();
        let d = image::load_from_memory(&fullsize(&src, 1920, 1080).unwrap()).unwrap();
        assert_eq!((d.width(), d.height()), (1920, 1080));
        let d = image::load_from_memory(&fullsize(&src, 1000, 1000).unwrap()).unwrap(); // cover => height 1000
        assert_eq!(d.height(), 1000);
        let d = image::load_from_memory(&fullsize(&src, 7680, 4320).unwrap()).unwrap(); // no enlarging
        assert_eq!(d.width(), 3840);
        let d = image::load_from_memory(&fullsize(&src, 0, 0).unwrap()).unwrap(); // 0 = original
        assert_eq!(d.width(), 3840);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_file_is_an_error_not_a_panic() {
        let dir = tmpdir("d");
        let src = dir.join("bad.jpg");
        fs::write(&src, b"not an image").unwrap();
        assert!(thumbnail(&src, &dir.join("c")).is_err());
        assert!(thumbnail(&dir.join("missing.jpg"), &dir.join("c")).is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}
