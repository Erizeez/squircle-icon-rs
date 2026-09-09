//! Liquid Glass Layered Icon Renderer Demo.
//!
//! Renders decomposed application icon layer packages extracted by `macos-icons`
//! using `liquid-rs` physical glass optics models and `squircle-icon-rs` compositing.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use liquid_rs::render::{GpuRenderer, GpuSize};
use liquid_rs::scene::{GlassMaterial, GlassScene, GlareStyle};
use resvg::usvg;
use serde::Deserialize;
use tiny_skia::{BlendMode, Pixmap, PixmapPaint, PremultipliedColorU8, Transform};

/// Headless GPU context wrapping liquid-rs WGPU rendering engine.
#[allow(dead_code)]
struct LiquidGpuContext {
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: GpuRenderer,
    size: GpuSize,
}

#[allow(dead_code)]
impl LiquidGpuContext {
    fn new(width: u32, height: u32) -> Result<Self, String> {
        let size = GpuSize::new(width, height);
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        }))
        .map_err(|e| format!("Failed to find suitable WGPU adapter: {:?}", e))?;

        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("liquid-glass-icon-device"),
                ..Default::default()
            },
        ))
        .map_err(|e| format!("Failed to create WGPU device: {:?}", e))?;

        let mut renderer = GpuRenderer::from_device(device.clone(), queue.clone(), size);
        renderer.set_transparent_background(true);
        Ok(Self { device, queue, renderer, size })
    }

    /// Renders a GlassScene over a backdrop pixmap with real liquid-rs GPU shaders.
    fn render_glass_scene(
        &mut self,
        backdrop: &Pixmap,
        scene: &GlassScene,
    ) -> Result<Pixmap, String> {
        let w = backdrop.width();
        let h = backdrop.height();

        // 1. Convert tiny_skia premultiplied pixels to straight RGBA8 for WGPU
        let mut straight_rgba = Vec::with_capacity((w * h * 4) as usize);
        for p in backdrop.pixels() {
            let a = p.alpha();
            if a == 0 {
                straight_rgba.extend_from_slice(&[0, 0, 0, 0]);
            } else if a == 255 {
                straight_rgba.extend_from_slice(&[p.red(), p.green(), p.blue(), 255]);
            } else {
                let inv_a = 255.0 / (a as f32);
                let r = ((p.red() as f32 * inv_a).round() as u8).min(255);
                let g = ((p.green() as f32 * inv_a).round() as u8).min(255);
                let b = ((p.blue() as f32 * inv_a).round() as u8).min(255);
                straight_rgba.extend_from_slice(&[r, g, b, a]);
            }
        }

        // 2. Upload straight RGBA8 backdrop to a source texture with COPY_SRC
        let source_texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("liquid-glass source backdrop texture"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.renderer.output_format(),
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &source_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &straight_rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w * 4),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );

        // 3. Create output texture with COPY_SRC
        let output_texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("liquid-glass output texture"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.renderer.output_format(),
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let output_view = output_texture.create_view(&wgpu::TextureViewDescriptor::default());

        // 4. Render the scene with the backdrop source using liquid-rs GPU shaders
        self.renderer
            .render_scene_to_view_with_source(&output_view, &source_texture, scene, 0.0)
            .map_err(|e| format!("Render scene with source error: {:?}", e))?;

        // 5. Read back the rendered texture from GPU to CPU
        let padded_bytes_per_row = (w * 4 + 255) & !255;
        let buffer_size = (padded_bytes_per_row * h) as u64;
        let readback_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("liquid readback buffer"),
            size: buffer_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("readback encoder"),
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &output_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback_buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_bytes_per_row),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        self.queue.submit(Some(encoder.finish()));

        let slice = readback_buffer.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |v| sender.send(v).unwrap());
        self.device.poll(wgpu::PollType::wait_indefinitely()).map_err(|e| format!("Poll error: {:?}", e))?;
        receiver.recv().unwrap().map_err(|e| format!("Readback wait error: {:?}", e))?;

        let mapped = slice.get_mapped_range();
        let mut out_pixmap = Pixmap::new(w, h).ok_or("Failed to allocate Pixmap")?;
        let out_pixels = out_pixmap.pixels_mut();

        for y in 0..h {
            let row_start = (y * padded_bytes_per_row) as usize;
            for x in 0..w {
                let px_start = row_start + (x * 4) as usize;
                let r = mapped[px_start];
                let g = mapped[px_start + 1];
                let b = mapped[px_start + 2];
                let a = mapped[px_start + 3];
                let pr = (((r as u16 * a as u16 + 128) / 255) as u8).min(a);
                let pg = (((g as u16 * a as u16 + 128) / 255) as u8).min(a);
                let pb = (((b as u16 * a as u16 + 128) / 255) as u8).min(a);
                out_pixels[(y * w + x) as usize] = PremultipliedColorU8::from_rgba(pr, pg, pb, a).unwrap_or(PremultipliedColorU8::TRANSPARENT);
            }
        }
        drop(mapped);
        readback_buffer.unmap();

        Ok(out_pixmap)
    }
}

#[derive(Debug, Deserialize)]
struct AppManifest {
    slug: String,
    title: String,
    has_squircle: bool,
    appearances: HashMap<String, Vec<LayerEntry>>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct LayerOpticsRefraction {
    strength: f32,
    height: f32,
    #[serde(default)]
    is_pure_lens: bool,
    #[serde(default)]
    suggested_liquid_strength: Option<f32>,
    #[serde(default)]
    suggested_liquid_thickness: Option<f32>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct LayerOpticsShadow {
    opacity: f32,
    style: i32,
    #[serde(default)]
    suggested_liquid_factor: Option<f32>,
    #[serde(default)]
    suggested_liquid_expand: Option<f32>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct LayerOptics {
    #[serde(default)]
    translucency: f32,
    #[serde(default)]
    blur_strength: f32,
    #[serde(default)]
    refraction: Option<LayerOpticsRefraction>,
    #[serde(default)]
    specular: bool,
    #[serde(default)]
    specular_placement: i32,
    #[serde(default)]
    shadow: Option<LayerOpticsShadow>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct LiquidMaterialHint {
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    variant: Option<String>,
    #[serde(default)]
    has_fresnel_rim: Option<bool>,
    #[serde(default)]
    has_glare: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct LayerEntry {
    name: String,
    #[serde(default)]
    canon_id: Option<String>,
    #[serde(rename = "type")]
    layer_type: String,
    file: String,
    #[serde(default = "default_position")]
    position: String,
    #[serde(default = "default_blend_mode")]
    blend_mode: String,
    #[serde(default = "default_opacity")]
    opacity: f32,
    optics: LayerOptics,
    #[serde(default)]
    liquid_material_hint: Option<LiquidMaterialHint>,
}

fn default_position() -> String {
    "0,0".to_string()
}
fn default_blend_mode() -> String {
    "Normal".to_string()
}
fn default_opacity() -> f32 {
    1.0
}

/// Parses tiny_skia BlendMode from Apple Assets.car string.
fn parse_blend_mode(mode_str: &str) -> BlendMode {
    match mode_str.to_lowercase().as_str() {
        "screen" => BlendMode::Screen,
        "multiply" => BlendMode::Multiply,
        "overlay" => BlendMode::Overlay,
        "darken" => BlendMode::Darken,
        "lighten" => BlendMode::Lighten,
        "color_dodge" | "colordodge" => BlendMode::ColorDodge,
        "color_burn" | "colorburn" => BlendMode::ColorBurn,
        "hard_light" | "hardlight" => BlendMode::HardLight,
        "soft_light" | "softlight" => BlendMode::SoftLight,
        "difference" => BlendMode::Difference,
        "exclusion" => BlendMode::Exclusion,
        _ => BlendMode::SourceOver,
    }
}

/// Rasterizes an SVG file into a tiny_skia Pixmap at specified dimensions.
fn render_svg_file(path: &Path, width: u32, height: u32) -> Result<Pixmap, String> {
    let svg_data = fs::read(path).map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
    let opt = usvg::Options::default();
    let tree = usvg::Tree::from_data(&svg_data, &opt)
        .map_err(|e| format!("SVG parse error {}: {}", path.display(), e))?;

    let tree_size = tree.size();
    let sx = width as f32 / tree_size.width();
    let sy = height as f32 / tree_size.height();
    let scale = sx.min(sy);
    let dx = (width as f32 - tree_size.width() * scale) / 2.0;
    let dy = (height as f32 - tree_size.height() * scale) / 2.0;

    let transform = Transform::from_translate(dx, dy).post_scale(scale, scale);
    let mut pixmap = Pixmap::new(width, height).ok_or("Pixmap allocation failed")?;
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    Ok(pixmap)
}

/// Rasterizes SVG string into a tiny_skia Pixmap.
fn render_svg_str(svg_data: &str, width: u32, height: u32) -> Result<Pixmap, String> {
    let opt = usvg::Options::default();
    let tree = usvg::Tree::from_str(svg_data, &opt)
        .map_err(|e| format!("SVG parse error: {}", e))?;

    let tree_size = tree.size();
    let sx = width as f32 / tree_size.width();
    let sy = height as f32 / tree_size.height();
    let scale = sx.min(sy);
    let dx = (width as f32 - tree_size.width() * scale) / 2.0;
    let dy = (height as f32 - tree_size.height() * scale) / 2.0;

    let transform = Transform::from_translate(dx, dy).post_scale(scale, scale);
    let mut pixmap = Pixmap::new(width, height).ok_or("Pixmap allocation failed")?;
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    Ok(pixmap)
}

/// Loads a PNG image file into a tiny_skia Pixmap at specified dimensions.
fn load_png_to_pixmap(path: &Path, width: u32, height: u32) -> Option<Pixmap> {
    let img = image::open(path).ok()?.to_rgba8();
    let img = if img.width() != width || img.height() != height {
        image::imageops::resize(&img, width, height, image::imageops::FilterType::Lanczos3)
    } else {
        img
    };
    let mut pixmap = Pixmap::new(width, height)?;
    let dest_pixels = pixmap.pixels_mut();
    for (i, p) in img.pixels().enumerate() {
        dest_pixels[i] = PremultipliedColorU8::from_rgba(p[0], p[1], p[2], p[3])
            .unwrap_or(PremultipliedColorU8::TRANSPARENT);
    }
    Some(pixmap)
}

/// Maps layer optics parameters to a liquid-rs physical glass material.
fn build_liquid_glass_material(layer: &LayerEntry) -> GlassMaterial {
    let is_pure_lens = layer.optics.refraction.as_ref().map_or(false, |r| r.is_pure_lens)
        || layer.liquid_material_hint.as_ref().map_or(false, |h| h.role.as_deref() == Some("pure_refraction_lens"));

    let mut mat = if is_pure_lens {
        let mut m = GlassMaterial::clear();
        m.blur.radius = 0.0;
        m.blur.edge_blur = false;
        m.dispersion.strength = 0.0;
        m.tint = liquid_rs::scene::Color::rgba(1.0, 1.0, 1.0, 0.0);
        m
    } else if layer.optics.translucency > 0.15 {
        let mut m = GlassMaterial::clear();
        m.blur.radius = 4.0;
        m.blur.edge_blur = false;
        m
    } else {
        GlassMaterial::regular()
    };

    if let Some(ref r) = layer.optics.refraction {
        mat.refraction.strength = (r.strength * 15.0).clamp(0.1, 1.0);
        mat.refraction.thickness = (r.height * 5.0).clamp(0.05, 0.8);
    }

    if layer.optics.specular {
        mat.fresnel.strength = 0.45;
        mat.fresnel.hardness = 0.65;
        mat.glare = GlareStyle::system();
    }

    if let Some(ref sh) = layer.optics.shadow {
        mat.shadow.factor = (sh.opacity * 0.4).clamp(0.1, 0.6);
        mat.shadow.expand = if sh.style >= 2 { 24.0 } else { 16.0 };
        mat.shadow.offset = [0.0, 2.5];
    }

    mat
}

/// 3-pass separable sliding-window box blur on alpha channel approximating a true Gaussian blur (CLT).
/// Eliminates Mach bands, linear corners, and banding artifacts.
fn blur_pixmap_alpha(pixmap: &mut Pixmap, radius: usize) {
    if radius == 0 {
        return;
    }
    let w = pixmap.width() as usize;
    let h = pixmap.height() as usize;

    let mut front: Vec<u8> = pixmap.pixels().iter().map(|p| p.alpha()).collect();
    let mut back = vec![0u8; w * h];
    let r = radius.max(1);

    for _ in 0..3 {
        // Horizontal pass: front -> back
        let div = (2 * r + 1) as u32;
        for y in 0..h {
            let row = y * w;
            let mut sum = (r as u32) * (front[row] as u32);
            for i in 0..=r {
                sum += front[row + i.min(w - 1)] as u32;
            }
            for x in 0..w {
                back[row + x] = (sum / div) as u8;
                let add_idx = (x + r + 1).min(w - 1);
                let rem_idx = x.saturating_sub(r);
                sum += front[row + add_idx] as u32;
                sum -= front[row + rem_idx] as u32;
            }
        }

        // Vertical pass: back -> front
        for x in 0..w {
            let mut sum = (r as u32) * (back[x] as u32);
            for j in 0..=r {
                sum += back[j.min(h - 1) * w + x] as u32;
            }
            for y in 0..h {
                front[y * w + x] = (sum / div) as u8;
                let add_idx = (y + r + 1).min(h - 1);
                let rem_idx = y.saturating_sub(r);
                sum += back[add_idx * w + x] as u32;
                sum -= back[rem_idx * w + x] as u32;
            }
        }
    }

    let pixels = pixmap.pixels_mut();
    for i in 0..(w * h) {
        let a = front[i];
        pixels[i] = tiny_skia::PremultipliedColorU8::from_rgba(0, 0, 0, a).unwrap_or(tiny_skia::PremultipliedColorU8::TRANSPARENT);
    }
}

/// Fast two-pass separable box blur on premultiplied RGBA channels for authentic frosted glass backdrop blur.
fn blur_pixmap_rgba(pixmap: &mut Pixmap, radius: usize) {
    if radius == 0 {
        return;
    }
    let w = pixmap.width() as usize;
    let h = pixmap.height() as usize;
    let pixels = pixmap.pixels_mut();

    let mut temp_r = vec![0u8; w * h];
    let mut temp_g = vec![0u8; w * h];
    let mut temp_b = vec![0u8; w * h];
    let mut temp_a = vec![0u8; w * h];

    // Horizontal pass
    for y in 0..h {
        let row_start = y * w;
        for x in 0..w {
            let left = x.saturating_sub(radius);
            let right = (x + radius).min(w - 1);
            let mut sr = 0u32;
            let mut sg = 0u32;
            let mut sb = 0u32;
            let mut sa = 0u32;
            let mut count = 0u32;
            for k in left..=right {
                let p = pixels[row_start + k];
                sr += p.red() as u32;
                sg += p.green() as u32;
                sb += p.blue() as u32;
                sa += p.alpha() as u32;
                count += 1;
            }
            let idx = row_start + x;
            temp_r[idx] = (sr / count) as u8;
            temp_g[idx] = (sg / count) as u8;
            temp_b[idx] = (sb / count) as u8;
            temp_a[idx] = (sa / count) as u8;
        }
    }

    // Vertical pass
    for x in 0..w {
        for y in 0..h {
            let top = y.saturating_sub(radius);
            let bot = (y + radius).min(h - 1);
            let mut sr = 0u32;
            let mut sg = 0u32;
            let mut sb = 0u32;
            let mut sa = 0u32;
            let mut count = 0u32;
            for k in top..=bot {
                let idx = k * w + x;
                sr += temp_r[idx] as u32;
                sg += temp_g[idx] as u32;
                sb += temp_b[idx] as u32;
                sa += temp_a[idx] as u32;
                count += 1;
            }
            let r = (sr / count) as u8;
            let g = (sg / count) as u8;
            let b = (sb / count) as u8;
            let a = (sa / count) as u8;
            pixels[y * w + x] = tiny_skia::PremultipliedColorU8::from_rgba(r, g, b, a).unwrap();
        }
    }
}

/// Samples a Pixmap with bilinear filtering and bounds clamping.
fn sample_bilinear(pixmap: &Pixmap, x: f32, y: f32) -> tiny_skia::PremultipliedColorU8 {
    let w = pixmap.width() as f32;
    let h = pixmap.height() as f32;
    let cx = x.clamp(0.0, w - 1.001);
    let cy = y.clamp(0.0, h - 1.001);
    let x0 = cx.floor() as usize;
    let y0 = cy.floor() as usize;
    let x1 = (x0 + 1).min(pixmap.width() as usize - 1);
    let y1 = (y0 + 1).min(pixmap.height() as usize - 1);

    let fx = cx - cx.floor();
    let fy = cy - cy.floor();

    let p00 = pixmap.pixel(x0 as u32, y0 as u32).unwrap();
    let p10 = pixmap.pixel(x1 as u32, y0 as u32).unwrap();
    let p01 = pixmap.pixel(x0 as u32, y1 as u32).unwrap();
    let p11 = pixmap.pixel(x1 as u32, y1 as u32).unwrap();

    let interp = |c00: u8, c10: u8, c01: u8, c11: u8| -> u8 {
        let top = (c00 as f32) * (1.0 - fx) + (c10 as f32) * fx;
        let bot = (c01 as f32) * (1.0 - fx) + (c11 as f32) * fx;
        (top * (1.0 - fy) + bot * fy).round().clamp(0.0, 255.0) as u8
    };

    tiny_skia::PremultipliedColorU8::from_rgba(
        interp(p00.red(), p10.red(), p01.red(), p11.red()),
        interp(p00.green(), p10.green(), p01.green(), p11.green()),
        interp(p00.blue(), p10.blue(), p01.blue(), p11.blue()),
        interp(p00.alpha(), p10.alpha(), p01.alpha(), p11.alpha()),
    ).unwrap_or(p00)
}

/// Applies physical liquid glass optics pass on top of a rendered layer pixmap:
/// - Soft cast drop shadows
/// - Pure refractive convex lens magnification
/// - Fresnel edge highlight rim
/// - Top-down specular glare and subtle surface dispersion
// macOS Unified Directional Key Light: from Top-Left (-0.6, -0.8), unit vector
const LIGHT_KEY_X: f32 = -0.60;
const LIGHT_KEY_Y: f32 = -0.80;

/// Applies physical liquid glass optics pass on top of a rendered layer pixmap:
/// - Directional Key Light Bevel Highlights (Top-Left rim)
/// - Backlight Bevel Occlusion / Depth Crease (Bottom-Right rim)
/// - Pure Refractive Crystal Convex Lens (snell warp, zero blur contamination)
/// - Soft directional drop shadows for volumetric extrusion
fn apply_liquid_glass_optics(
    dest_canvas: &mut Pixmap,
    fg_coverage: &mut Pixmap,
    layer_pixmap: &Pixmap,
    layer: &LayerEntry,
    material: &GlassMaterial,
    width: u32,
    height: u32,
    app_dir: &Path,
) {
    let blend = parse_blend_mode(&layer.blend_mode);
    let w = width as usize;
    let h = height as usize;

    let is_spherical_lens = (layer.canon_id.as_deref() == Some("circle") || layer.name.contains("circle") || layer.name.contains("lens"))
        && (layer.optics.refraction.as_ref().map_or(false, |r| r.is_pure_lens)
            || layer.liquid_material_hint.as_ref().map_or(false, |h| h.role.as_deref() == Some("pure_refraction_lens")));

    let is_convex_puck = (layer.canon_id.as_deref() == Some("white_puck") || layer.name.contains("puck"))
        && (layer.optics.refraction.is_some() || layer.liquid_material_hint.as_ref().map_or(false, |h| h.role.as_deref() == Some("convex_puck_lens")));

    // 1. Directional Cast Drop Shadow Pass (provides volumetric height / Z-elevation)
    let shadow_style = layer.optics.shadow.as_ref().map_or(0, |s| s.style);
    let is_minor_detail = layer.name.contains("detail") || layer.name.contains("head") || layer.name.contains("tics");
    let should_cast_shadow = !is_spherical_lens
        && !is_convex_puck
        && !is_minor_detail
        && shadow_style > 0
        && ((material.shadow.factor > 0.01)
            || (layer.optics.translucency > 0.25)
            || (layer.optics.refraction.is_some())
            || (layer.optics.specular && layer.layer_type != "background_plate" && shadow_style >= 2));

    if should_cast_shadow {
        let sh_opacity = layer.optics.shadow.as_ref().map_or(0.55, |s| s.opacity);
        // Directional shadow offset matching macOS key light from top-left (-0.6, -0.8):
        // Casts shadow towards bottom-right (+dx, +dy)
        let scale = (width as f32) / 1024.0;
        let ox = ((if shadow_style >= 3 { 8.0 } else if shadow_style >= 2 { 5.0 } else { 3.0 }) * scale).round() as i32;
        let oy = ((if shadow_style >= 3 { 14.0 } else if shadow_style >= 2 { 10.0 } else { 6.0 }) * scale).round() as i32;
        let mut shadow_pixmap = Pixmap::new(width, height).unwrap();

        let src_pixels = layer_pixmap.pixels();
        let sh_pixels = shadow_pixmap.pixels_mut();

        for y in 0..h {
            for x in 0..w {
                let p = src_pixels[y * w + x];
                if p.alpha() > 16 {
                    let dy = y as i32 + oy;
                    let dx = x as i32 + ox;
                    if dy >= 0 && dy < h as i32 && dx >= 0 && dx < w as i32 {
                        let target_idx = (dy as usize) * w + (dx as usize);
                        let a = p.alpha();
                        if a > sh_pixels[target_idx].alpha() {
                            sh_pixels[target_idx] = tiny_skia::PremultipliedColorU8::from_rgba(0, 0, 0, a).unwrap();
                        }
                    }
                }
            }
        }

        // 3-pass Gaussian blur for authentic soft ambient drop
        let blur_radius = ((if shadow_style >= 3 { 16.0 } else if shadow_style >= 2 { 11.0 } else { 7.0 }) * scale).round() as usize;
        blur_pixmap_alpha(&mut shadow_pixmap, blur_radius);

        // Strictly occlude shadow underneath the layer's own footprint (standard drop-shadow occlusion)
        // so that translucent surfaces do not get dirty/darkened from behind by their own cast shadow.
        let sh_pixels = shadow_pixmap.pixels_mut();
        for y in 0..h {
            for x in 0..w {
                let target_idx = y * w + x;
                let raw_a = src_pixels[target_idx].alpha() as f32;
                if raw_a > 0.0 {
                    let norm_a = (raw_a / (layer.opacity * 255.0).max(1.0)).min(1.0);
                    let cur_a = sh_pixels[target_idx].alpha() as f32;
                    let atten_a = (cur_a * (1.0 - norm_a)).round() as u8;
                    sh_pixels[target_idx] = tiny_skia::PremultipliedColorU8::from_rgba(0, 0, 0, atten_a).unwrap_or(tiny_skia::PremultipliedColorU8::TRANSPARENT);
                }
            }
        }

        let mut shadow_paint = PixmapPaint::default();
        let base_mult = if layer.name.contains("blue") || layer.name.contains("puck") { 0.65 } else { 0.38 };
        shadow_paint.opacity = (sh_opacity * base_mult).clamp(0.10, 0.55);
        dest_canvas.draw_pixmap(0, 0, shadow_pixmap.as_ref(), &shadow_paint, Transform::identity(), None);
    }

    if is_spherical_lens {
        // Pure Refractive Inlaid Clear Lens & Inset Crater Optics Pass:
        // Zero blur contamination, authentic Snell magnification, physically accurate inset crater bevel:
        // - Top-left inner shadow falling down into the crater
        // - Bottom-right inner wall specular reflection catching direct light
        // - Top-left delicate outer lip crest hairline
        // - Smooth edge vignette roll-off
        let _refr_strength = layer.optics.refraction.as_ref().map_or(0.86, |r| r.strength);
        let src_pixels = layer_pixmap.pixels();

        let mut sum_x = 0.0f64;
        let mut sum_y = 0.0f64;
        let mut sum_w = 0.0f64;
        for y in 0..h {
            for x in 0..w {
                let a = src_pixels[y * w + x].alpha() as f64;
                if a > 32.0 {
                    sum_x += (x as f64) * a;
                    sum_y += (y as f64) * a;
                    sum_w += a;
                }
            }
        }

        if sum_w > 100.0 {
            let cx = (sum_x / sum_w) as f32;
            let cy = (sum_y / sum_w) as f32;
            let radius = ((sum_w / 255.0) / std::f64::consts::PI).sqrt() as f32;

            let backdrop = dest_canvas.to_owned();
            let dest_pixels = dest_canvas.pixels_mut();

            // 1. Ray Refraction with Snell magnification & inward crater shading
            for y in 0..h {
                for x in 0..w {
                    let mask_alpha = src_pixels[y * w + x].alpha() as f32 / 255.0;
                    if mask_alpha <= 0.005 {
                        continue;
                    }

                    let dx = x as f32 - cx;
                    let dy = y as f32 - cy;
                    let dist = (dx * dx + dy * dy).sqrt();
                    let rho = dist / radius;

                    if rho <= 1.025 {
                        let nx = dx / dist.max(0.001);
                        let ny = dy / dist.max(0.001);

                        // 1. liquid-rs physical Snell refraction ray displacement
                        let refr_strength = layer.optics.refraction.as_ref().map_or(0.86, |r| r.strength);
                        let refr_height = layer.optics.refraction.as_ref().map_or(0.26, |r| r.height);
                        let ref_index = 1.50f32; // optical crown glass

                        let inside_dist = (radius - dist).max(0.0);
                        let refr_thickness = radius * refr_height;

                        let mut sample_x = x as f32;
                        let mut sample_y = y as f32;

                        if inside_dist < refr_thickness && dist > 0.001 {
                            let incidence_ratio = 1.0 - inside_dist / refr_thickness;
                            let theta_i = (incidence_ratio.powi(2)).clamp(0.0, 0.9999).asin();
                            let theta_t = (1.0 / ref_index * theta_i.sin()).clamp(0.0, 0.9999).asin();
                            let edge_factor = -(theta_t - theta_i).tan();

                            // liquid-rs refOffset: moves backdrop sample along optical normal towards center
                            let offset = edge_factor * refr_strength * radius * 0.06;
                            sample_x = x as f32 - nx * offset;
                            sample_y = y as f32 - ny * offset;
                        }

                        let mut refr_col = sample_bilinear(&backdrop, sample_x, sample_y);

                        // 2. liquid-rs edge glass absorption (Fresnel vignette)
                        if (0.88..=0.99).contains(&rho) {
                            let edge_t = ((rho - 0.88) / 0.11).clamp(0.0, 1.0);
                            let glass_absorption = edge_t * edge_t * 0.055;
                            let r = ((refr_col.red() as f32) * (1.0 - glass_absorption)).round() as u8;
                            let g = ((refr_col.green() as f32) * (1.0 - glass_absorption)).round() as u8;
                            let b = ((refr_col.blue() as f32) * (1.0 - glass_absorption)).round() as u8;
                            refr_col = tiny_skia::PremultipliedColorU8::from_rgba(r, g, b, refr_col.alpha()).unwrap_or(refr_col);
                        }

                        // 3. Inset Crater Ambient Occlusion & Subtle Bevel Shadow:
                        // Surface normal pointing towards top-left key light (-0.60, -0.80)
                        let light_dot = (nx * LIGHT_KEY_X + ny * LIGHT_KEY_Y).max(0.0);
                        if (0.90..=0.995).contains(&rho) && light_dot > 0.02 {
                            let d = (rho - 0.960) / 0.030;
                            let shadow_curve = (-0.5 * d * d).exp();
                            let shadow_factor = shadow_curve * light_dot.powf(1.0) * 0.035;
                            let r = ((refr_col.red() as f32) * (1.0 - shadow_factor)).round() as u8;
                            let g = ((refr_col.green() as f32) * (1.0 - shadow_factor)).round() as u8;
                            let b = ((refr_col.blue() as f32) * (1.0 - shadow_factor)).round() as u8;
                            refr_col = tiny_skia::PremultipliedColorU8::from_rgba(r, g, b, refr_col.alpha()).unwrap_or(refr_col);
                        }

                        // Blend into canvas with antialiased edge
                        let edge_f = ((1.015 - rho) / 0.025).clamp(0.0, 1.0) * mask_alpha;
                        let target_idx = y * w + x;
                        let orig_col = dest_pixels[target_idx];

                        let r = (orig_col.red() as f32 * (1.0 - edge_f) + refr_col.red() as f32 * edge_f).round() as u8;
                        let g = (orig_col.green() as f32 * (1.0 - edge_f) + refr_col.green() as f32 * edge_f).round() as u8;
                        let b = (orig_col.blue() as f32 * (1.0 - edge_f) + refr_col.blue() as f32 * edge_f).round() as u8;
                        let a = (orig_col.alpha() as f32 * (1.0 - edge_f) + refr_col.alpha() as f32 * edge_f).round() as u8;

                        dest_pixels[target_idx] = tiny_skia::PremultipliedColorU8::from_rgba(r, g, b, a).unwrap_or(orig_col);
                    }
                }
            }

            // 2. Specular Pass:
            // - Delicate inward crater wall specular at bottom-right [0.955, 0.995]
            // - Delicate outer lip crest hairline at top-left [0.985, 1.018]
            let mut rim_pixmap = Pixmap::new(width, height).unwrap();
            let rim_pixels = rim_pixmap.pixels_mut();
            for y in 0..h {
                for x in 0..w {
                    let dx = x as f32 - cx;
                    let dy = y as f32 - cy;
                    let dist = (dx * dx + dy * dy).sqrt();
                    let rho = dist / radius;

                    let nx = dx / dist.max(0.001);
                    let ny = dy / dist.max(0.001);
                    let opp_dot = (nx * LIGHT_KEY_X + ny * LIGHT_KEY_Y).max(0.0);

                    let mut spec_intensity = 0.0f32;

                    // Top/top-left outer lip crest highlight (centered towards key light from top/top-left)
                    // In Apple's reference, the lip crest is brightest at the top (dx ~ 0, dy < 0)
                    let crest_dot = (nx * (-0.25) + ny * (-0.968)).max(0.0);
                    if (0.960..=1.010).contains(&rho) && crest_dot > 0.05 {
                        let crest_d = (rho - 0.985) / 0.014;
                        let crest_curve = (-0.5 * crest_d * crest_d).exp();
                        spec_intensity += crest_curve * crest_dot.powf(1.5) * 55.0;
                    }

                    // Bottom-right inner crater wall: very faint ambient bounce matching Apple's subtle tone
                    if (0.965..=0.995).contains(&rho) && opp_dot > 0.10 {
                        let wall_d = (rho - 0.980) / 0.010;
                        let wall_curve = (-0.5 * wall_d * wall_d).exp();
                        spec_intensity += wall_curve * opp_dot.powf(2.2) * 22.0;
                    }

                    let out_a = spec_intensity.clamp(0.0, 255.0) as u8;
                    if out_a > 1 {
                        rim_pixels[y * w + x] = tiny_skia::PremultipliedColorU8::from_rgba(out_a, out_a, out_a, out_a).unwrap();
                    }
                }
            }

            let mut rim_paint = PixmapPaint::default();
            rim_paint.blend_mode = BlendMode::Screen;
            rim_paint.opacity = 0.85;
            dest_canvas.draw_pixmap(0, 0, rim_pixmap.as_ref(), &rim_paint, Transform::identity(), None);
        }
    } else if is_convex_puck {
        // Physical Toroidal / Beveled Convex Puck Lens Optics Pass (e.g. Maps puck navigation lens ring):
        // Outer beveled ring with authentic outward Snell refraction displacement, crest arc glare, and contact drop shadow
        let refr_strength = layer.optics.refraction.as_ref().map_or(0.56, |r| r.strength);
        let src_pixels = layer_pixmap.pixels();

        let mut sum_x = 0.0f64;
        let mut sum_y = 0.0f64;
        let mut sum_w = 0.0f64;
        for y in 0..h {
            for x in 0..w {
                let a = src_pixels[y * w + x].alpha() as f64;
                if a > 32.0 {
                    sum_x += (x as f64) * a;
                    sum_y += (y as f64) * a;
                    sum_w += a;
                }
            }
        }

        if sum_w > 100.0 {
            let cx = (sum_x / sum_w) as f32;
            let cy = (sum_y / sum_w) as f32;
            let radius = ((sum_w / 255.0) / std::f64::consts::PI).sqrt() as f32;

            // 1. High-Fidelity Local Contact Drop Shadow for Puck
            let scale = (width as f32) / 1024.0;
            let oy = (14.0 * scale).round() as i32;
            let mut shadow_pixmap = Pixmap::new(width, height).unwrap();
            let sh_pixels = shadow_pixmap.pixels_mut();
            for y in 0..h {
                for x in 0..w {
                    let p = src_pixels[y * w + x];
                    if p.alpha() > 16 {
                        let dy = y as i32 + oy;
                        if dy >= 0 && dy < h as i32 {
                            let target_idx = (dy as usize) * w + x;
                            if p.alpha() > sh_pixels[target_idx].alpha() {
                                sh_pixels[target_idx] = PremultipliedColorU8::from_rgba(0, 0, 0, p.alpha()).unwrap();
                            }
                        }
                    }
                }
            }
            blur_pixmap_alpha(&mut shadow_pixmap, (20.0 * scale).round() as usize);
            let sh_pixels = shadow_pixmap.pixels_mut();
            for y in 0..h {
                for x in 0..w {
                    let target_idx = y * w + x;
                    if src_pixels[target_idx].alpha() > 200 {
                        sh_pixels[target_idx] = PremultipliedColorU8::TRANSPARENT;
                    }
                }
            }
            let mut sh_paint = PixmapPaint::default();
            sh_paint.opacity = 0.46;
            dest_canvas.draw_pixmap(0, 0, shadow_pixmap.as_ref(), &sh_paint, Transform::identity(), None);

            // 2. Toroidal Convex Bevel Refraction Pass
            let backdrop = dest_canvas.to_owned();
            let dest_pixels = dest_canvas.pixels_mut();

            let r_in = radius * 0.7733; // 232.0 / 300.0 inner core radius
            let ring_width = radius - r_in;

            for y in 0..h {
                for x in 0..w {
                    let mask_alpha = src_pixels[y * w + x].alpha() as f32 / 255.0;
                    if mask_alpha <= 0.005 {
                        continue;
                    }

                    let dx = x as f32 - cx;
                    let dy = y as f32 - cy;
                    let dist = (dx * dx + dy * dy).sqrt();

                    if dist <= radius + 2.0 {
                        let mut sample_x = x as f32;
                        let mut sample_y = y as f32;

                        if dist >= r_in && dist <= radius + 2.0 {
                            let u = ((dist - r_in) / ring_width).clamp(0.0, 1.0);
                            let nx = dx / dist.max(0.001);
                            let ny = dy / dist.max(0.001);

                            // Authentic convex lens displacement: peaks at ~0.46 of the ring width with deeper inner penetration
                            let hump = u.powf(0.85) * (1.0 - u).powf(0.95) * 2.65;
                            let offset = hump * refr_strength * (radius * 0.22);

                            // Snell refraction on convex bevel bends light outward towards surface normal
                            sample_x = x as f32 + nx * offset;
                            sample_y = y as f32 + ny * offset;
                        }

                        let refr_col = sample_bilinear(&backdrop, sample_x, sample_y);

                        // Apple's authentic translucent white puck gradient (Gradient-2: Color-8 (0.78) to Color-9 (0.69), layer opacity 0.40)
                        let y_rel = ((y as f32 - (cy - radius)) / (2.0 * radius)).clamp(0.0, 1.0);
                        let base_white_alpha = 0.40 * (0.78 * (1.0 - y_rel) + 0.69 * y_rel);
                        let u_edge = ((dist - r_in) / 6.0).clamp(0.0, 1.0) * ((radius - dist) / 3.0).clamp(0.0, 1.0);
                        let white_tint = (base_white_alpha * u_edge).clamp(0.0, 0.40);

                        let r = (refr_col.red() as f32 * (1.0 - white_tint) + 255.0 * white_tint).round() as u8;
                        let g = (refr_col.green() as f32 * (1.0 - white_tint) + 255.0 * white_tint).round() as u8;
                        let b = (refr_col.blue() as f32 * (1.0 - white_tint) + 255.0 * white_tint).round() as u8;

                        // Antialiased outer boundary feathering
                        let edge_f = ((radius + 1.5 - dist) / 2.5).clamp(0.0, 1.0) * mask_alpha;
                        let target_idx = y * w + x;
                        let orig_col = dest_pixels[target_idx];

                        let out_r = (orig_col.red() as f32 * (1.0 - edge_f) + r as f32 * edge_f).round() as u8;
                        let out_g = (orig_col.green() as f32 * (1.0 - edge_f) + g as f32 * edge_f).round() as u8;
                        let out_b = (orig_col.blue() as f32 * (1.0 - edge_f) + b as f32 * edge_f).round() as u8;
                        let out_a = (orig_col.alpha() as f32 * (1.0 - edge_f) + 255.0 * edge_f).round() as u8;

                        dest_pixels[target_idx] = PremultipliedColorU8::from_rgba(out_r, out_g, out_b, out_a).unwrap_or(orig_col);
                    }
                }
            }

            // 3. Specular Glare & Crest Highlight Pass
            // Procedural Crest Glare & Fresnel Outer Rim
            let mut spec_pixmap = Pixmap::new(width, height).unwrap();
            let spec_pixels = spec_pixmap.pixels_mut();

            for y in 0..h {
                for x in 0..w {
                    let dx = x as f32 - cx;
                    let dy = y as f32 - cy;
                    let dist = (dx * dx + dy * dy).sqrt();

                    if dist >= r_in - 5.0 && dist <= radius + 5.0 {
                        let nx = dx / dist.max(0.001);
                        let ny = dy / dist.max(0.001);
                        let mut intensity = 0.0f32;

                        // A. Top-Key Crest Arc Highlight (spanning 11 to 1 o'clock)
                        let dot_top = (nx * (-0.15) + ny * (-0.989)).max(0.0);
                        if dist >= r_in && dist <= radius && dot_top > 0.10 {
                            let r_crest = r_in + ring_width * 0.48; // ~265px
                            let d = (dist - r_crest) / 6.2;
                            let crest_factor = (-0.5 * d * d).exp();
                            intensity += crest_factor * dot_top.powf(1.8) * 310.0;
                        }

                        // B. Upper-Right Specular Glare Blob (around 1:30 o'clock, angle -50 deg over green land)
                        let angle = dy.atan2(dx); // [-PI, PI]
                        let target_ang = -50.0f32.to_radians(); // 1:30 direction
                        let mut ang_diff = (angle - target_ang).abs();
                        if ang_diff > std::f32::consts::PI {
                            ang_diff = 2.0 * std::f32::consts::PI - ang_diff;
                        }
                        if ang_diff < 0.40 && dist >= r_in && dist <= radius {
                            let r_blob = r_in + ring_width * 0.52; // ~267px
                            let dr = (dist - r_blob) / 9.0;
                            let da = ang_diff / 0.14;
                            let blob_factor = (-0.5 * (dr * dr + da * da)).exp();
                            intensity += blob_factor * 290.0;
                        }

                        // B2. Lower-Right Specular Glare Blob (around 4:30 o'clock, angle +50 deg over yellow land)
                        let target_ang_lr = 50.0f32.to_radians();
                        let mut ang_diff_lr = (angle - target_ang_lr).abs();
                        if ang_diff_lr > std::f32::consts::PI {
                            ang_diff_lr = 2.0 * std::f32::consts::PI - ang_diff_lr;
                        }
                        if ang_diff_lr < 0.35 && dist >= r_in && dist <= radius {
                            let r_blob = r_in + ring_width * 0.52;
                            let dr = (dist - r_blob) / 8.0;
                            let da = ang_diff_lr / 0.12;
                            let blob_factor = (-0.5 * (dr * dr + da * da)).exp();
                            intensity += blob_factor * 220.0;
                        }

                        // C. Ultra-fine Sharp Fresnel Outer Edge Rim (upper half 9 to 3 o'clock and bottom 6 o'clock)
                        if dist >= radius - 3.8 && dist <= radius + 1.8 {
                            let d_edge = (dist - (radius - 1.2)) / 1.0;
                            let rim_factor = (-0.5 * d_edge * d_edge).exp();
                            let upper_arc = (-ny).max(0.0); // upper half
                            let bottom_arc = (ny * 1.5 - 0.5).max(0.0); // bottom 6 o'clock
                            let rim_mask = (upper_arc * 1.25 + bottom_arc * 0.95).min(1.0);
                            intensity += rim_factor * rim_mask * 250.0;
                        }

                        // D. Inner contact boundary rim with blue puck (especially visible at bottom-right 4 to 6 o'clock)
                        if dist >= r_in - 2.0 && dist <= r_in + 2.5 {
                            let d_in = (dist - r_in) / 1.1;
                            let in_factor = (-0.5 * d_in * d_in).exp();
                            let in_dot = (nx * 0.707 + ny * 0.707).max(0.0); // bottom-right inner bounce
                            intensity += in_factor * (35.0 + in_dot * 100.0);
                        }

                        let out_a = intensity.clamp(0.0, 255.0) as u8;
                        if out_a > 1 {
                            spec_pixels[y * w + x] = PremultipliedColorU8::from_rgba(out_a, out_a, out_a, out_a).unwrap();
                        }
                    }
                }
            }

            let mut spec_paint = PixmapPaint::default();
            spec_paint.blend_mode = BlendMode::Screen;
            spec_paint.opacity = 0.90;
            dest_canvas.draw_pixmap(0, 0, spec_pixmap.as_ref(), &spec_paint, Transform::identity(), None);

            // Record foreground coverage
            let mut cov_paint = PixmapPaint::default();
            cov_paint.blend_mode = BlendMode::SourceOver;
            fg_coverage.draw_pixmap(0, 0, layer_pixmap.as_ref(), &cov_paint, Transform::identity(), None);
        }
    } else {
        // 2. Frosted Backdrop Blur Pass (for frosted acrylic / glass surfaces like Shortcuts, Weather cloud, Stickies)
        let is_refraction_helper = layer.name.ends_with(".refraction");
        let is_frosted = !is_refraction_helper && (
            layer.optics.blur_strength > 0.02
            || layer.liquid_material_hint.as_ref().map_or(false, |h| h.variant.as_deref() == Some("Frosted"))
        );

        if is_frosted {
            let blur_r = if layer.optics.blur_strength > 0.02 {
                (layer.optics.blur_strength * 26.0).clamp(4.0, 30.0).round() as usize
            } else if let Some(ref r) = layer.optics.refraction {
                (r.strength * 20.0).clamp(5.0, 24.0).round() as usize
            } else {
                (layer.optics.translucency * 16.0).clamp(3.0, 20.0).round() as usize
            };

            let mut blurred_backdrop = dest_canvas.to_owned();
            blur_pixmap_rgba(&mut blurred_backdrop, blur_r);

            let src_pixels = layer_pixmap.pixels();
            let blur_pixels = blurred_backdrop.pixels();
            let dest_pixels = dest_canvas.pixels_mut();

            let blur_blend_factor = if layer.optics.blur_strength > 0.05 {
                0.92f32
            } else {
                0.80f32
            };

            for y in 0..h {
                for x in 0..w {
                    let mask_a = src_pixels[y * w + x].alpha() as f32 / 255.0;
                    if mask_a > 0.01 {
                        let target_idx = y * w + x;
                        let cur = dest_pixels[target_idx];
                        let b = blur_pixels[target_idx];

                        // Blend frosted backdrop blur under the frosted surface
                        let f = mask_a * blur_blend_factor;
                        if f > 0.005 {
                            let r = (cur.red() as f32 * (1.0 - f) + b.red() as f32 * f).round() as u8;
                            let g = (cur.green() as f32 * (1.0 - f) + b.green() as f32 * f).round() as u8;
                            let bl = (cur.blue() as f32 * (1.0 - f) + b.blue() as f32 * f).round() as u8;
                            dest_pixels[target_idx] = tiny_skia::PremultipliedColorU8::from_rgba(r, g, bl, cur.alpha()).unwrap_or(cur);
                        }
                    }
                }
            }
        }

        // 3. Base Content Blit with authentic layer opacity and blend mode
        let (actual_blend, actual_opacity) = if is_frosted && (layer.optics.translucency > 0.05) {
            let opac = if layer.blend_mode.to_lowercase() == "lighten" {
                layer.opacity.min(0.85)
            } else {
                (layer.opacity * (1.0 - layer.optics.translucency * 0.35)).clamp(0.35, 0.95)
            };
            (blend, opac)
        } else {
            (blend, layer.opacity)
        };

        let mut layer_paint = PixmapPaint::default();
        layer_paint.blend_mode = actual_blend;
        layer_paint.opacity = actual_opacity;
        dest_canvas.draw_pixmap(0, 0, layer_pixmap.as_ref(), &layer_paint, Transform::identity(), None);

        // Record foreground coverage for subsequent layers
        if layer.layer_type != "background_plate" && !is_refraction_helper {
            let mut cov_paint = PixmapPaint::default();
            cov_paint.blend_mode = BlendMode::SourceOver;
            fg_coverage.draw_pixmap(0, 0, layer_pixmap.as_ref(), &cov_paint, Transform::identity(), None);
        }

        // 4. Directional Key Light & Volumetric Bevel Pass
        // Injects physical bevel highlights on top-left facing edges and subtle bevel occlusion on bottom-right edges
        let is_foreground = layer.layer_type != "background_plate";
        let is_flat_floor = layer.canon_id.as_deref() == Some("circle") && !is_spherical_lens;
        let should_light = (layer.optics.specular || layer.optics.translucency > 0.01 || layer.optics.refraction.is_some() || is_foreground)
            && !layer.name.ends_with(".refraction")
            && !is_flat_floor;

        if should_light && material.fresnel.strength > 0.02 {
            let mut highlight_pixmap = Pixmap::new(width, height).unwrap();
            let mut occlusion_pixmap = Pixmap::new(width, height).unwrap();

            let src_pixels = layer_pixmap.pixels();
            let hi_pixels = highlight_pixmap.pixels_mut();
            let occ_pixels = occlusion_pixmap.pixels_mut();

            for y in 1..(h - 1) {
                for x in 1..(w - 1) {
                    let center_p = src_pixels[y * w + x];
                    let center_a = center_p.alpha() as f32;
                    if center_a < 12.0 {
                        continue;
                    }

                    let a_top = src_pixels[(y - 1) * w + x].alpha() as f32;
                    let a_bot = src_pixels[(y + 1) * w + x].alpha() as f32;
                    let a_left = src_pixels[y * w + (x - 1)].alpha() as f32;
                    let a_right = src_pixels[y * w + (x + 1)].alpha() as f32;

                    // Inward alpha gradient
                    let grad_x = (a_right - a_left) / 255.0;
                    let grad_y = (a_bot - a_top) / 255.0;
                    let edge_len = (grad_x * grad_x + grad_y * grad_y).sqrt();

                    if edge_len > 0.18 {
                        // Normal pointing towards light (outward normal is (-grad_x, -grad_y) / edge_len)
                        // Dot product with Key Light (LIGHT_KEY_X, LIGHT_KEY_Y):
                        let light_dot = (grad_x * (-LIGHT_KEY_X) + grad_y * (-LIGHT_KEY_Y)) / edge_len;

                        // A. Top-Left Facing Bevel Highlight (Creamy specular rim)
                        if light_dot > 0.04 {
                            let lit_factor = light_dot.powf(1.0);
                            let edge_val = (edge_len * lit_factor * 1.6).clamp(0.0, 1.0);
                            let out_a = (edge_val * (center_a / 255.0) * 220.0) as u8;
                            if out_a > 2 {
                                hi_pixels[y * w + x] = tiny_skia::PremultipliedColorU8::from_rgba(out_a, out_a, out_a, out_a).unwrap();
                            }
                        } else if is_frosted && edge_len > 0.35 {
                            // Omnidirectional subtle Fresnel edge rim for frosted acrylic
                            let out_a = (edge_len * 0.28 * 255.0).clamp(0.0, 70.0) as u8;
                            if out_a > hi_pixels[y * w + x].alpha() {
                                hi_pixels[y * w + x] = tiny_skia::PremultipliedColorU8::from_rgba(out_a, out_a, out_a, out_a).unwrap();
                            }
                        }

                        // B. Bottom-Right Facing Bevel Occlusion / Depth Crease
                        let occ_dot = -light_dot;
                        if occ_dot > 0.04 {
                            let occ_factor = occ_dot.powf(1.1);
                            let edge_val = (edge_len * occ_factor * 0.32).clamp(0.0, 1.0);
                            let out_a = (edge_val * (center_a / 255.0) * 110.0) as u8;
                            if out_a > 2 {
                                occ_pixels[y * w + x] = tiny_skia::PremultipliedColorU8::from_rgba(0, 0, 0, out_a).unwrap();
                            }
                        }
                    }
                }
            }

            // Composite Bevel Highlight (Screen)
            let mut hi_paint = PixmapPaint::default();
            hi_paint.blend_mode = BlendMode::Screen;
            hi_paint.opacity = 0.85;
            dest_canvas.draw_pixmap(0, 0, highlight_pixmap.as_ref(), &hi_paint, Transform::identity(), None);

            // Composite Bevel Occlusion (Multiply) for physical thickness / extrusion
            let mut occ_paint = PixmapPaint::default();
            occ_paint.blend_mode = BlendMode::Multiply;
            occ_paint.opacity = 0.40;
            dest_canvas.draw_pixmap(0, 0, occlusion_pixmap.as_ref(), &occ_paint, Transform::identity(), None);
        }
    }
}

/// Renders a complete layered application icon package.
fn render_layered_app_icon(
    app_dir: &Path,
    target_size: u32,
    output_png: &Path,
    _gpu_ctx: Option<&mut LiquidGpuContext>,
) -> Result<(), String> {
    let manifest_path = app_dir.join("manifest.json");
    if !manifest_path.is_file() {
        return Err(format!("Manifest not found: {}", manifest_path.display()));
    }

    let manifest_str = fs::read_to_string(&manifest_path)
        .map_err(|e| format!("Failed to read manifest: {}", e))?;
    let manifest: AppManifest = serde_json::from_str(&manifest_str)
        .map_err(|e| format!("Failed to parse manifest JSON: {}", e))?;

    // Prefer Aqua or DarkAqua appearance
    let layers = manifest
        .appearances
        .get("NSAppearanceNameAqua")
        .or_else(|| manifest.appearances.get("UIAppearanceLight"))
        .or_else(|| manifest.appearances.get("NSAppearanceNameDarkAqua"))
        .or_else(|| manifest.appearances.get("UIAppearanceDark"))
        .or_else(|| manifest.appearances.values().next())
        .ok_or_else(|| "No appearance layers found in manifest".to_string())?;

    let mut canvas = Pixmap::new(target_size, target_size)
        .ok_or("Failed to allocate destination canvas")?;
    let mut fg_coverage = Pixmap::new(target_size, target_size)
        .ok_or("Failed to allocate foreground coverage map")?;

    println!(
        "Rendering [{}] (slug: {}, has_squircle: {}, layers: {})",
        manifest.title,
        manifest.slug,
        manifest.has_squircle,
        layers.len()
    );

    // Render layers in Z-order sequence
    for (idx, layer) in layers.iter().enumerate() {
        if layer.opacity <= 0.001 || layer.name.ends_with(".refraction") {
            continue;
        }

        let svg_file = app_dir.join(&layer.file);
        if !svg_file.is_file() {
            println!("  [Layer {}] Missing file: {}, skipping", idx, layer.file);
            continue;
        }

        let layer_pixmap = render_svg_file(&svg_file, target_size, target_size)?;
        let material = build_liquid_glass_material(layer);

        let is_glass = layer.optics.refraction.is_some()
            || layer.optics.translucency > 0.01
            || layer.optics.specular
            || layer.optics.shadow.is_some();

        if is_glass {
            let _is_pure_lens = layer.optics.refraction.as_ref().map_or(false, |r| r.is_pure_lens)
                || layer.liquid_material_hint.as_ref().map_or(false, |h| h.role.as_deref() == Some("pure_refraction_lens"));

            println!(
                "  [Layer {}] Optically active physical layer: '{}' (translucency: {:.2}, specular: {}, refraction: {:?})",
                idx,
                layer.name,
                layer.optics.translucency,
                layer.optics.specular,
                layer.optics.refraction.as_ref().map(|r| r.strength)
            );
            apply_liquid_glass_optics(&mut canvas, &mut fg_coverage, &layer_pixmap, layer, &material, target_size, target_size, app_dir);
        } else {
            println!("  [Layer {}] Standard layer: '{}'", idx, layer.name);
            let mut paint = PixmapPaint::default();
            paint.blend_mode = parse_blend_mode(&layer.blend_mode);
            paint.opacity = layer.opacity;
            canvas.draw_pixmap(0, 0, layer_pixmap.as_ref(), &paint, Transform::identity(), None);
            if layer.layer_type != "background_plate" {
                fg_coverage.draw_pixmap(0, 0, layer_pixmap.as_ref(), &paint, Transform::identity(), None);
            }
        }
    }

    // Clip whole icon to continuous Apple Squircle contour if application has squircle plate,
    // and render authentic macOS Squircle Contact + Ambient Grounding Shadows!
    if manifest.has_squircle {
        const APPLE_SQUIRCLE_PATH: &str =
            "M 100 285 C 100 165 165 100 285 100 \
             L 739 100 C 859 100 924 165 924 285 \
             L 924 739 C 924 859 859 924 739 924 \
             L 285 924 C 165 924 100 859 100 739 Z";

        let mask_svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024" width="{}" height="{}"><path d="{}" fill="black"/></svg>"#,
            target_size, target_size, APPLE_SQUIRCLE_PATH
        );
        if let Ok(mask_pixmap) = render_svg_str(&mask_svg, target_size, target_size) {
            let mut squircle_content = canvas.to_owned();
            let mut mask_paint = PixmapPaint::default();
            mask_paint.blend_mode = BlendMode::DestinationIn;
            squircle_content.draw_pixmap(0, 0, mask_pixmap.as_ref(), &mask_paint, Transform::identity(), None);

            let s = target_size as f32 / 1024.0;

            // 1. Ambient Grounding Shadow (large radius, subtle spread)
            let mut amb_pixmap = mask_pixmap.to_owned();
            let amb_r = (24.0 * s).round() as usize;
            blur_pixmap_alpha(&mut amb_pixmap, amb_r);

            // 2. Contact Grounding Shadow (smaller radius, darker contact)
            let mut con_pixmap = mask_pixmap.to_owned();
            let con_r = (8.0 * s).round() as usize;
            blur_pixmap_alpha(&mut con_pixmap, con_r);

            let amb_dy = (12.0 * s).round() as i32;
            let con_dy = (8.0 * s).round() as i32;

            canvas.fill(tiny_skia::Color::TRANSPARENT);

            let mut amb_paint = PixmapPaint::default();
            amb_paint.opacity = 0.12;
            canvas.draw_pixmap(0, amb_dy, amb_pixmap.as_ref(), &amb_paint, Transform::identity(), None);

            let mut con_paint = PixmapPaint::default();
            con_paint.opacity = 0.12;
            canvas.draw_pixmap(0, con_dy, con_pixmap.as_ref(), &con_paint, Transform::identity(), None);

            let copy_paint = PixmapPaint::default();
            canvas.draw_pixmap(0, 0, squircle_content.as_ref(), &copy_paint, Transform::identity(), None);
        }
    }

    if let Some(parent) = output_png.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    canvas
        .save_png(output_png)
        .map_err(|e| format!("Failed to encode PNG {}: {}", output_png.display(), e))?;

    println!("  -> Saved render to: {}\n", output_png.display());
    Ok(())
}

fn main() {
    println!("=== squircle-icon-rs: Liquid Glass Layered Icon Renderer Demo ===");

    let base_layers_dir = if Path::new("applications/layers").is_dir() {
        PathBuf::from("applications/layers")
    } else if Path::new("../macos-icons/applications/layers").is_dir() {
        PathBuf::from("../macos-icons/applications/layers")
    } else {
        PathBuf::from("/Users/sail/Workspace/macos-icons/applications/layers")
    };

    let output_dir = if Path::new("Cargo.toml").is_file()
        && fs::read_to_string("Cargo.toml").map(|s| s.contains("squircle-icon-rs")).unwrap_or(false)
    {
        PathBuf::from("output/liquid_demo")
    } else {
        PathBuf::from("../squircle-icon-rs/output/liquid_demo")
    };

    let cli_args: Vec<String> = std::env::args().skip(1).filter(|a| !a.starts_with('-')).collect();

    // Discover targets: either CLI args or all dirs in base_layers_dir with manifest.json
    let mut slugs: Vec<String> = Vec::new();
    if !cli_args.is_empty() {
        slugs = cli_args;
    } else if let Ok(entries) = fs::read_dir(&base_layers_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && path.join("manifest.json").is_file() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    slugs.push(name.to_string());
                }
            }
        }
        slugs.sort();
    }

    println!("Found {} target application packages in {}\n", slugs.len(), base_layers_dir.display());

    let start_total = Instant::now();
    let target_size = 1024;
    let mut success_count = 0;

    let mut gpu_ctx = match LiquidGpuContext::new(target_size, target_size) {
        Ok(ctx) => {
            println!("[liquid-rs GPU] Successfully initialized Metal/WGPU headless rendering engine!\n");
            Some(ctx)
        }
        Err(e) => {
            eprintln!("[liquid-rs GPU warning] Headless GPU init failed ({:?}), using CPU fallback pipeline\n", e);
            None
        }
    };

    for slug in &slugs {
        println!("------------------------------------------------------------");
        println!("Target App: {}", slug);
        let app_dir = base_layers_dir.join(slug);
        let out_png = output_dir.join(format!("{}.png", slug));

        let t0 = Instant::now();
        match render_layered_app_icon(&app_dir, target_size, &out_png, gpu_ctx.as_mut()) {
            Ok(()) => {
                success_count += 1;
                println!("Completed in {:.2}ms", t0.elapsed().as_secs_f64() * 1000.0);
            }
            Err(e) => {
                eprintln!("Error rendering {}: {}", slug, e);
            }
        }
    }

    println!("============================================================");
    println!(
        "Rendered {}/{} icons successfully in {:.2}s.",
        success_count,
        slugs.len(),
        start_total.elapsed().as_secs_f64()
    );
    println!("Output directory: {}", output_dir.display());
}
