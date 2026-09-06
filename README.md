# squircle-icon-rs

Apple macOS HIG-compliant application icon lookup, squircle clipping, intelligent matting, and multi-format rasterizer in pure Rust.

## Features

- **Freedesktop XDG Icon Discovery**: Automatically resolves application icon names across user and system icon themes (`hicolor`, `Adwaita`, `breeze`, `gnome`), prioritizing scalable SVGs before descending through raster resolutions.
- **Intelligent Apple Matting**: Automatically detects whether an icon is a circular cutout or freeform silhouette (such as Moonlight, Steam, GIMP), framing it onto an Apple continuous curvature ($G^2$) squircle background plate with a subtle vertical gradient and hairline inner border.
- **Continuous Curvature Squircle Clipping**: Full-bleed application icons are neatly clipped using continuous curvature ($G^2$) superellipses via `squircle-rs`.
- **Multi-Format Pipeline**:
  - Vector SVG rendering via `resvg` + `usvg` with 100% authentic color fidelity (no single-color tint crushing).
  - Raster decoding (PNG, JPEG, WebP) via `image`.
  - High-performance memory caching for instantaneous lookup and rendering.
  - Multi-target outputs: raw straight RGBA8, premultiplied RGBA8 (`tiny_skia::Pixmap`), PNG bytes, Data URI, and SVG markups.

## Usage

```rust
use squircle_icon_rs::{render_icon, render_svg_markup, PlateOptions, PlateTheme};

// Render an icon by name or file path
let icon = render_icon("moonlight", 128, PlateOptions::default())
    .expect("icon rendered");

// Access raw RGBA8 pixels for GPU textures (e.g. WGPU / Smithay)
let rgba = icon.to_straight_rgba();

// Or get an SVG markup string embedding the framed icon (for iced or web)
let svg_markup = render_svg_markup("org.mozilla.firefox", 64, PlateOptions::default());
```

## License

MIT
