//! Persistent on-disk binary cache for pre-rendered squircle-framed icon bitmaps.
//!
//! Stores premultiplied RGBA8 pixel buffers keyed by deterministic hash of
//! (source path, mtime, file length, target size, plate options).
//! Loading cached icons skips all SVG parsing (resvg), image decompression,
//! Lanczos resizing, and squircle background matting calculations.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;
use tiny_skia::Pixmap;

const MAGIC: &[u8; 8] = b"BMOLICN1";
const HEADER_LEN: usize = 16;

/// Deterministic 64-bit FNV-1a hash algorithm.
pub fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// Computes the disk cache directory: `~/.cache/bmol/icons`.
pub fn disk_cache_dir() -> PathBuf {
    if let Some(xdg) = std::env::var_os("XDG_CACHE_HOME") {
        PathBuf::from(xdg).join("bmol/icons")
    } else if let Some(home) = std::env::var_os("HOME") {
        PathBuf::from(home).join(".cache/bmol/icons")
    } else {
        std::env::temp_dir().join("bmol/icons")
    }
}

/// Computes the 64-bit cache key hash for an icon request.
pub fn compute_cache_key(
    resolved_path: &Path,
    size: u32,
    strategy: u8,
    theme: u8,
) -> Option<(u64, PathBuf)> {
    let metadata = fs::metadata(resolved_path).ok()?;
    let (mtime_sec, mtime_nsec) = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| (d.as_secs(), d.subsec_nanos()))
        .unwrap_or((0, 0));
    let len = metadata.len();

    let mut buf = Vec::with_capacity(256);
    buf.extend_from_slice(resolved_path.to_string_lossy().as_bytes());
    buf.extend_from_slice(&mtime_sec.to_le_bytes());
    buf.extend_from_slice(&mtime_nsec.to_le_bytes());
    buf.extend_from_slice(&len.to_le_bytes());
    buf.extend_from_slice(&size.to_le_bytes());
    buf.push(strategy);
    buf.push(theme);

    let hash = fnv1a_64(&buf);
    let filename = format!("{:016x}_{size}.bin", hash);
    Some((hash, disk_cache_dir().join(filename)))
}

/// Attempts to load an icon from persistent disk cache.
pub fn load_from_disk(cache_path: &Path, expected_size: u32) -> Option<Pixmap> {
    let data = fs::read(cache_path).ok()?;
    if data.len() < HEADER_LEN || &data[0..8] != MAGIC {
        return None;
    }
    let w = u32::from_le_bytes(data[8..12].try_into().ok()?);
    let h = u32::from_le_bytes(data[12..16].try_into().ok()?);
    if w != expected_size || h != expected_size {
        return None;
    }
    let expected_payload = (w as usize).checked_mul(h as usize)?.checked_mul(4)?;
    if data.len() != HEADER_LEN + expected_payload {
        return None;
    }
    let mut pixmap = Pixmap::new(w, h)?;
    pixmap.data_mut().copy_from_slice(&data[HEADER_LEN..]);
    Some(pixmap)
}

/// Writes an icon pixmap to persistent disk cache.
pub fn save_to_disk(cache_path: &Path, pixmap: &Pixmap) {
    let (w, h) = (pixmap.width(), pixmap.height());
    let payload = pixmap.data();
    let mut buf = Vec::with_capacity(HEADER_LEN + payload.len());
    buf.extend_from_slice(MAGIC);
    buf.extend_from_slice(&w.to_le_bytes());
    buf.extend_from_slice(&h.to_le_bytes());
    buf.extend_from_slice(payload);

    if let Some(parent) = cache_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let tmp_path = cache_path.with_extension("tmp");
    if fs::write(&tmp_path, &buf).is_ok() {
        let _ = fs::rename(&tmp_path, cache_path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fnv1a_determinism() {
        let h1 = fnv1a_64(b"test-icon-path-1234");
        let h2 = fnv1a_64(b"test-icon-path-1234");
        assert_eq!(h1, h2);
        assert_ne!(h1, fnv1a_64(b"test-icon-path-5678"));
    }

    #[test]
    fn test_disk_cache_roundtrip() {
        let tmp_dir = std::env::temp_dir().join("bmol_test_icon_cache");
        let cache_file = tmp_dir.join("test_roundtrip.bin");
        let mut pixmap = Pixmap::new(32, 32).unwrap();
        pixmap.fill(tiny_skia::Color::from_rgba8(255, 128, 64, 255));

        save_to_disk(&cache_file, &pixmap);
        let loaded = load_from_disk(&cache_file, 32).expect("must load from disk");
        assert_eq!(loaded.width(), 32);
        assert_eq!(loaded.height(), 32);
        assert_eq!(loaded.data(), pixmap.data());

        // Incorrect expected size should be rejected
        assert!(load_from_disk(&cache_file, 64).is_none());

        let _ = fs::remove_file(cache_file);
        let _ = fs::remove_dir(tmp_dir);
    }
}
