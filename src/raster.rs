use std::fs;
use std::path::Path;
use tiny_skia::{Pixmap, PremultipliedColorU8, Transform};
use resvg::usvg;

#[derive(Debug)]
pub enum RasterError {
    Io(std::io::Error),
    SvgParse(usvg::Error),
    ImageDecode(image::ImageError),
    InvalidSize,
    AllocationFailed,
}

impl From<std::io::Error> for RasterError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<usvg::Error> for RasterError {
    fn from(err: usvg::Error) -> Self {
        Self::SvgParse(err)
    }
}

impl From<image::ImageError> for RasterError {
    fn from(err: image::ImageError) -> Self {
        Self::ImageDecode(err)
    }
}

/// Rasterizes an SVG byte stream into a `tiny_skia::Pixmap` at target dimensions.
///
/// Preserves full, authentic RGB colors without arbitrary single-color tinting.
pub fn rasterize_svg_data(
    svg_data: &[u8],
    target_width: u32,
    target_height: u32,
) -> Result<Pixmap, RasterError> {
    if target_width == 0 || target_height == 0 {
        return Err(RasterError::InvalidSize);
    }
    let opt = usvg::Options::default();
    let tree = usvg::Tree::from_data(svg_data, &opt)?;
    let tree_size = tree.size();

    let sx = target_width as f32 / tree_size.width();
    let sy = target_height as f32 / tree_size.height();
    let scale = sx.min(sy);

    let dx = (target_width as f32 - tree_size.width() * scale) / 2.0;
    let dy = (target_height as f32 - tree_size.height() * scale) / 2.0;

    let transform = Transform::from_translate(dx, dy).post_scale(scale, scale);
    let mut pixmap = Pixmap::new(target_width, target_height).ok_or(RasterError::AllocationFailed)?;
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    Ok(pixmap)
}

/// Decodes and rescales a raster image (PNG, JPEG, WebP) into a `tiny_skia::Pixmap`.
pub fn rasterize_image_data(
    image_data: &[u8],
    target_width: u32,
    target_height: u32,
) -> Result<Pixmap, RasterError> {
    if target_width == 0 || target_height == 0 {
        return Err(RasterError::InvalidSize);
    }
    let mut rgba_img = image::load_from_memory(image_data)?.into_rgba8();
    // Pre-clean dirty transparent pixels (alpha <= 8) before Lanczos filtering
    // to prevent dirty matte RGB from bleeding into adjacent visible pixels.
    for pixel in rgba_img.pixels_mut() {
        if pixel[3] <= 8 {
            pixel[0] = 0;
            pixel[1] = 0;
            pixel[2] = 0;
            pixel[3] = 0;
        }
    }
    let resized = image::imageops::resize(
        &rgba_img,
        target_width,
        target_height,
        image::imageops::FilterType::Lanczos3,
    );
    let (w, h) = resized.dimensions();

    let mut pixmap = Pixmap::new(target_width, target_height).ok_or(RasterError::AllocationFailed)?;
    let offset_x = ((target_width - w) / 2) as usize;
    let offset_y = ((target_height - h) / 2) as usize;

    let pixels = pixmap.pixels_mut();
    for y in 0..h as usize {
        for x in 0..w as usize {
            let pixel = resized.get_pixel(x as u32, y as u32);
            let a = if pixel[3] <= 8 { 0 } else { pixel[3] };
            if a > 0
                && let Some(color) = PremultipliedColorU8::from_rgba(pixel[0], pixel[1], pixel[2], a)
            {
                let target_idx = (offset_y + y) * target_width as usize + (offset_x + x);
                if target_idx < pixels.len() {
                    pixels[target_idx] = color;
                }
            }
        }
    }
    Ok(pixmap)
}

/// Reads and rasterizes an icon file (.svg, .png, .webp, .jpg) to target dimensions.
pub fn rasterize_file(
    path: &Path,
    target_width: u32,
    target_height: u32,
) -> Result<Pixmap, RasterError> {
    let data = fs::read(path)?;
    let is_svg = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|s| s.eq_ignore_ascii_case("svg"))
        .unwrap_or(false);

    if is_svg {
        rasterize_svg_data(&data, target_width, target_height)
    } else {
        rasterize_image_data(&data, target_width, target_height)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rasterize_sample_svg() {
        let svg = r#"<svg viewBox="0 0 100 100"><circle cx="50" cy="50" r="50" fill="red"/></svg>"#;
        let pixmap = rasterize_svg_data(svg.as_bytes(), 64, 64).expect("rasterize svg");
        assert_eq!(pixmap.width(), 64);
        assert_eq!(pixmap.height(), 64);
        let pixel = pixmap.pixel(32, 32).expect("center pixel");
        assert_eq!(pixel.red(), 255);
        assert_eq!(pixel.green(), 0);
        assert_eq!(pixel.blue(), 0);
        assert_eq!(pixel.alpha(), 255);
    }

    #[test]
    fn test_rasterize_moonlight_preserves_white_and_slate() {
        let moonlight_svg = r#"<svg viewBox="0 0 256 256">
            <circle cx="128" cy="128" r="128" fill="rgb(86,92,100)"/>
            <circle cx="128" cy="128" r="96" fill="rgb(255,255,255)"/>
        </svg>"#;
        let pixmap = rasterize_svg_data(moonlight_svg.as_bytes(), 128, 128).expect("rasterize moonlight");
        let inner_pixel = pixmap.pixel(64, 64).expect("inner pixel");
        // Must NOT be black! Must be pure white as designed!
        assert_eq!(inner_pixel.red(), 255);
        assert_eq!(inner_pixel.green(), 255);
        assert_eq!(inner_pixel.blue(), 255);

        // Edge circle is slate gray rgb(86, 92, 100)
        let edge_pixel = pixmap.pixel(64, 8).expect("edge pixel");
        assert_eq!(edge_pixel.red(), 86);
        assert_eq!(edge_pixel.green(), 92);
        assert_eq!(edge_pixel.blue(), 100);
    }
}
