use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

static LOOKUP_CACHE: OnceLock<Mutex<HashMap<String, Option<PathBuf>>>> = OnceLock::new();

/// Resolves standard XDG data search directories for application icons.
pub fn icon_search_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();

    if let Ok(home) = env::var("HOME") {
        roots.push(PathBuf::from(&home).join(".icons"));
    }

    if let Ok(data_home) = env::var("XDG_DATA_HOME") {
        roots.push(PathBuf::from(data_home).join("icons"));
    } else if let Ok(home) = env::var("HOME") {
        roots.push(PathBuf::from(home).join(".local/share/icons"));
    }

    if let Ok(data_dirs) = env::var("XDG_DATA_DIRS") {
        for dir in data_dirs.split(':') {
            if !dir.trim().is_empty() {
                roots.push(PathBuf::from(dir).join("icons"));
            }
        }
    } else {
        roots.push(PathBuf::from("/usr/local/share/icons"));
        roots.push(PathBuf::from("/usr/share/icons"));
    }

    // Flatpak exports and appstream roots (both user and system)
    if let Ok(home) = env::var("HOME") {
        roots.push(PathBuf::from(&home).join(".local/share/flatpak/exports/share/icons"));
        roots.push(PathBuf::from(home).join(".local/share/flatpak/appstream/flathub/x86_64/active/icons"));
    }
    roots.push(PathBuf::from("/var/lib/flatpak/exports/share/icons"));
    roots.push(PathBuf::from("/var/lib/flatpak/appstream/flathub/x86_64/active/icons"));

    // Pixmaps fallback roots
    roots.push(PathBuf::from("/usr/share/pixmaps"));
    if let Ok(home) = env::var("HOME") {
        roots.push(PathBuf::from(home).join(".local/share/pixmaps"));
    }

    roots.retain(|p| p.is_dir());
    let mut deduped = Vec::new();
    for root in roots {
        if !deduped.contains(&root) {
            deduped.push(root);
        }
    }
    deduped
}

/// Resolves an application icon name to a concrete on-disk SVG or raster asset path.
pub fn resolve_icon(icon_name: &str) -> Option<PathBuf> {
    let trimmed = icon_name.trim();
    if trimmed.is_empty() {
        return None;
    }

    let cache = LOOKUP_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(guard) = cache.lock()
        && let Some(cached) = guard.get(trimmed)
    {
        return cached.clone();
    }

    let resolved = resolve_icon_uncached(trimmed);
    if let Ok(mut guard) = cache.lock() {
        guard.insert(trimmed.to_string(), resolved.clone());
    }
    resolved
}

fn resolve_icon_uncached(icon_name: &str) -> Option<PathBuf> {
    // 1. Direct path check
    let direct = Path::new(icon_name);
    if direct.is_file() {
        return Some(direct.to_path_buf());
    }

    // Determine candidate names (e.g. "com.usebottles.bottles" -> ["com.usebottles.bottles", "bottles"])
    let mut candidate_names = vec![icon_name.to_string()];
    if let Some(short_stem) = icon_name.rsplit('.').next()
        && short_stem != icon_name
        && !short_stem.is_empty()
    {
        candidate_names.push(short_stem.to_string());
    }
    let lower = icon_name.to_lowercase();
    if lower != icon_name && !candidate_names.contains(&lower) {
        candidate_names.push(lower);
    }
    let mut chars = icon_name.chars();
    if let Some(first) = chars.next() {
        let capitalized = format!("{}{}", first.to_uppercase(), chars.as_str());
        if capitalized != icon_name && !candidate_names.contains(&capitalized) {
            candidate_names.push(capitalized);
        }
    }

    let roots = icon_search_roots();
    let themes = [
        "hicolor",
        "breeze",
        "AdwaitaLegacy",
        "Adwaita",
        "breeze-dark",
        "gnome",
    ];
    // Prefer scalable SVGs first, then descending raster resolutions (including @2 high-DPI and KDE numeric sizes)
    let sizes = [
        "scalable", "1024x1024", "512x512@2", "512x512", "256x256@2", "256x256",
        "128x128@2", "128x128", "96x96", "64x64", "48x48", "32x32", "24x24", "22x22", "16x16",
        "64", "48", "32", "24", "22", "16", "symbolic",
    ];
    let categories = [
        "apps", "devices", "preferences", "categories", "status",
        "mimetypes", "places", "legacy", "actions", "system",
    ];
    let extensions = ["svg", "png", "webp", "xpm"];

    let has_graphic_ext = matches!(
        direct.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref(),
        Some("svg" | "png" | "webp" | "xpm" | "jpg" | "jpeg" | "ico")
    );

    // Helper closure to search standard theme trees for a set of names.
    // Prioritizes highest resolution sizes (scalable, 1024, 512, 256) first across themes and roots.
    let search_theme_trees = |names: &[String]| -> Option<PathBuf> {
        for name in names {
            for theme in &themes {
                for size in &sizes {
                    for root in &roots {
                        if root.ends_with("pixmaps") {
                            continue;
                        }
                        let theme_root = root.join(theme);
                        if !theme_root.is_dir() {
                            continue;
                        }
                        for category in &categories {
                            // Support both GNOME (<size>/<category>) and KDE (<category>/<size>)
                            for dir in [
                                theme_root.join(size).join(category),
                                theme_root.join(category).join(size),
                            ] {
                                let candidate = dir.join(name);
                                if candidate.is_file() {
                                    return Some(candidate);
                                }
                                if !has_graphic_ext {
                                    for ext in extensions {
                                        let candidate = dir.join(format!("{name}.{ext}"));
                                        if candidate.is_file() {
                                            return Some(candidate);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        None
    };

    // 2. Search standard theme trees for exact candidate names
    if let Some(path) = search_theme_trees(&candidate_names) {
        return Some(path);
    }

    // 3. Direct size roots (e.g. Flatpak appstream <root>/<size>/<name>.<ext>)
    for size in &sizes {
        for root in &roots {
            if root.ends_with("pixmaps") {
                continue;
            }
            let dir = root.join(size);
            if !dir.is_dir() {
                continue;
            }
            for name in &candidate_names {
                let candidate = dir.join(name);
                if candidate.is_file() {
                    return Some(candidate);
                }
                if !has_graphic_ext {
                    for ext in extensions {
                        let candidate = dir.join(format!("{name}.{ext}"));
                        if candidate.is_file() {
                            return Some(candidate);
                        }
                    }
                }
            }
        }
    }

    // 4. Fallback: if a short name was searched (e.g. "bottles" or "apifox"), scan for reverse-DNS matches (e.g. "*.bottles.svg")
    if !icon_name.contains('.') {
        for name in &candidate_names {
            let suffix_dot = format!(".{name}").to_lowercase();
            // Search theme trees
            for theme in &themes {
                for size in &sizes {
                    for root in &roots {
                        if root.ends_with("pixmaps") {
                            continue;
                        }
                        let theme_root = root.join(theme);
                        if !theme_root.is_dir() {
                            continue;
                        }
                        for category in &categories {
                            for dir in [
                                theme_root.join(size).join(category),
                                theme_root.join(category).join(size),
                            ] {
                                if let Ok(entries) = std::fs::read_dir(&dir) {
                                    for entry in entries.flatten() {
                                        let file_name = entry.file_name();
                                        let name_str = file_name.to_string_lossy().to_lowercase();
                                        for ext in extensions {
                                            let target_suffix = format!("{suffix_dot}.{ext}");
                                            if name_str.ends_with(&target_suffix) {
                                                return Some(entry.path());
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            // Also search direct size roots (Flatpak appstream)
            for size in &sizes {
                for root in &roots {
                    if root.ends_with("pixmaps") {
                        continue;
                    }
                    let dir = root.join(size);
                    if let Ok(entries) = std::fs::read_dir(&dir) {
                        for entry in entries.flatten() {
                            let file_name = entry.file_name();
                            let name_str = file_name.to_string_lossy().to_lowercase();
                            for ext in extensions {
                                let target_suffix = format!("{suffix_dot}.{ext}");
                                if name_str.ends_with(&target_suffix) {
                                    return Some(entry.path());
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // 5. Fallback to symbolic names (e.g. "network-wired" -> "network-wired-symbolic")
    let symbolic_names: Vec<String> = candidate_names
        .iter()
        .filter(|n| !n.ends_with("-symbolic"))
        .map(|n| format!("{n}-symbolic"))
        .collect();
    if !symbolic_names.is_empty()
        && let Some(path) = search_theme_trees(&symbolic_names)
    {
        return Some(path);
    }

    // 6. Search pixmaps roots (fallback of last resort per XDG Icon Theme Spec)
    for root in &roots {
        if root.ends_with("pixmaps") {
            for name in &candidate_names {
                let candidate = root.join(name);
                if candidate.is_file() {
                    return Some(candidate);
                }
                if !has_graphic_ext {
                    for ext in extensions {
                        let candidate = root.join(format!("{name}.{ext}"));
                        if candidate.is_file() {
                            return Some(candidate);
                        }
                    }
                }
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_icon_search_roots_not_empty() {
        let roots = icon_search_roots();
        assert!(!roots.is_empty(), "Should discover at least one system icon directory");
    }

    #[test]
    fn test_resolve_moonlight_or_common_icon() {
        let resolved = resolve_icon("moonlight");
        if Path::new("/usr/share/icons/hicolor/scalable/apps/moonlight.svg").is_file() {
            assert!(resolved.is_some());
            assert!(resolved.unwrap().to_str().unwrap().contains("moonlight"));
        }
    }

    #[test]
    fn test_resolve_bottles() {
        println!("icon_search_roots: {:?}", icon_search_roots());
        let r1 = resolve_icon("bottles");
        println!("resolve_icon('bottles') -> {:?}", r1);
        println!("resolve_icon('com.usebottles.bottles') -> {:?}", resolve_icon("com.usebottles.bottles"));
        println!("resolve_icon('Alacritty') -> {:?}", resolve_icon("Alacritty"));
        println!("resolve_icon('alacritty') -> {:?}", resolve_icon("alacritty"));
        println!("resolve_icon('steam') -> {:?}", resolve_icon("steam"));
        assert!(resolve_icon("Alacritty").is_some());
        assert!(resolve_icon("alacritty").is_some());
    }

    #[test]
    fn test_resolve_network_wired_and_devices() {
        let r = resolve_icon("network-wired");
        println!("resolve_icon('network-wired') -> {:?}", r);
        assert!(r.is_some(), "network-wired should be resolved");
        let path = r.unwrap();
        assert!(path.is_file());
    }
}
