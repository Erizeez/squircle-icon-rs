use squircle_rs::{
    squircle_alpha, squircle_border_coverage, Point, APPLE_CORNER_SMOOTHING,
};
use tiny_skia::{Pixmap, PremultipliedColorU8, Transform};

/// Icon background plate styling theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlateTheme {
    #[default]
    Light,
    Dark,
}

/// Strategy for framing an application icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MattingStrategy {
    /// Automatically detects if an icon is a circular or transparent cutout,
    /// generating an Apple-style continuous curvature plate only when appropriate.
    #[default]
    Auto,
    /// Always forces the icon into a macOS squircle plate.
    AlwaysPlate,
    /// Only clips the continuous curvature corners without generating a background plate.
    SquircleClipOnly,
    /// No plate and no corner clipping.
    Raw,
}

/// Options controlling squircle icon framing, scaling, and plates.
#[derive(Debug, Clone, Copy)]
pub struct PlateOptions {
    pub strategy: MattingStrategy,
    pub theme: PlateTheme,
    /// Relative corner radius as a ratio of half-size (Apple default is 0.444).
    pub corner_radius_ratio: f32,
    /// Relative scale of the inner glyph when placed on a plate (Apple standard ~0.78).
    pub glyph_scale: f32,
}

impl Default for PlateOptions {
    fn default() -> Self {
        Self {
            strategy: MattingStrategy::Auto,
            theme: PlateTheme::Light,
            corner_radius_ratio: 0.444,
            glyph_scale: 0.78,
        }
    }
}

/// Analyzes whether an icon's corners are transparent (floating cutout / circle).
pub fn is_cutout_icon(pixmap: &Pixmap) -> bool {
    let w = pixmap.width();
    let h = pixmap.height();
    if w < 8 || h < 8 {
        return false;
    }

    // Check corner regions (within 10% of width and height)
    let sample_w = (w / 10).max(2);
    let sample_h = (h / 10).max(2);

    let corners = [
        (0..sample_w, 0..sample_h),                         // Top-Left
        (w - sample_w..w, 0..sample_h),                     // Top-Right
        (0..sample_w, h - sample_h..h),                     // Bottom-Left
        (w - sample_w..w, h - sample_h..h),                 // Bottom-Right
    ];

    let mut transparent_corner_samples = 0;
    let mut total_corner_samples = 0;

    for (xs, ys) in corners {
        for y in ys {
            for x in xs.clone() {
                if let Some(pixel) = pixmap.pixel(x, y) {
                    total_corner_samples += 1;
                    if pixel.alpha() < 32 {
                        transparent_corner_samples += 1;
                    }
                }
            }
        }
    }

    if total_corner_samples == 0 {
        return false;
    }

    // If more than 75% of the 4 outer corner samples are transparent, it's a cutout/circle
    let ratio = transparent_corner_samples as f32 / total_corner_samples as f32;
    ratio >= 0.75
}

/// Applies macOS-style continuous curvature squircle masking and plate framing.
pub fn apply_squircle_plate(source: &Pixmap, options: PlateOptions) -> Pixmap {
    let width = source.width();
    let height = source.height();
    if width == 0 || height == 0 {
        return source.clone();
    }

    let needs_plate = match options.strategy {
        MattingStrategy::Raw => return source.clone(),
        MattingStrategy::SquircleClipOnly => false,
        MattingStrategy::AlwaysPlate => true,
        MattingStrategy::Auto => is_cutout_icon(source),
    };

    let half_w = width as f32 / 2.0;
    let half_h = height as f32 / 2.0;
    let half_size = Point { x: half_w, y: half_h };
    let radius = half_w.min(half_h) * options.corner_radius_ratio;

    let mut output = Pixmap::new(width, height).unwrap_or_else(|| source.clone());

    if needs_plate {
        // 1. Draw Apple squircle background plate with subtle gradient and hairline border
        let (top_color, bottom_color, border_color) = match options.theme {
            PlateTheme::Light => (
                (255, 255, 255, 248),
                (242, 242, 247, 248),
                (0, 0, 0, 24),
            ),
            PlateTheme::Dark => (
                (44, 44, 46, 248),
                (28, 28, 30, 248),
                (255, 255, 255, 30),
            ),
        };

        let pixels = output.pixels_mut();
        for y in 0..height {
            let py = y as f32 + 0.5;
            let rel_y = py - half_h;
            let grad_t = py / height as f32;
            let plate_r = lerp_u8(top_color.0, bottom_color.0, grad_t);
            let plate_g = lerp_u8(top_color.1, bottom_color.1, grad_t);
            let plate_b = lerp_u8(top_color.2, bottom_color.2, grad_t);
            let plate_a = lerp_u8(top_color.3, bottom_color.3, grad_t);

            for x in 0..width {
                let px = x as f32 + 0.5;
                let rel_x = px - half_w;
                let pt = Point { x: rel_x, y: rel_y };
                let (fill_alpha, border_cov) = squircle_border_coverage(
                    pt,
                    half_size,
                    radius,
                    0.75,
                    APPLE_CORNER_SMOOTHING,
                );
                let alpha = fill_alpha.max(border_cov);
                if alpha <= 0.001 {
                    continue;
                }

                // Blend plate color with inner border
                let final_r = lerp_u8(plate_r, border_color.0, border_cov);
                let final_g = lerp_u8(plate_g, border_color.1, border_cov);
                let final_b = lerp_u8(plate_b, border_color.2, border_cov);
                let final_a = (plate_a as f32 / 255.0 * alpha).clamp(0.0, 1.0);

                let premul = PremultipliedColorU8::from_rgba(
                    (final_r as f32 * final_a).round() as u8,
                    (final_g as f32 * final_a).round() as u8,
                    (final_b as f32 * final_a).round() as u8,
                    (final_a * 255.0).round() as u8,
                );
                if let Some(c) = premul {
                    pixels[(y * width + x) as usize] = c;
                }
            }
        }

        // 2. Composite the source glyph centered at scaled proportion
        let glyph_scale = options.glyph_scale.clamp(0.4, 0.95);
        let scaled_w = (width as f32 * glyph_scale).round() as u32;
        let scaled_h = (height as f32 * glyph_scale).round() as u32;

        let offset_x = (width - scaled_w) as f32 / 2.0;
        let offset_y = (height - scaled_h) as f32 / 2.0;

        let scale = glyph_scale;
        let transform = Transform::from_translate(offset_x, offset_y).post_scale(scale, scale);

        output.draw_pixmap(
            0,
            0,
            source.as_ref(),
            &tiny_skia::PixmapPaint::default(),
            transform,
            None,
        );
    } else {
        // Full-bleed icon: Clip corners directly with continuous curvature squircle mask
        let pixels = output.pixels_mut();
        let src_pixels = source.pixels();

        for y in 0..height {
            let py = y as f32 + 0.5;
            let rel_y = py - half_h;
            for x in 0..width {
                let px = x as f32 + 0.5;
                let rel_x = px - half_w;
                let pt = Point { x: rel_x, y: rel_y };
                let mask_alpha = squircle_alpha(pt, half_size, radius, APPLE_CORNER_SMOOTHING);
                if mask_alpha <= 0.001 {
                    continue;
                }

                let idx = (y * width + x) as usize;
                let src_pixel = src_pixels[idx];

                if mask_alpha >= 0.999 {
                    pixels[idx] = src_pixel;
                } else {
                    let a = (src_pixel.alpha() as f32 / 255.0 * mask_alpha).clamp(0.0, 1.0);
                    let r = (src_pixel.red() as f32 * mask_alpha).round() as u8;
                    let g = (src_pixel.green() as f32 * mask_alpha).round() as u8;
                    let b = (src_pixel.blue() as f32 * mask_alpha).round() as u8;
                    if let Some(c) = PremultipliedColorU8::from_rgba(r, g, b, (a * 255.0).round() as u8) {
                        pixels[idx] = c;
                    }
                }
            }
        }
    }

    output
}

#[inline]
fn lerp_u8(a: u8, b: u8, t: f32) -> u8 {
    let t = t.clamp(0.0, 1.0);
    ((a as f32) * (1.0 - t) + (b as f32) * t).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cutout_detection() {
        let mut cutout = Pixmap::new(100, 100).unwrap();
        // Draw a circle in the center, leave corners transparent
        let mut paint = tiny_skia::Paint::default();
        paint.set_color_rgba8(255, 0, 0, 255);
        let mut pb = tiny_skia::PathBuilder::new();
        pb.push_circle(50.0, 50.0, 40.0);
        let path = pb.finish().unwrap();
        cutout.fill_path(&path, &paint, tiny_skia::FillRule::Winding, Transform::identity(), None);

        assert!(is_cutout_icon(&cutout), "Circle with transparent corners should be detected as cutout");

        let mut full_bleed = Pixmap::new(100, 100).unwrap();
        let c = tiny_skia::Color::from_rgba8(0, 100, 200, 255);
        full_bleed.fill(c);
        assert!(!is_cutout_icon(&full_bleed), "Opaque rectangle should not be detected as cutout");
    }

    #[test]
    fn test_apply_squircle_plate_dimensions() {
        let mut icon = Pixmap::new(128, 128).unwrap();
        let c = tiny_skia::Color::WHITE;
        icon.fill(c);
        let framed = apply_squircle_plate(&icon, PlateOptions::default());
        assert_eq!(framed.width(), 128);
        assert_eq!(framed.height(), 128);
    }
}
