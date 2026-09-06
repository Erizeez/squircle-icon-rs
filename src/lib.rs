//! squircle-icon-rs: Apple macOS HIG-compliant application icon processing & rendering engine.
//!
//! Provides:
//! - Freedesktop XDG icon discovery with multi-theme and scalable SVG fallback.
//! - Multi-format vector (SVG) and raster (PNG, JPEG, WebP) decoding.
//! - Continuous curvature ($G^2$) squircle corner clipping (`squircle-rs`).
//! - Intelligent matting: automated cutout/circle detection and subtle Apple-style background plate generation.
//! - High-performance raster output, PNG encoding, data URIs, and SVG-wrapped embeds.

pub mod lookup;
pub mod plate;
pub mod raster;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

pub use lookup::{icon_search_roots, resolve_icon};
pub use plate::{apply_squircle_plate, is_cutout_icon, MattingStrategy, PlateOptions, PlateTheme};
pub use raster::{rasterize_file, rasterize_image_data, rasterize_svg_data, RasterError};
pub use tiny_skia::Pixmap;

/// Processed application icon bitmap in RGBA8 format.
#[derive(Clone)]
pub struct IconBitmap {
    width: u32,
    height: u32,
    pixmap: Pixmap,
}

impl IconBitmap {
    /// Creates an `IconBitmap` from an existing `tiny_skia::Pixmap`.
    pub fn from_pixmap(pixmap: Pixmap) -> Self {
        let width = pixmap.width();
        let height = pixmap.height();
        Self { width, height, pixmap }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn pixmap(&self) -> &Pixmap {
        &self.pixmap
    }

    pub fn into_pixmap(self) -> Pixmap {
        self.pixmap
    }

    /// Returns premultiplied RGBA8 pixel slice (native `tiny_skia` layout).
    pub fn as_premultiplied_rgba(&self) -> &[u8] {
        self.pixmap.data()
    }

    /// Converts premultiplied RGBA8 pixels into straight (unpremultiplied) RGBA8 bytes.
    pub fn to_straight_rgba(&self) -> Vec<u8> {
        let pixels = self.pixmap.pixels();
        let mut out = Vec::with_capacity(pixels.len() * 4);
        for p in pixels {
            let a = p.alpha();
            if a == 0 {
                out.extend_from_slice(&[0, 0, 0, 0]);
            } else if a == 255 {
                out.extend_from_slice(&[p.red(), p.green(), p.blue(), 255]);
            } else {
                let af = a as f32 / 255.0;
                let r = ((p.red() as f32 / af).min(255.0)).round() as u8;
                let g = ((p.green() as f32 / af).min(255.0)).round() as u8;
                let b = ((p.blue() as f32 / af).min(255.0)).round() as u8;
                out.extend_from_slice(&[r, g, b, a]);
            }
        }
        out
    }

    /// Encodes the icon bitmap as standard PNG bytes.
    pub fn to_png_bytes(&self) -> Result<Vec<u8>, png::EncodingError> {
        self.pixmap.encode_png()
    }

    /// Returns a base64-encoded `data:image/png;base64,...` URI string.
    pub fn to_data_uri(&self) -> Option<String> {
        let png = self.to_png_bytes().ok()?;
        Some(format!("data:image/png;base64,{}", base64_encode(&png)))
    }

    /// Wraps the processed squircle icon in a standalone SVG document.
    ///
    /// This enables seamless vector integration in GUI engines (e.g. `iced::widget::svg`)
    /// without requiring a separate raster image pipeline.
    pub fn to_svg_markup(&self) -> Option<String> {
        let uri = self.to_data_uri()?;
        let w = self.width;
        let h = self.height;
        Some(format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w} {h}"><image href="{uri}" x="0" y="0" width="{w}" height="{h}"/></svg>"#
        ))
    }
}

/// Global icon rendering and cache engine.
pub struct AppIconEngine {
    cache: Mutex<HashMap<CacheKey, CacheEntry>>,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct CacheKey {
    identifier: String,
    size: u32,
    strategy: u8,
    theme: u8,
}

#[derive(Clone)]
struct CacheEntry {
    bitmap: IconBitmap,
    svg_markup: Option<String>,
}

static ENGINE: OnceLock<AppIconEngine> = OnceLock::new();

impl AppIconEngine {
    pub fn global() -> &'static Self {
        ENGINE.get_or_init(|| Self {
            cache: Mutex::new(HashMap::new()),
        })
    }

    /// Loads, rasters, and applies squircle framing to an icon file or app-id.
    pub fn render(
        &self,
        name_or_path: &str,
        size: u32,
        options: PlateOptions,
    ) -> Option<IconBitmap> {
        let key = CacheKey {
            identifier: name_or_path.to_string(),
            size,
            strategy: options.strategy as u8,
            theme: options.theme as u8,
        };

        if let Ok(guard) = self.cache.lock()
            && let Some(entry) = guard.get(&key)
        {
            return Some(entry.bitmap.clone());
        }

        let path = if Path::new(name_or_path).is_file() {
            PathBuf::from(name_or_path)
        } else {
            resolve_icon(name_or_path)?
        };

        let raw_pixmap = rasterize_file(&path, size, size).ok()?;
        let framed = apply_squircle_plate(&raw_pixmap, options);
        let bitmap = IconBitmap::from_pixmap(framed);
        let svg_markup = bitmap.to_svg_markup();

        let entry = CacheEntry {
            bitmap: bitmap.clone(),
            svg_markup,
        };

        if let Ok(mut guard) = self.cache.lock() {
            guard.insert(key, entry);
        }

        Some(bitmap)
    }

    /// Returns the cached SVG markup embedding the squircle-framed icon.
    pub fn render_svg_markup(
        &self,
        name_or_path: &str,
        size: u32,
        options: PlateOptions,
    ) -> Option<String> {
        let key = CacheKey {
            identifier: name_or_path.to_string(),
            size,
            strategy: options.strategy as u8,
            theme: options.theme as u8,
        };

        if let Ok(guard) = self.cache.lock()
            && let Some(entry) = guard.get(&key)
        {
            return entry.svg_markup.clone();
        }

        let bitmap = self.render(name_or_path, size, options)?;
        if let Ok(guard) = self.cache.lock()
            && let Some(entry) = guard.get(&key)
        {
            return entry.svg_markup.clone();
        }
        bitmap.to_svg_markup()
    }
}

/// Convenience function to render and squircle-frame an icon to an `IconBitmap`.
pub fn render_icon(name_or_path: &str, size: u32, options: PlateOptions) -> Option<IconBitmap> {
    AppIconEngine::global().render(name_or_path, size, options)
}

/// Convenience function to render and squircle-frame an icon to an SVG markup string.
pub fn render_svg_markup(name_or_path: &str, size: u32, options: PlateOptions) -> Option<String> {
    AppIconEngine::global().render_svg_markup(name_or_path, size, options)
}

fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied().unwrap_or(0);
        let third = chunk.get(2).copied().unwrap_or(0);
        encoded.push(TABLE[(first >> 2) as usize] as char);
        encoded.push(TABLE[((first & 0x03) << 4 | (second >> 4)) as usize] as char);
        encoded.push(if chunk.len() > 1 {
            TABLE[((second & 0x0f) << 2 | (third >> 6)) as usize] as char
        } else {
            '='
        });
        encoded.push(if chunk.len() > 2 {
            TABLE[(third & 0x3f) as usize] as char
        } else {
            '='
        });
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_icon_bitmap_to_png_and_svg() {
        let mut pixmap = Pixmap::new(64, 64).unwrap();
        pixmap.fill(tiny_skia::Color::from_rgba8(255, 0, 0, 255));
        let bitmap = IconBitmap::from_pixmap(pixmap);
        let png_bytes = bitmap.to_png_bytes().expect("encode png");
        assert!(png_bytes.starts_with(b"\x89PNG\r\n\x1a\n"));

        let svg = bitmap.to_svg_markup().expect("generate svg markup");
        assert!(svg.contains("<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 64 64\">"));
        assert!(svg.contains("<image href=\"data:image/png;base64,"));
    }

    #[test]
    fn test_render_svg_with_squircle_plate() {
        let svg = r#"<svg viewBox="0 0 100 100"><circle cx="50" cy="50" r="40" fill="blue"/></svg>"#;
        let pixmap = rasterize_svg_data(svg.as_bytes(), 128, 128).unwrap();
        // Circle has transparent corners -> auto matting should create an Apple squircle background plate
        let framed = apply_squircle_plate(&pixmap, PlateOptions::default());
        assert_eq!(framed.width(), 128);
        assert_eq!(framed.height(), 128);
        // Center has glyph
        let center = framed.pixel(64, 64).unwrap();
        assert_eq!(center.blue(), 255);
        // Plate background near edge (e.g. x=20, y=20) should have high alpha (the plate)
        let plate_pixel = framed.pixel(20, 20).unwrap();
        assert!(plate_pixel.alpha() > 200, "Apple squircle plate should fill corner areas");
    }

    #[test]
    fn test_render_svg_markup_caching() {
        let moonlight_path = "/usr/share/icons/hicolor/scalable/apps/moonlight.svg";
        if Path::new(moonlight_path).is_file() {
            let markup = render_svg_markup(moonlight_path, 64, PlateOptions::default());
            assert!(markup.is_some());
            let s = markup.unwrap();
            assert!(s.starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\""));
            assert!(s.contains("data:image/png;base64,"));
        }
    }
}
