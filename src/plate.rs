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

/// Detailed icon profile detected from spatial alpha distribution and color characteristics.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum IconProfile {
    /// Icon fills the entire canvas with opaque corners (e.g. Alacritty, Firefox).
    FullBleed,
    /// Icon is an existing squircle card with transparent padding (e.g. Apifox, PeaZip, Antigravity).
    /// Contains the scale factor and translation offsets needed to seamlessly bleed into the canvas,
    /// plus the synchronized top and bottom background colors of the card.
    PreFramedSquircle {
        scale: f32,
        offset_x: f32,
        offset_y: f32,
        top_color: Color,
        bottom_color: Color,
    },
    /// Icon has a uniform dark/colored circular rim (e.g. Moonlight).
    UniformColoredCircle {
        top_color: Color,
        bottom_color: Color,
    },
    /// Cutout glyph or multi-color badge (e.g. Chrome, CMake, Fcitx5, Alacritty).
    /// May contain intelligently extracted adaptive background plate colors (e.g. for dark terminal apps or solid bodies).
    FloatingCutout {
        top_color: Option<Color>,
        bottom_color: Option<Color>,
    },
}



fn color_saturation(c: Color) -> f32 {
    let max = c.red().max(c.green()).max(c.blue());
    let min = c.red().min(c.green()).min(c.blue());
    if max <= 0.001 { 0.0 } else { (max - min) / max }
}

fn average_colors(c1: Color, c2: Color) -> Color {
    let r = ((c1.red() + c2.red()) / 2.0 * 255.0).round() as u8;
    let g = ((c1.green() + c2.green()) / 2.0 * 255.0).round() as u8;
    let b = ((c1.blue() + c2.blue()) / 2.0 * 255.0).round() as u8;
    Color::from_rgba8(r, g, b, 255)
}

fn pick_adaptive_color_pair(c1: Option<Color>, c2: Option<Color>) -> Option<Color> {
    match (c1, c2) {
        (Some(a), Some(b)) => {
            let sat_a = color_saturation(a);
            let sat_b = color_saturation(b);
            if sat_a > 0.35 && sat_b < 0.20 {
                // b hit a white/grayscale foreground glyph, preserve colorful card background a
                Some(a)
            } else if sat_b > 0.35 && sat_a < 0.20 {
                // a hit a white/grayscale foreground glyph, preserve colorful card background b
                Some(b)
            } else {
                Some(average_colors(a, b))
            }
        }
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

/// Detects the icon archetype from its alpha distribution and color symmetry.
pub fn detect_icon_profile(pixmap: &Pixmap) -> IconProfile {
    let w = pixmap.width();
    let h = pixmap.height();
    if w < 8 || h < 8 {
        return IconProfile::FullBleed;
    }


    // 2. Scan bounding box and opaque pixel density
    let mut min_x = w;
    let mut max_x = 0;
    let mut min_y = h;
    let mut max_y = 0;
    let mut opaque_count = 0;

    for y in 0..h {
        for x in 0..w {
            if let Some(p) = pixmap.pixel(x, y)
                && p.alpha() > 32
            {
                opaque_count += 1;
                min_x = min_x.min(x);
                max_x = max_x.max(x);
                min_y = min_y.min(y);
                max_y = max_y.max(y);
            }
        }
    }

    if max_x < min_x || max_y < min_y || opaque_count == 0 {
        return IconProfile::FloatingCutout {
            top_color: None,
            bottom_color: None,
        };
    }

    let box_w = max_x - min_x + 1;
    let box_h = max_y - min_y + 1;
    let box_area = (box_w * box_h) as f32;
    let box_fill_ratio = opaque_count as f32 / box_area;
    let w_ratio = box_w as f32 / w as f32;
    let h_ratio = box_h as f32 / h as f32;

    // 3. True Full-Bleed detection (e.g. WeChat, Feishu):
    // The graphic content spans virtually the entire canvas AND touches the canvas perimeter,
    // AND the canvas corners are NOT transparent.
    let mid_x = w / 2;
    let mid_y = h / 2;
    let edge_touches = [
        pixmap.pixel(mid_x, 0).or_else(|| pixmap.pixel(mid_x, 1)),
        pixmap.pixel(mid_x, h.saturating_sub(1)).or_else(|| pixmap.pixel(mid_x, h.saturating_sub(2))),
        pixmap.pixel(0, mid_y).or_else(|| pixmap.pixel(1, mid_y)),
        pixmap.pixel(w.saturating_sub(1), mid_y).or_else(|| pixmap.pixel(w.saturating_sub(2), mid_y)),
    ]
    .into_iter()
    .filter(|p| p.is_some_and(|px| px.alpha() > 128))
    .count();

    let corners_transparent = [
        pixmap.pixel(0, 0),
        pixmap.pixel(w.saturating_sub(1), 0),
        pixmap.pixel(0, h.saturating_sub(1)),
        pixmap.pixel(w.saturating_sub(1), h.saturating_sub(1)),
    ]
    .into_iter()
    .all(|p| p.is_none_or(|px| px.alpha() < 32));

    if !corners_transparent && edge_touches >= 3 && w_ratio >= 0.96 && h_ratio >= 0.96 && box_fill_ratio >= 0.88 {
        return IconProfile::FullBleed;
    }

    // 4. PreFramedSquircle detection (inner card with transparent padding, rounded corners, or subtle drop shadow):
    // macOS Big Sur standard: 824px in 1024px canvas (~80.5% width/height).
    // Cards with subtle drop shadows or padding may span up to 100% of width/height.
    // The continuous squircle fills ~86-96% of its bounding box.
    // In addition, all four boundary edges (top, bottom, left, right) must span >= 60% of the box
    // to strictly distinguish cards from T-shaped objects (monitors with stands), triangles, etc.
    if (0.70..=1.00).contains(&w_ratio)
        && (0.70..=1.00).contains(&h_ratio)
        && (w_ratio - h_ratio).abs() < 0.10
        && box_fill_ratio >= 0.86
    {
        let inset_y = (box_h / 16).max(3);
        let y_top = min_y + inset_y;
        let y_bot = max_y.saturating_sub(inset_y);
        let top_span = (min_x..=max_x)
            .filter(|&x| pixmap.pixel(x, y_top).is_some_and(|p| p.alpha() > 32))
            .count();
        let bot_span = (min_x..=max_x)
            .filter(|&x| pixmap.pixel(x, y_bot).is_some_and(|p| p.alpha() > 32))
            .count();

        let inset_x = (box_w / 16).max(3);
        let x_left = min_x + inset_x;
        let x_right = max_x.saturating_sub(inset_x);
        let left_span = (min_y..=max_y)
            .filter(|&y| pixmap.pixel(x_left, y).is_some_and(|p| p.alpha() > 32))
            .count();
        let right_span = (min_y..=max_y)
            .filter(|&y| pixmap.pixel(x_right, y).is_some_and(|p| p.alpha() > 32))
            .count();

        let is_squircle_card = (top_span as f32 / box_w as f32) >= 0.60
            && (bot_span as f32 / box_w as f32) >= 0.60
            && (left_span as f32 / box_h as f32) >= 0.60
            && (right_span as f32 / box_h as f32) >= 0.60;

        if is_squircle_card {
            let sx = w as f32 / box_w as f32;
            let sy = h as f32 / box_h as f32;
            let base_s = sx.min(sy);
            let bleed = if w_ratio > 0.88 && box_fill_ratio < 0.92 {
                // Card with sprawling drop shadows (like Alger Music Player):
                // Needs extra magnification to push the drop shadow outside the squircle mask.
                1.18
            } else if w_ratio <= 0.85 {
                // Pre-framed card with generous outer padding (e.g. Flatpak macOS icons ~80% width):
                // Needs 1.20 bleed so the card's original small rounded corner arc is completely outside the continuous squircle mask.
                1.20
            } else {
                // Card spanning near full canvas (e.g. 96-100% like local desktop assets):
                // Gentle 1.04 bleed eliminates anti-aliasing edges without resizing artwork.
                1.04
            };
            let s = base_s * bleed;
            let cx = (min_x + max_x) as f32 / 2.0;
            let cy = (min_y + max_y) as f32 / 2.0;
            let offset_x = (w as f32 / 2.0) - cx * s;
            let offset_y = (h as f32 / 2.0) - cy * s;

            // Sample corner colors with adaptive insets to guarantee hitting the card plate rather than central artwork
            let sample_corner = |x_base: u32, y_base: u32, x_dir: i32, y_dir: i32| -> Option<Color> {
                for pct in [0.08, 0.06, 0.10, 0.12] {
                    let ix = ((box_w as f32) * pct) as i32 * x_dir;
                    let iy = ((box_h as f32) * pct) as i32 * y_dir;
                    let x = (x_base as i32 + ix).clamp(min_x as i32, max_x as i32) as u32;
                    let y = (y_base as i32 + iy).clamp(min_y as i32, max_y as i32) as u32;
                    if let Some(p) = pixmap.pixel(x, y) && p.alpha() > 200 {
                        let r = (p.red() as f32 / p.alpha() as f32 * 255.0).round() as u8;
                        let g = (p.green() as f32 / p.alpha() as f32 * 255.0).round() as u8;
                        let b = (p.blue() as f32 / p.alpha() as f32 * 255.0).round() as u8;
                        return Some(Color::from_rgba8(r, g, b, 255));
                    }
                }
                None
            };

            let pt_tl = sample_corner(min_x, min_y, 1, 1);
            let pt_tr = sample_corner(max_x, min_y, -1, 1);
            let pb_bl = sample_corner(min_x, max_y, 1, -1);
            let pb_br = sample_corner(max_x, max_y, -1, -1);

            let top_color = pick_adaptive_color_pair(pt_tl, pt_tr).unwrap_or(Color::WHITE);
            let bottom_color = pick_adaptive_color_pair(pb_bl, pb_br).unwrap_or(top_color);

            return IconProfile::PreFramedSquircle {
                scale: s,
                offset_x,
                offset_y,
                top_color,
                bottom_color,
            };
        }
    }

    // 4. Circle with uniform rim detection (e.g. Moonlight, Clash Verge):
    // Circle fills pi/4 = ~78.5% of its bounding box.
    if (0.73..=0.84).contains(&box_fill_ratio) && w_ratio >= 0.85 && h_ratio >= 0.85 {
        let mid_x = (min_x + max_x) / 2;
        let mid_y = (min_y + max_y) / 2;
        let p_top = pixmap.pixel(mid_x, (min_y + 4).min(max_y));
        let p_bot = pixmap.pixel(mid_x, (max_y.saturating_sub(4)).max(min_y));
        let p_left = pixmap.pixel((min_x + 4).min(max_x), mid_y);
        let p_right = pixmap.pixel((max_x.saturating_sub(4)).max(min_x), mid_y);

        if let (Some(top), Some(bot), Some(left), Some(right)) = (p_top, p_bot, p_left, p_right)
            && top.alpha() > 200
            && bot.alpha() > 200
            && left.alpha() > 200
            && right.alpha() > 200
        {
            let diff_lr = (left.red() as i32 - right.red() as i32).abs()
                + (left.green() as i32 - right.green() as i32).abs()
                + (left.blue() as i32 - right.blue() as i32).abs();
            if diff_lr < 60 {
                return IconProfile::UniformColoredCircle {
                    top_color: Color::from_rgba8(top.red(), top.green(), top.blue(), 255),
                    bottom_color: Color::from_rgba8(bot.red(), bot.green(), bot.blue(), 255),
                };
            }
        }
    }

    // 5. Default: floating cutout / multi-color badge / irregular glyph.
    // Intelligently extract ambient/background colors instead of blindly falling back to white:
    // When an icon represents a solid dark terminal/IDE, dark window frame, or dark-themed app container
    // (e.g. Alacritty, Kitty, Neovim, terminal utilities), extract its ambient dark slate/charcoal tones.
    // IMPORTANT: Dark plates must NEVER be applied to sparse wireframes or monochrome silhouettes
    // (e.g. network-wired / avahi, hwloc, keyboard outlines) where dark foreground on dark plate
    // destroys contrast and renders the icon invisible.
    let total_pixels = (w * h) as f32;
    let canvas_fill_ratio = opaque_count as f32 / total_pixels;

    let is_solid_host_card = box_fill_ratio >= 0.70
        && canvas_fill_ratio >= 0.50
        && w_ratio >= 0.75
        && h_ratio >= 0.75;

    if is_solid_host_card {
        let mut dark_count = 0u64;
        for pixel in pixmap.pixels() {
            if pixel.alpha() > 48 {
                let max_c = pixel.red().max(pixel.green()).max(pixel.blue());
                if max_c < 80 {
                    dark_count += 1;
                }
            }
        }
        let dark_ratio = dark_count as f32 / opaque_count.max(1) as f32;
        if dark_ratio >= 0.45 {
            // Sample dark frame / ambient tone from bottom perimeter
            let inset_y = (box_h / 8).max(2);
            let mut bot_r = 0u32;
            let mut bot_g = 0u32;
            let mut bot_b = 0u32;
            let mut bot_n = 0u32;
            let y_sample = max_y.saturating_sub(inset_y).max(min_y);
            for x in (min_x + box_w / 4)..=(max_x.saturating_sub(box_w / 4)) {
                if let Some(p) = pixmap.pixel(x, y_sample) && p.alpha() > 48 {
                    bot_r += p.red() as u32;
                    bot_g += p.green() as u32;
                    bot_b += p.blue() as u32;
                    bot_n += 1;
                }
            }
            let (br, bg, bb) = match bot_n {
                0 => (35, 37, 43),
                n => (bot_r / n, bot_g / n, bot_b / n),
            };

            // Anchor dark surface luminance to macOS HIG standards (~28-48 top, ~16-26 bottom)
            // keeping the hue/saturation from the icon's authentic palette
            let top_r = ((br as f32) * 0.55).clamp(26.0, 48.0) as u8;
            let top_g = ((bg as f32) * 0.55).clamp(28.0, 50.0) as u8;
            let top_b = ((bb as f32) * 0.55).clamp(32.0, 56.0) as u8;

            let bot_r = ((br as f32) * 0.30).clamp(14.0, 26.0) as u8;
            let bot_g = ((bg as f32) * 0.30).clamp(15.0, 28.0) as u8;
            let bot_b = ((bb as f32) * 0.30).clamp(18.0, 32.0) as u8;

            return IconProfile::FloatingCutout {
                top_color: Some(Color::from_rgba8(top_r, top_g, top_b, 255)),
                bottom_color: Some(Color::from_rgba8(bot_r, bot_g, bot_b, 255)),
            };
        }
    }

    IconProfile::FloatingCutout {
        top_color: None,
        bottom_color: None,
    }
}

/// Analyzes whether an icon's corners are transparent (floating cutout / circle).
pub fn is_cutout_icon(pixmap: &Pixmap) -> bool {
    !matches!(detect_icon_profile(pixmap), IconProfile::FullBleed)
}

/// Applies macOS-style continuous curvature squircle masking and adaptive plate framing.
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

    // 1. Prepare continuous curvature squircle clipping mask
    let mut mask = match Mask::new(width, height) {
        Some(m) => m,
        None => return output,
    };
    mask.fill_path(&path, tiny_skia::FillRule::Winding, true, Transform::identity());

    // 2. Identify icon archetype
    let profile = match options.strategy {
        MattingStrategy::AlwaysPlate => match detect_icon_profile(source) {
            IconProfile::FloatingCutout { top_color, bottom_color } => {
                IconProfile::FloatingCutout { top_color, bottom_color }
            }
            _ => IconProfile::FloatingCutout {
                top_color: None,
                bottom_color: None,
            },
        },
        MattingStrategy::SquircleClipOnly => IconProfile::FullBleed,
        MattingStrategy::Auto => detect_icon_profile(source),
        MattingStrategy::Raw => unreachable!(),
    };


    let pixmap_paint = PixmapPaint {
        quality: tiny_skia::FilterQuality::Bicubic,
        ..Default::default()
    };

    match profile {
        IconProfile::FullBleed => {
            // Full-bleed: render at scale 1.0 covering the tile, clipped to squircle.
            // Pre-fill squircle plate to guarantee that zero transparent holes exist inside the squircle mask.
            let (top_color, bottom_color) = match options.theme {
                PlateTheme::Light => (Color::WHITE, Color::from_rgba8(242, 242, 247, 255)),
                PlateTheme::Dark => (Color::from_rgba8(48, 48, 51, 255), Color::from_rgba8(28, 28, 30, 255)),
            };
            let mut plate_paint = Paint::default();
            if let Some(shader) = LinearGradient::new(
                tiny_skia::Point::from_xy(w / 2.0, 0.0),
                tiny_skia::Point::from_xy(w / 2.0, h),
                vec![GradientStop::new(0.0, top_color), GradientStop::new(1.0, bottom_color)],
                SpreadMode::Pad,
                Transform::identity(),
            ) {
                plate_paint.shader = shader;
            } else {
                plate_paint.set_color(top_color);
            }
            output.fill_path(&path, &plate_paint, tiny_skia::FillRule::Winding, Transform::identity(), None);
            output.draw_pixmap(0, 0, source.as_ref(), &pixmap_paint, Transform::identity(), Some(&mask));
        }
        IconProfile::PreFramedSquircle {
            scale,
            offset_x,
            offset_y,
            top_color,
            bottom_color,
        } => {
            // Adaptive Fusion (Apifox, PeaZip, Antigravity):
            // 1. Fill plate with synchronized background color/gradient matching the card
            let mut plate_paint = Paint::default();
            if let Some(shader) = LinearGradient::new(
                tiny_skia::Point::from_xy(w / 2.0, 0.0),
                tiny_skia::Point::from_xy(w / 2.0, h),
                vec![GradientStop::new(0.0, top_color), GradientStop::new(1.0, bottom_color)],
                SpreadMode::Pad,
                Transform::identity(),
            ) {
                plate_paint.shader = shader;
            } else {
                plate_paint.set_color(top_color);
            }
            output.fill_path(&path, &plate_paint, tiny_skia::FillRule::Winding, Transform::identity(), None);

            // 2. Scale and center the existing squircle card with margin bleed over the synchronized background
            let transform = Transform::from_scale(scale, scale).post_translate(offset_x, offset_y);
            output.draw_pixmap(0, 0, source.as_ref(), &pixmap_paint, transform, Some(&mask));
        }
        IconProfile::UniformColoredCircle { top_color, bottom_color } => {
            // Circular icon with uniform rim (e.g. Moonlight):
            // Render adaptive plate matching the rim tone
            let mut plate_paint = Paint::default();
            if let Some(shader) = LinearGradient::new(
                tiny_skia::Point::from_xy(w / 2.0, 0.0),
                tiny_skia::Point::from_xy(w / 2.0, h),
                vec![GradientStop::new(0.0, top_color), GradientStop::new(1.0, bottom_color)],
                SpreadMode::Pad,
                Transform::identity(),
            ) {
                plate_paint.shader = shader;
            } else {
                plate_paint.set_color(top_color);
            }
            output.fill_path(&path, &plate_paint, tiny_skia::FillRule::Winding, Transform::identity(), None);

            let scale = options.glyph_scale.clamp(0.5, 0.95);
            let dx = (w - (w * scale)) / 2.0;
            let dy = (h - (h * scale)) / 2.0;
            let transform = Transform::from_scale(scale, scale).post_translate(dx, dy);
            output.draw_pixmap(0, 0, source.as_ref(), &pixmap_paint, transform, Some(&mask));
        }
        IconProfile::FloatingCutout { top_color: extracted_top, bottom_color: extracted_bottom } => {
            // Floating glyph / cutout (Chrome, CMake, Fcitx5, lstopo, Alacritty):
            // Intelligently use extracted adaptive plate colors if available, otherwise theme defaults
            let (top_color, bottom_color) = match (extracted_top, extracted_bottom) {
                (Some(t), Some(b)) => (t, b),
                (Some(t), None) => (t, t),
                _ => match options.theme {
                    PlateTheme::Light => (Color::WHITE, Color::from_rgba8(242, 242, 247, 255)),
                    PlateTheme::Dark => (Color::from_rgba8(48, 48, 51, 255), Color::from_rgba8(28, 28, 30, 255)),
                },
            };
            let mut plate_paint = Paint::default();
            if let Some(shader) = LinearGradient::new(
                tiny_skia::Point::from_xy(w / 2.0, 0.0),
                tiny_skia::Point::from_xy(w / 2.0, h),
                vec![GradientStop::new(0.0, top_color), GradientStop::new(1.0, bottom_color)],
                SpreadMode::Pad,
                Transform::identity(),
            ) {
                plate_paint.shader = shader;
            } else {
                plate_paint.set_color(top_color);
            }
            output.fill_path(&path, &plate_paint, tiny_skia::FillRule::Winding, Transform::identity(), None);

            let scale = options.glyph_scale.clamp(0.5, 0.95);
            let dx = (w - (w * scale)) / 2.0;
            let dy = (h - (h * scale)) / 2.0;
            let transform = Transform::from_scale(scale, scale).post_translate(dx, dy);
            output.draw_pixmap(0, 0, source.as_ref(), &pixmap_paint, transform, Some(&mask));
        }
    }

    // Subtle hairline inner border clipped strictly inside squircle.
    // When the plate surface is dark (either from Dark theme or extracted dark tones),
    // use a crisp translucent white stroke (rgba 255, 255, 255, 30) to catch the rim light.
    let is_dark_surface = match profile {
        IconProfile::FloatingCutout { top_color: Some(top), .. } => {
            let lum = 0.299 * top.red() + 0.587 * top.green() + 0.114 * top.blue();
            lum < 0.45
        }
        _ => options.theme == PlateTheme::Dark,
    };
    let border_color = if is_dark_surface {
        Color::from_rgba8(255, 255, 255, 30)
    } else {
        match options.theme {
            PlateTheme::Light => Color::from_rgba8(0, 0, 0, 20),
            PlateTheme::Dark => Color::from_rgba8(255, 255, 255, 30),
        }
    };

    let border_width = (w / 128.0).max(1.0);
    let mut stroke_paint = Paint::default();
    stroke_paint.set_color(border_color);
    let stroke = Stroke {
        width: border_width,
        ..Default::default()
    };
    output.stroke_path(&path, &stroke_paint, &stroke, Transform::identity(), Some(&mask));

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
    fn test_profile_detection_synthetic() {
        // 1. Full bleed (completely opaque)
        let mut full_bleed = Pixmap::new(100, 100).unwrap();
        full_bleed.fill(Color::from_rgba8(20, 50, 100, 255));
        assert_eq!(detect_icon_profile(&full_bleed), IconProfile::FullBleed);

        // 2. Pre-framed squircle card (80x80 card in 100x100 canvas, squircle shape)
        let mut squircle_card = Pixmap::new(100, 100).unwrap();
        let path = build_squircle_path(80.0, 80.0, 18.0, squircle_rs::APPLE_CORNER_SMOOTHING).unwrap();
        let mut paint = Paint::default();
        paint.set_color_rgba8(255, 100, 50, 255);
        squircle_card.fill_path(&path, &paint, tiny_skia::FillRule::Winding, Transform::from_translate(10.0, 10.0), None);
        assert!(matches!(detect_icon_profile(&squircle_card), IconProfile::PreFramedSquircle { .. }));

        // 3. Render squircle plate on pre-framed card
        let rendered = apply_squircle_plate(&squircle_card, PlateOptions::default());
        assert_eq!(rendered.width(), 100);
        assert_eq!(rendered.height(), 100);
    }

    #[test]
    fn test_profile_detection_system_icons() {
        let targets = [
            ("Apifox", "/var/lib/flatpak/appstream/flathub/x86_64/70da372709099bbdd422326b03f09cdf46afe365999abb4aac127a5b9bc7f0ad/icons/128x128/com.apifox.Apifox.png", "squircle"),
            ("PeaZip", "/var/lib/flatpak/appstream/flathub/x86_64/70da372709099bbdd422326b03f09cdf46afe365999abb4aac127a5b9bc7f0ad/icons/128x128/io.github.peazip.PeaZip.png", "squircle"),
            ("Antigravity", "/home/eriz/.local/share/icons/hicolor/512x512/apps/antigravity.png", "squircle"),
            ("Clash Verge", "/usr/share/icons/hicolor/128x128/apps/clash-verge.png", "circle"),
            ("lstopo (hwloc)", "/home/eriz/.local/share/icons/hicolor/scalable/apps/hwloc.svg", "cutout"),
            ("WeChat", "/usr/share/icons/hicolor/128x128/apps/wechat.png", "squircle"),
            ("CMake", "/usr/share/icons/hicolor/128x128/apps/CMakeSetup.png", "cutout"),
            ("Alacritty", "/usr/share/pixmaps/Alacritty.svg", "cutout"),
            ("Alger Music Player", "/home/eriz/.local/share/icons/hicolor/512x512/apps/algermusicplayer.png", "squircle"),
            ("network-wired", "/usr/share/icons/breeze/devices/24/network-wired.svg", "cutout_light"),
        ];

        for (name, path, expected) in targets {
            let p = std::path::Path::new(path);
            if !p.exists() {
                continue;
            }
            let pix = crate::raster::rasterize_file(p, 128, 128).unwrap();
            let profile = detect_icon_profile(&pix);
            match expected {
                "squircle" => assert!(
                    matches!(profile, IconProfile::PreFramedSquircle { .. }),
                    "{name} expected PreFramedSquircle, got {:?}", profile
                ),
                "circle" => assert!(
                    matches!(profile, IconProfile::UniformColoredCircle { .. }),
                    "{name} expected UniformColoredCircle, got {:?}", profile
                ),
                "full_bleed" => assert!(
                    matches!(profile, IconProfile::FullBleed),
                    "{name} expected FullBleed, got {:?}", profile
                ),
                "cutout" => {
                    assert!(
                        matches!(profile, IconProfile::FloatingCutout { .. }),
                        "{name} expected FloatingCutout, got {:?}", profile
                    );
                    if name == "Alacritty" {
                        if let IconProfile::FloatingCutout { top_color, bottom_color } = profile {
                            assert!(top_color.is_some(), "Alacritty should have extracted dark adaptive plate top color");
                            assert!(bottom_color.is_some(), "Alacritty should have extracted dark adaptive plate bottom color");
                        }
                    }
                }
                "cutout_light" => {
                    assert!(
                        matches!(profile, IconProfile::FloatingCutout { top_color: None, bottom_color: None }),
                        "{name} expected FloatingCutout with default light plate, got {:?}", profile
                    );
                }
                _ => {}
            }
        }
    }
}
