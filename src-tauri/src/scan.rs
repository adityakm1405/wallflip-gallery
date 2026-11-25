//! Folder scanner. Uses only `read_dir` + `DirEntry::file_type` (no per-file
//! stat calls), never follows symlinks/junctions (no loops), skips dot-files.

use std::fs;
use std::path::{Path, PathBuf};

const MAX_DEPTH: usize = 8;

fn wanted(path: &Path, exts: &[String]) -> bool {
    let name_ok = path
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| !n.starts_with('.'));
    if !name_ok {
        return false;
    }
    match path.extension().and_then(|e| e.to_str()) {
        Some(e) => {
            let e = e.to_ascii_lowercase();
            exts.contains(&e)
        }
        None => false,
    }
}

fn walk(dir: &Path, recursive: bool, exts: &[String], depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for entry in rd.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_file() {
            if wanted(&path, exts) {
                out.push(path);
            }
        } else if ft.is_dir() && recursive && depth < MAX_DEPTH {
            walk(&path, recursive, exts, depth + 1, out);
        }
    }
}

/// Returns all matching image files (unsorted; `Playlist::reset` sorts them).
pub fn scan(dir: &Path, recursive: bool, exts: &[String]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk(dir, recursive, exts, 0, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DEFAULT_EXTENSIONS;

    fn exts() -> Vec<String> {
        DEFAULT_EXTENSIONS.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn finds_images_filters_others_and_honours_recursion() {
        let root = std::env::temp_dir().join(format!("wallflip-scan-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("sub/deeper")).unwrap();
        for f in [
            "a.JPG",
            "b.png",
            "notes.txt",
            ".hidden.png",
            "noext",
            "sub/c.webp",
            "sub/deeper/d.bmp",
        ] {
            fs::write(root.join(f), b"x").unwrap();
        }
        let flat = scan(&root, false, &exts());
        assert_eq!(flat.len(), 2, "{flat:?}");
        let deep = scan(&root, true, &exts());
        assert_eq!(deep.len(), 4, "{deep:?}");
        assert!(scan(&root.join("missing"), true, &exts()).is_empty());
        let _ = fs::remove_dir_all(&root);
    }
}
