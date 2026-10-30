//! Folder browsing and file metadata.

use std::path::{Path, PathBuf};

/// Extensions we step through when browsing a folder. ImageIO can decode
/// more than this, but these are the ones worth stopping on.
const IMAGE_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "jpe", "jfif", "png", "gif", "tif", "tiff", "bmp", "heic", "heif", "webp",
    "psd", "tga", "ico", "icns", "pdf", "avif", "jp2", "exr", "hdr", "pbm", "pgm", "ppm", "pnm",
    "cr2", "cr3", "nef", "arw", "dng", "raf", "orf", "rw2", "pef", "srw",
];

pub fn is_image(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            let e = e.to_ascii_lowercase();
            IMAGE_EXTENSIONS.contains(&e.as_str())
        })
        .unwrap_or(false)
}

/// All image files in the folder containing `path`, sorted case-insensitively
/// by file name (Finder-like, but stable), plus the index of `path` itself.
pub fn siblings(path: &Path) -> (Vec<PathBuf>, usize) {
    let dir = match path.parent() {
        Some(d) => d,
        None => return (vec![path.to_path_buf()], 0),
    };
    let mut list: Vec<PathBuf> = match std::fs::read_dir(dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.is_file() && is_image(p))
            .filter(|p| {
                !p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with('.'))
                    .unwrap_or(true)
            })
            .collect(),
        Err(_) => vec![],
    };
    list.sort_by_cached_key(|p| {
        p.file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.to_lowercase())
            .unwrap_or_default()
    });
    let idx = list.iter().position(|p| p == path).unwrap_or_else(|| {
        list.insert(0, path.to_path_buf());
        0
    });
    (list, idx)
}

/// First image file in a folder, by the same ordering as `siblings`.
pub fn first_image_in(dir: &Path) -> Option<PathBuf> {
    let mut list: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file() && is_image(p))
        .collect();
    list.sort_by_cached_key(|p| file_name(p).to_lowercase());
    list.into_iter().next()
}

pub fn file_size(path: &Path) -> Option<u64> {
    std::fs::metadata(path).ok().map(|m| m.len())
}

pub fn human_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "kB", "MB", "GB"];
    let mut v = bytes as f64;
    let mut u = 0;
    while v >= 1000.0 && u < UNITS.len() - 1 {
        v /= 1000.0;
        u += 1;
    }
    if u == 0 {
        format!("{bytes} B")
    } else {
        format!("{v:.1} {} ({bytes} bytes)", UNITS[u])
    }
}

pub fn file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("?")
        .to_string()
}

pub fn extension_lower(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default()
}

pub fn modified_string(path: &Path) -> String {
    use std::time::UNIX_EPOCH;
    let Ok(md) = std::fs::metadata(path) else {
        return String::from("?");
    };
    let Ok(t) = md.modified() else {
        return String::from("?");
    };
    let secs = t.duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) as i64;
    // Local time without pulling in chrono: use libc's localtime_r.
    unsafe {
        let mut tm: libc_tm = std::mem::zeroed();
        let time: i64 = secs;
        localtime_r(&time, &mut tm);
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            tm.tm_year + 1900,
            tm.tm_mon + 1,
            tm.tm_mday,
            tm.tm_hour,
            tm.tm_min,
            tm.tm_sec
        )
    }
}

// Minimal libc declarations so we don't need the `libc` crate just for this.
#[repr(C)]
#[allow(non_camel_case_types)]
struct libc_tm {
    tm_sec: i32,
    tm_min: i32,
    tm_hour: i32,
    tm_mday: i32,
    tm_mon: i32,
    tm_year: i32,
    tm_wday: i32,
    tm_yday: i32,
    tm_isdst: i32,
    tm_gmtoff: i64,
    tm_zone: *const i8,
}
extern "C" {
    fn localtime_r(time: *const i64, result: *mut libc_tm) -> *mut libc_tm;
}
