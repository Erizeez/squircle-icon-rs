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

    // Flatpak exports roots (both user and system)
    if let Ok(home) = env::var("HOME") {
        roots.push(PathBuf::from(home).join(".local/share/flatpak/exports/share/icons"));
    }
    roots.push(PathBuf::from("/var/lib/flatpak/exports/share/icons"));

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
    if let Some(short_stem) = icon_name.rsplit('.').next() {
        if short_stem != icon_name && !short_stem.is_empty() {
            candidate_names.push(short_stem.to_string());
        }
    }

    let roots = icon_search_roots();
    let themes = ["hicolor", "Adwaita", "breeze", "gnome"];
    // Prefer scalable SVGs first, then descending raster resolutions (including @2 high-DPI)
    let sizes = [
        "scalable", "1024x1024", "512x512@2", "512x512", "256x256@2", "256x256",
        "128x128@2", "128x128", "96x96", "64x64", "48x48", "32x32",
    ];
    let categories = ["apps", "mimetypes", "categories", "status", "places"];
    let extensions = ["svg", "png", "webp", "xpm"];

    let has_graphic_ext = matches!(
        direct.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref(),
        Some("svg" | "png" | "webp" | "xpm" | "jpg" | "jpeg" | "ico")
    );

    // 2. Search pixmaps roots
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

    // 3. Search standard theme trees
    for name in &candidate_names {
        for theme in &themes {
            for root in &roots {
                if root.ends_with("pixmaps") {
                    continue;
                }
                let theme_root = root.join(theme);
                if !theme_root.is_dir() {
                    continue;
                }
                for size in &sizes {
                    for category in &categories {
                        let dir = theme_root.join(size).join(category);
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

    // 4. Fallback: if a short name was searched (e.g. "bottles"), scan for reverse-DNS matches (e.g. "*.bottles.svg")
    if !icon_name.contains('.') {
        let suffix_dot = format!(".{icon_name}");
        for root in &roots {
            if root.ends_with("pixmaps") {
                continue;
            }
            for theme in &themes {
                let theme_root = root.join(theme);
                if !theme_root.is_dir() {
                    continue;
                }
                for size in &sizes {
                    for category in &categories {
                        let dir = theme_root.join(size).join(category);
                        if let Ok(entries) = std::fs::read_dir(&dir) {
                            for entry in entries.flatten() {
                                let file_name = entry.file_name();
                                let name_str = file_name.to_string_lossy();
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
        let r2 = resolve_icon("com.usebottles.bottles");
        println!("resolve_icon('com.usebottles.bottles') -> {:?}", r2);
    }
}
