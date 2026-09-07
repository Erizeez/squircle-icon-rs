//! Apple macOS-style generic application fallback icon.
//!
//! Provides an exquisite, resolution-independent vector blueprint application
//! icon (drafting compass, ruler, and pencil forming the letter 'A' over an
//! architectural blueprint grid) for unknown or unbranded executables, conforming
//! strictly to Apple macOS Human Interface Guidelines and continuous curvature squircles.

use crate::plate::{apply_squircle_plate, PlateOptions};
use crate::raster::rasterize_svg_data;
use crate::IconBitmap;

/// Resolution-independent SVG markup for the Apple macOS-style blueprint application icon.
pub const FALLBACK_APP_ICON_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512" width="512" height="512">
  <defs>
    <linearGradient id="bgGrad" x1="0%" y1="0%" x2="0%" y2="100%">
      <stop offset="0%" stop-color="#217bfe"/>
      <stop offset="100%" stop-color="#084ec4"/>
    </linearGradient>
    <linearGradient id="rulerGrad" x1="0%" y1="0%" x2="100%" y2="100%">
      <stop offset="0%" stop-color="#f3f4f6"/>
      <stop offset="100%" stop-color="#d1d5db"/>
    </linearGradient>
    <linearGradient id="woodGrad" x1="0%" y1="0%" x2="100%" y2="100%">
      <stop offset="0%" stop-color="#fbbf24"/>
      <stop offset="100%" stop-color="#d97706"/>
    </linearGradient>
    <filter id="dropShadow" x="-10%" y="-10%" width="120%" height="120%">
      <feDropShadow dx="0" dy="12" stdDeviation="16" flood-color="#000" flood-opacity="0.35"/>
    </filter>
  </defs>
  
  <!-- Background Blueprint Card -->
  <rect width="512" height="512" rx="115" fill="url(#bgGrad)"/>
  
  <!-- Subtle Blueprint Architectural Grid Lines -->
  <g stroke="#ffffff" stroke-opacity="0.18" stroke-width="1.5" fill="none">
    <circle cx="256" cy="256" r="180" stroke-dasharray="6 6"/>
    <circle cx="256" cy="256" r="120" stroke-dasharray="4 4"/>
    <circle cx="256" cy="256" r="60" stroke-dasharray="3 3"/>
    <line x1="76" y1="256" x2="436" y2="256"/>
    <line x1="256" y1="76" x2="256" y2="436"/>
    <line x1="128" y1="128" x2="384" y2="384" stroke-dasharray="4 4"/>
    <line x1="384" y1="128" x2="128" y2="384" stroke-dasharray="4 4"/>
    <path d="M100 130 h30 M100 130 v30 M412 130 h-30 M412 130 v30 M100 382 h30 M100 382 v-30 M412 382 h-30 M412 382 v-30"/>
  </g>
  
  <!-- The Iconic macOS Drafting Tools Trio (Letter A) -->
  <g filter="url(#dropShadow)">
    <!-- Ruler (Left Leg of A) -->
    <g transform="translate(256,256) rotate(-32) translate(-256,-256)">
      <rect x="232" y="100" width="48" height="312" rx="8" fill="url(#rulerGrad)"/>
      <!-- Measurement ticks -->
      <g stroke="#6b7280" stroke-width="2.5">
        <line x1="232" y1="130" x2="252" y2="130"/>
        <line x1="232" y1="150" x2="244" y2="150"/>
        <line x1="232" y1="170" x2="244" y2="170"/>
        <line x1="232" y1="190" x2="252" y2="190"/>
        <line x1="232" y1="210" x2="244" y2="210"/>
        <line x1="232" y1="230" x2="244" y2="230"/>
        <line x1="232" y1="250" x2="252" y2="250"/>
        <line x1="232" y1="270" x2="244" y2="270"/>
        <line x1="232" y1="290" x2="244" y2="290"/>
        <line x1="232" y1="310" x2="252" y2="310"/>
        <line x1="232" y1="330" x2="244" y2="330"/>
        <line x1="232" y1="350" x2="244" y2="350"/>
        <line x1="232" y1="370" x2="252" y2="370"/>
      </g>
    </g>

    <!-- Pencil (Right Leg of A) -->
    <g transform="translate(256,256) rotate(32) translate(-256,-256)">
      <!-- Pencil Body -->
      <rect x="234" y="130" width="44" height="240" rx="4" fill="url(#woodGrad)"/>
      <path d="M234 130 h44 v-20 c0 -6 -4 -10 -10 -10 h-24 c-6 0 -10 4 -10 10 v20 z" fill="#f43f5e"/>
      <rect x="234" y="120" width="44" height="14" fill="#9ca3af"/>
      <!-- Sharpened Lead Tip -->
      <polygon points="234,370 278,370 256,415" fill="#fed7aa"/>
      <polygon points="248,400 264,400 256,415" fill="#1f2937"/>
    </g>

    <!-- Drafting Caliper / Crossbar (Horizontal bar of A) -->
    <g>
      <rect x="165" y="276" width="182" height="24" rx="6" fill="#e5e7eb"/>
      <circle cx="185" cy="288" r="5" fill="#6b7280"/>
      <circle cx="327" cy="288" r="5" fill="#6b7280"/>
      <line x1="200" y1="288" x2="312" y2="288" stroke="#9ca3af" stroke-width="3" stroke-linecap="round"/>
    </g>
  </g>
</svg>"##;

/// Renders the Apple macOS-style blueprint generic application fallback icon.
pub fn render_fallback_icon(size: u32, options: PlateOptions) -> IconBitmap {
    let pixmap = rasterize_svg_data(FALLBACK_APP_ICON_SVG.as_bytes(), size, size)
        .expect("rasterize fallback app icon svg");
    let framed = apply_squircle_plate(&pixmap, options);
    IconBitmap::from_pixmap(framed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_fallback_icon() {
        let bitmap = render_fallback_icon(128, PlateOptions::default());
        assert_eq!(bitmap.width(), 128);
        assert_eq!(bitmap.height(), 128);
        assert!(bitmap.to_png_bytes().is_ok());
    }
}
