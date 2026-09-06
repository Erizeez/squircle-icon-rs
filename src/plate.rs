use squircle_rs::{
    squircle_path_commands, PathCommand, SquircleParams, APPLE_CORNER_SMOOTHING,
};
use tiny_skia::{
    Color, GradientStop, LinearGradient, Mask, Paint, Path, PathBuilder, Pixmap, PixmapPaint,
    SpreadMode, Stroke, Transform,
};

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
    /// Relative scale of the inner glyph when placed on a plate (Apple standard ~0.80).
    pub glyph_scale: f32,
}

impl Default for PlateOptions {
    fn default() -> Self {
        Self {
            strategy: MattingStrategy::Auto,
            theme: PlateTheme::Light,
            corner_radius_ratio: 0.444,
            glyph_scale: 0.80,
        }
    }
}

/// Constructs an exact Apple continuous curvature squircle [`tiny_skia::Path`].
pub fn build_squircle_path(
    width: f32,
    height: f32,
    corner_radius: f32,
    smoothing: f32,
) -> Option<Path> {
    let params = SquircleParams::new(width, height, corner_radius).with_smoothing(smoothing);
    let cmds = squircle_path_commands(&params);
    let mut pb = PathBuilder::new();
    for cmd in cmds {
        match cmd {
            PathCommand::MoveTo(p) => pb.move_to(p.x, p.y),
            PathCommand::LineTo(p) => pb.line_to(p.x, p.y),
            PathCommand::CubicTo { c0, c1, to } => {
                pb.cubic_to(c0.x, c0.y, c1.x, c1.y, to.x, to.y);
            }
            PathCommand::Close => pb.close(),
        }
    }
    pb.finish()
}

/// Analyzes whether an icon's corners are transparent (floating cutout / circle).
pub fn is_cutout_icon(pixmap: &Pixmap) -> bool {
    let w = pixmap.width();
    let h = pixmap.height();
    if w < 8 || h < 8 {
        return false;
    }

    // Check corner regions (within 8% of width and height)
    let sample_w = (w / 12).max(2);
    let sample_h = (h / 12).max(2);

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

    // If more than 60% of the 4 outer corner samples are transparent, it's a cutout/circle
    let ratio = transparent_corner_samples as f32 / total_corner_samples as f32;
    ratio >= 0.60
}

/// Applies macOS-style continuous curvature squircle masking and plate framing.
pub fn apply_squircle_plate(source: &Pixmap, options: PlateOptions) -> Pixmap {
    let width = source.width();
    let height = source.height();
    if width == 0 || height == 0 {
        return source.clone();
    }

    if options.strategy == MattingStrategy::Raw {
        return source.clone();
    }

    let w = width as f32;
    let h = height as f32;

    // Corner radius based on Apple macOS HIG (ratio ~0.224 of size, or 0.444 of half-size)
    let radius = (w.min(h) / 2.0) * options.corner_radius_ratio;
    let path = match build_squircle_path(w, h, radius, APPLE_CORNER_SMOOTHING) {
        Some(p) => p,
        None => return source.clone(),
    };

    let mut output = match Pixmap::new(width, height) {
        Some(p) => p,
        None => return source.clone(),
    };

    // 1. Render Apple squircle background plate (pure white with subtle depth gradient)
    let (top_color, bottom_color, border_color) = match options.theme {
        PlateTheme::Light => (
            Color::WHITE,
            Color::from_rgba8(242, 242, 247, 255),
            Color::from_rgba8(0, 0, 0, 20),
        ),
        PlateTheme::Dark => (
            Color::from_rgba8(48, 48, 51, 255),
            Color::from_rgba8(28, 28, 30, 255),
            Color::from_rgba8(255, 255, 255, 30),
        ),
    };

    let mut plate_paint = Paint::default();
    if let Some(shader) = LinearGradient::new(
        tiny_skia::Point::from_xy(w / 2.0, 0.0),
        tiny_skia::Point::from_xy(w / 2.0, h),
        vec![
            GradientStop::new(0.0, top_color),
            GradientStop::new(1.0, bottom_color),
        ],
        SpreadMode::Pad,
        Transform::identity(),
    ) {
        plate_paint.shader = shader;
    } else {
        plate_paint.set_color(top_color);
    }
    output.fill_path(&path, &plate_paint, tiny_skia::FillRule::Winding, Transform::identity(), None);

    // Subtle hairline inner border
    let border_width = (w / 128.0).max(1.0);
    let mut stroke_paint = Paint::default();
    stroke_paint.set_color(border_color);
    let stroke = Stroke {
        width: border_width,
        ..Default::default()
    };
    output.stroke_path(&path, &stroke_paint, &stroke, Transform::identity(), None);

    // 2. Prepare continuous curvature squircle clipping mask
    let mut mask = match Mask::new(width, height) {
        Some(m) => m,
        None => return output,
    };
    mask.fill_path(&path, tiny_skia::FillRule::Winding, true, Transform::identity());

    // 3. Composite source icon onto plate
    let is_cutout = match options.strategy {
        MattingStrategy::AlwaysPlate => true,
        MattingStrategy::SquircleClipOnly => false,
        MattingStrategy::Auto => is_cutout_icon(source),
        MattingStrategy::Raw => unreachable!(),
    };

    let transform = if is_cutout {
        // Floating glyph / cutout (Moonlight, CMake, mpv, System Settings):
        // Scale to glyph_scale (~0.80) and center on plate
        let scale = options.glyph_scale.clamp(0.5, 0.95);
        let dx = (w - (w * scale)) / 2.0;
        let dy = (h - (h * scale)) / 2.0;
        Transform::from_scale(scale, scale).post_translate(dx, dy)
    } else {
        // Full-bleed icon with background fill (Alacritty, Chrome, Firefox):
        // Render at 1.0 scale covering plate, clipped to squircle
        Transform::identity()
    };

    output.draw_pixmap(
        0,
        0,
        source.as_ref(),
        &PixmapPaint::default(),
        transform,
        Some(&mask),
    );

    output
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
    fn test_mask_api() {
        let mut mask = tiny_skia::Mask::new(100, 100).unwrap();
        let mut pb = tiny_skia::PathBuilder::new();
        pb.push_circle(50.0, 50.0, 40.0);
        let path = pb.finish().unwrap();
        mask.fill_path(&path, tiny_skia::FillRule::Winding, true, Transform::identity());
        let mut pixmap = Pixmap::new(100, 100).unwrap();
        let mut paint = tiny_skia::Paint::default();
        let grad = tiny_skia::LinearGradient::new(
            tiny_skia::Point::from_xy(50.0, 0.0),
            tiny_skia::Point::from_xy(50.0, 100.0),
            vec![
                tiny_skia::GradientStop::new(0.0, tiny_skia::Color::WHITE),
                tiny_skia::GradientStop::new(1.0, tiny_skia::Color::from_rgba8(240, 240, 245, 255)),
            ],
            tiny_skia::SpreadMode::Pad,
            Transform::identity(),
        ).unwrap();
        paint.shader = grad;
        pixmap.fill_path(&path, &paint, tiny_skia::FillRule::Winding, Transform::identity(), None);
        let mut stroke_paint = tiny_skia::Paint::default();
        stroke_paint.set_color_rgba8(0, 0, 0, 20);
        let stroke = tiny_skia::Stroke {
            width: 1.0,
            ..Default::default()
        };
        pixmap.stroke_path(&path, &stroke_paint, &stroke, Transform::identity(), None);
    }

    #[test]
    fn test_cmake_cutout_if_exists() {
        let p = std::path::Path::new("/usr/share/icons/hicolor/128x128/apps/CMakeSetup.png");
        if p.exists() {
            let pix = crate::raster::rasterize_file(p, 128, 128).unwrap();
            let cutout = is_cutout_icon(&pix);
            println!("CMake is_cutout_icon: {cutout}");
            assert!(cutout, "CMake icon should be recognized as cutout");
        }
    }
}
