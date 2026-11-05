//! User settings (persisted as JSON in the app data dir) and default folders.

use crate::appdata::AppData;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Extensions we index. Matching is case-insensitive.
pub const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp", "gif", "bmp"];
/// Recognised but not decodable in this build (see DECISIONS.md). Counted as skipped.
pub const UNSUPPORTED_EXTENSIONS: &[&str] = &["heic", "heif", "avif"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FolderKind {
    Screenshots,
    Pictures,
    Downloads,
    Desktop,
    Custom,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Folder {
    pub path: PathBuf,
    pub kind: FolderKind,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub folders: Vec<Folder>,
    /// Files smaller than this are skipped (tiny icons, tracking pixels).
    pub min_file_bytes: u64,
    /// Files larger than this are skipped.
    pub max_file_bytes: u64,
    /// Images whose shorter side is below this many pixels are skipped (icons).
    pub min_dimension: u32,
    /// Decompression-bomb guard: images with more pixels than this are skipped.
    pub max_pixels: u64,
    /// Global hotkey for the quick search bar.
    pub hotkey: String,
    /// Compute image embeddings ("meaning search"). Requires the CLIP model.
    pub semantic_enabled: bool,
    /// Indexing paused by the user (persists across restarts).
    pub paused: bool,
    /// Number of indexing worker threads; 0 = automatic.
    pub workers: usize,
    /// User accepted the one-time model download.
    pub models_consented: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            folders: default_folders(),
            min_file_bytes: 2 * 1024,
            max_file_bytes: 40 * 1024 * 1024,
            min_dimension: 64,
            max_pixels: 80_000_000,
            hotkey: "Alt+Shift+S".into(),
            semantic_enabled: true,
            paused: false,
            workers: 0,
            models_consented: false,
        }
    }
}

impl Settings {
    pub fn load(appdata: &AppData) -> Settings {
        let path = appdata.settings_path();
        match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
                tracing::warn!("settings.json unreadable ({e}); using defaults");
                Settings::default()
            }),
            Err(_) => Settings::default(),
        }
    }

    pub fn save(&self, appdata: &AppData) -> Result<()> {
        appdata.write_atomic(&appdata.settings_path(), &serde_json::to_vec_pretty(self)?)
    }

    pub fn enabled_folders(&self) -> impl Iterator<Item = &Folder> {
        self.folders.iter().filter(|f| f.enabled)
    }

    pub fn effective_workers(&self) -> usize {
        if self.workers > 0 {
            return self.workers;
        }
        let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2);
        // Background app: leave most of the machine to the user.
        (cores / 4).clamp(1, 3)
    }

    pub fn is_screenshot_folder(&self, p: &Path) -> bool {
        self.folders.iter().any(|f| f.kind == FolderKind::Screenshots && p.starts_with(&f.path))
    }
}

pub fn has_image_extension(p: &Path) -> bool {
    ext_lower(p).is_some_and(|e| IMAGE_EXTENSIONS.contains(&e.as_str()))
}

pub fn has_unsupported_extension(p: &Path) -> bool {
    ext_lower(p).is_some_and(|e| UNSUPPORTED_EXTENSIONS.contains(&e.as_str()))
}

fn ext_lower(p: &Path) -> Option<String> {
    p.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase())
}

/// OS-specific default folders: screenshots, Pictures, Downloads.
pub fn default_folders() -> Vec<Folder> {
    let mut out: Vec<Folder> = Vec::new();
    let mut push = |path: PathBuf, kind: FolderKind| {
        if path.as_os_str().is_empty() || out.iter().any(|f| f.path == path) {
            return;
        }
        out.push(Folder { path, kind, enabled: true });
    };
    for p in screenshot_dirs() {
        push(p, FolderKind::Screenshots);
    }
    if let Some(p) = dirs::picture_dir() {
        push(p, FolderKind::Pictures);
    }
    if let Some(p) = dirs::download_dir() {
        push(p, FolderKind::Downloads);
    }
    out
}

/// Where the OS (and common tools) save screenshots. Only existing dirs are returned.
pub fn screenshot_dirs() -> Vec<PathBuf> {
    let mut c: Vec<PathBuf> = Vec::new();
    let pics = dirs::picture_dir();
    let home = dirs::home_dir();
    #[cfg(windows)]
    {
        // Win+PrtScn and the Snipping Tool (Windows 11) save here.
        if let Some(p) = &pics {
            c.push(p.join("Screenshots"));
        }
        // OneDrive "save screenshots to OneDrive".
        if let Ok(od) = std::env::var("OneDrive") {
            c.push(PathBuf::from(&od).join("Pictures").join("Screenshots"));
            c.push(PathBuf::from(&od).join("Screenshots"));
        }
        if let Some(v) = dirs::video_dir() {
            c.push(v.join("Captures")); // Xbox Game Bar
        }
    }
    #[cfg(target_os = "macos")]
    {
        // macOS saves to the Desktop unless `defaults write com.apple.screencapture location` is set.
        if let Some(loc) = macos_screencapture_location() {
            c.push(loc);
        }
        if let Some(d) = dirs::desktop_dir() {
            c.push(d);
        }
        if let Some(p) = &pics {
            c.push(p.join("Screenshots"));
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Some(p) = &pics {
            c.push(p.join("Screenshots")); // GNOME, KDE Spectacle
        }
        if let Some(h) = &home {
            c.push(h.join("Screenshots"));
        }
    }
    let _ = (&pics, &home);
    c.retain(|p| p.is_dir());
    c.dedup();
    c
}

#[cfg(target_os = "macos")]
fn macos_screencapture_location() -> Option<PathBuf> {
    let out =
        std::process::Command::new("defaults").args(["read", "com.apple.screencapture", "location"]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let p = if let Some(rest) = s.strip_prefix("~/") { dirs::home_dir()?.join(rest) } else { PathBuf::from(s) };
    p.is_dir().then_some(p)
}

/// Heuristic: does this file name look like a screenshot? (Folder membership is
/// checked separately.) Covers the default names on Windows, macOS, Linux,
/// Android, iOS exports and common tools, in several languages.
pub fn looks_like_screenshot_name(name: &str) -> bool {
    let n = name.to_lowercase();
    const NEEDLES: &[&str] = &[
        "screenshot",
        "screen shot",
        "screen_shot",
        "screen-shot",
        "screencap",
        "screen capture",
        "scr_",
        "snip",
        "capture d",           // fr: "Capture d’écran"
        "captura de pantalla", // es
        "captura de tela",     // pt
        "bildschirmfoto",      // de
        "schermafbeelding",    // nl
        "schermata",           // it
        "zrzut ekranu",        // pl
        "снимок экрана",       // ru
        "скриншот",
        "スクリーンショット",
        "截图",
        "截屏",
        "屏幕截图",
        "스크린샷",
        "lightshot",
        "cleanshot",
        "flameshot",
        "greenshot",
        "sharex",
    ];
    NEEDLES.iter().any(|k| n.contains(k))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screenshot_names() {
        for n in [
            "Screenshot 2024-05-01 at 10.12.33.png",
            "Screenshot_20240501-101233_WhatsApp.jpg",
            "Screen Shot 2019-01-01 at 1.00.00 PM.png",
            "Bildschirmfoto vom 2024-01-01.png",
            "スクリーンショット 2024-01-01.png",
            "Capture d’écran 2024.png",
            "CleanShot 2024-01-01.png",
        ] {
            assert!(looks_like_screenshot_name(n), "{n}");
        }
        for n in ["IMG_1234.JPG", "holiday.png", "logo.webp"] {
            assert!(!looks_like_screenshot_name(n), "{n}");
        }
    }

    #[test]
    fn extensions() {
        assert!(has_image_extension(Path::new("a/B.PNG")));
        assert!(has_image_extension(Path::new("a/b.jpeg")));
        assert!(!has_image_extension(Path::new("a/b.txt")));
        assert!(!has_image_extension(Path::new("a/png")));
        assert!(has_unsupported_extension(Path::new("x.HEIC")));
    }

    #[test]
    fn settings_roundtrip_and_partial_json() {
        let dir = tempfile::tempdir().unwrap();
        let ad = AppData::open(dir.path()).unwrap();
        let s = Settings { hotkey: "Ctrl+Space".into(), ..Default::default() };
        s.save(&ad).unwrap();
        assert_eq!(Settings::load(&ad), s);
        // Missing fields fall back to defaults (forward compatibility).
        ad.write_atomic(&ad.settings_path(), br#"{"hotkey":"F1"}"#).unwrap();
        let s2 = Settings::load(&ad);
        assert_eq!(s2.hotkey, "F1");
        assert_eq!(s2.max_file_bytes, Settings::default().max_file_bytes);
    }
}
