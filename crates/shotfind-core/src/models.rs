//! Model manifest, installation state, and the one-time downloader.
//!
//! This is the only module that talks to the network, and only when the user
//! has agreed to the download. Every byte is verified against a pinned SHA-256
//! before it is used; files land in the app data `models/` dir.

use crate::appdata::AppData;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

const MANIFEST_JSON: &str = include_str!("../models.json");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "lowercase")]
pub enum ModelId {
    Ocr,
    Clip,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Extract {
    pub member: String,
    pub file: String,
    pub sha256: String,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Download {
    pub url: String,
    /// Target file name for direct downloads (absent for archives).
    #[serde(default)]
    pub file: Option<String>,
    pub sha256: String,
    pub size: u64,
    #[serde(default)]
    pub note: Option<String>,
    /// For archives: members to extract (everything else is discarded).
    #[serde(default)]
    pub extract: Vec<Extract>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelSpec {
    pub id: ModelId,
    pub dir: String,
    pub title: String,
    pub detail: String,
    pub license: String,
    pub homepage: String,
    pub required: bool,
    pub downloads: Vec<Download>,
}

impl ModelSpec {
    /// Bytes transferred over the network.
    pub fn download_size(&self) -> u64 {
        self.downloads.iter().map(|d| d.size).sum()
    }
    /// Bytes kept on disk once installed.
    pub fn disk_size(&self) -> u64 {
        self.downloads
            .iter()
            .map(|d| if d.extract.is_empty() { d.size } else { d.extract.iter().map(|e| e.size).sum() })
            .sum()
    }
    /// Final files and their expected hashes.
    pub fn files(&self) -> Vec<(String, String, u64)> {
        let mut out = Vec::new();
        for d in &self.downloads {
            if d.extract.is_empty() {
                out.push((d.file.clone().unwrap_or_default(), d.sha256.clone(), d.size));
            } else {
                for e in &d.extract {
                    out.push((e.file.clone(), e.sha256.clone(), e.size));
                }
            }
        }
        out
    }
    /// All hashes pinned (a download with an unpinned hash is refused).
    pub fn is_pinned(&self) -> bool {
        self.downloads.iter().all(|d| is_hex64(&d.sha256) && d.extract.iter().all(|e| is_hex64(&e.sha256)))
    }
}

fn is_hex64(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

#[derive(Deserialize)]
struct Manifest {
    models: Vec<ModelSpec>,
}

pub fn manifest() -> &'static [ModelSpec] {
    static M: OnceLock<Vec<ModelSpec>> = OnceLock::new();
    M.get_or_init(|| serde_json::from_str::<Manifest>(MANIFEST_JSON).expect("models.json is valid").models)
}

pub fn spec(id: ModelId) -> &'static ModelSpec {
    manifest().iter().find(|m| m.id == id).expect("model in manifest")
}

pub fn model_dir(models_dir: &Path, id: ModelId) -> PathBuf {
    models_dir.join(&spec(id).dir)
}

const MARKER: &str = "installed.json";

#[derive(Serialize, Deserialize)]
struct Marker {
    /// (file name, verified sha256, size on disk)
    files: Vec<(String, String, u64)>,
}

/// Installed = a marker written after verification, with matching file sizes.
/// (Hashes were checked at install time; re-hashing 150 MB on every start is avoided.)
pub fn is_installed(models_dir: &Path, id: ModelId) -> bool {
    let spec = spec(id);
    let dir = model_dir(models_dir, id);
    let Ok(bytes) = std::fs::read(dir.join(MARKER)) else { return false };
    let Ok(marker) = serde_json::from_slice::<Marker>(&bytes) else { return false };
    spec.files().iter().all(|(name, sha, _)| {
        marker.files.iter().any(|(n, s, size)| {
            n == name
                && s.eq_ignore_ascii_case(sha)
                && std::fs::metadata(dir.join(name)).map(|m| m.len() == *size).unwrap_or(false)
        })
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelStatus {
    pub id: ModelId,
    pub title: String,
    pub detail: String,
    pub license: String,
    pub homepage: String,
    pub required: bool,
    pub download_bytes: u64,
    pub disk_bytes: u64,
    pub installed: bool,
    pub pinned: bool,
    pub sources: Vec<String>,
}

pub fn statuses(models_dir: &Path) -> Vec<ModelStatus> {
    manifest()
        .iter()
        .map(|m| ModelStatus {
            id: m.id,
            title: m.title.clone(),
            detail: m.detail.clone(),
            license: m.license.clone(),
            homepage: m.homepage.clone(),
            required: m.required,
            download_bytes: m.download_size(),
            disk_bytes: m.disk_size(),
            installed: is_installed(models_dir, m.id),
            pinned: m.is_pinned(),
            sources: m.downloads.iter().map(|d| host_of(&d.url)).collect(),
        })
        .collect()
}

fn host_of(url: &str) -> String {
    url.split("://").nth(1).and_then(|r| r.split('/').next()).unwrap_or(url).to_string()
}

#[derive(Debug, Clone, Serialize)]
pub struct DownloadProgress {
    pub model: ModelId,
    pub file: String,
    pub downloaded: u64,
    pub total: u64,
    /// Overall across everything requested.
    pub overall_downloaded: u64,
    pub overall_total: u64,
    pub stage: &'static str,
}

/// Downloads, verifies and installs the given models. Resumes partial
/// downloads. Never runs without the caller having obtained consent.
#[cfg(feature = "download")]
pub fn install(
    appdata: &AppData,
    ids: &[ModelId],
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(&DownloadProgress),
) -> Result<()> {
    let models_dir = appdata.models_dir();
    let todo: Vec<&ModelSpec> = ids.iter().map(|&id| spec(id)).filter(|s| !is_installed(&models_dir, s.id)).collect();
    for s in &todo {
        if !s.is_pinned() {
            bail!(
                "{} has no pinned SHA-256 in models.json, so it will not be downloaded. \
                 Run `node scripts/pin-models.mjs` (needs internet) and rebuild.",
                s.title
            );
        }
    }
    let overall_total: u64 = todo.iter().map(|s| s.download_size()).sum();
    let mut overall_done = 0u64;
    let agent = ureq::Agent::new_with_config(
        ureq::config::Config::builder()
            .user_agent(concat!("shotfind/", env!("CARGO_PKG_VERSION"), " (model download)"))
            .timeout_connect(Some(std::time::Duration::from_secs(20)))
            // OS certificate store: works behind corporate TLS-inspecting proxies.
            .tls_config(ureq::tls::TlsConfig::builder().root_certs(ureq::tls::RootCerts::PlatformVerifier).build())
            .build(),
    );
    for s in todo {
        let dir = model_dir(&models_dir, s.id);
        let partial = models_dir.join(".partial");
        for d in &s.downloads {
            let name = d.file.clone().unwrap_or_else(|| file_name_of_url(&d.url));
            let part = partial.join(format!("{}-{}.part", s.dir, name));
            let base = overall_done;
            fetch_verified(appdata, &agent, &d.url, &d.sha256, &part, cancel, &mut |done, total| {
                progress(&DownloadProgress {
                    model: s.id,
                    file: name.clone(),
                    downloaded: done,
                    total,
                    overall_downloaded: base + done.min(d.size),
                    overall_total,
                    stage: "downloading",
                })
            })?;
            if d.extract.is_empty() {
                appdata.rename(&part, &dir.join(&name))?;
            } else {
                progress(&DownloadProgress {
                    model: s.id,
                    file: name.clone(),
                    downloaded: d.size,
                    total: d.size,
                    overall_downloaded: base + d.size,
                    overall_total,
                    stage: "extracting",
                });
                extract_members(appdata, &part, &d.extract, &dir)?;
                appdata.remove_file(&part)?;
            }
            overall_done += d.size;
        }
        write_marker(appdata, s, &dir)?;
    }
    Ok(())
}

#[cfg(feature = "download")]
fn fetch_verified(
    appdata: &AppData,
    agent: &ureq::Agent,
    url: &str,
    sha256: &str,
    part: &Path,
    cancel: &AtomicBool,
    on_progress: &mut dyn FnMut(u64, u64),
) -> Result<()> {
    if !url.starts_with("https://") {
        bail!("refusing non-HTTPS model URL {url}");
    }
    // Resume: hash what we already have, then ask for the rest.
    let mut hasher = Sha256::new();
    let mut have = 0u64;
    if let Ok(mut f) = std::fs::File::open(part) {
        let mut buf = vec![0u8; 1 << 16];
        loop {
            let n = f.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            have += n as u64;
        }
    }
    let mut req = agent.get(url);
    if have > 0 {
        req = req.header("Range", &format!("bytes={have}-"));
    }
    let resp = req.call().with_context(|| format!("downloading {url}"))?;
    let status = resp.status().as_u16();
    let mut out = if have > 0 && status == 206 {
        appdata.open_append(part)?
    } else {
        // Server ignored the range (or fresh start): start over.
        hasher = Sha256::new();
        have = 0;
        appdata.create_file(part)?
    };
    let total = resp
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .map(|n| n + have)
        .unwrap_or(0);
    let mut reader = resp.into_body().into_reader();
    let mut buf = vec![0u8; 1 << 16];
    let mut done = have;
    let mut last_tick = std::time::Instant::now();
    loop {
        if cancel.load(Ordering::Relaxed) {
            out.flush()?;
            bail!("download cancelled");
        }
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n])?;
        hasher.update(&buf[..n]);
        done += n as u64;
        if last_tick.elapsed().as_millis() > 100 {
            on_progress(done, total);
            last_tick = std::time::Instant::now();
        }
    }
    out.flush()?;
    out.sync_all().ok();
    drop(out);
    on_progress(done, total.max(done));
    let got = hex::encode(hasher.finalize());
    if !got.eq_ignore_ascii_case(sha256) {
        appdata.remove_file(part)?;
        bail!("SHA-256 mismatch for {url}: expected {sha256}, got {got}. The file was discarded.");
    }
    Ok(())
}

fn file_name_of_url(url: &str) -> String {
    url.rsplit('/').next().unwrap_or("download").split('?').next().unwrap_or("download").to_string()
}

fn extract_members(appdata: &AppData, archive: &Path, members: &[Extract], dir: &Path) -> Result<()> {
    let f = std::fs::File::open(archive)?;
    let mut zip = zip::ZipArchive::new(f).context("opening archive")?;
    for m in members {
        let mut entry = zip.by_name(&m.member).with_context(|| format!("{} missing from archive", m.member))?;
        let mut bytes = Vec::with_capacity(m.size as usize);
        entry.read_to_end(&mut bytes)?;
        let got = hex::encode(Sha256::digest(&bytes));
        if !got.eq_ignore_ascii_case(&m.sha256) {
            bail!("SHA-256 mismatch for {}: expected {}, got {got}", m.member, m.sha256);
        }
        appdata.write_atomic(&dir.join(&m.file), &bytes)?;
    }
    Ok(())
}

fn write_marker(appdata: &AppData, s: &ModelSpec, dir: &Path) -> Result<()> {
    let mut files = Vec::new();
    for (n, h, _) in s.files() {
        let size = std::fs::metadata(dir.join(&n)).with_context(|| format!("{n} missing after install"))?.len();
        files.push((n, h, size));
    }
    let marker = Marker { files };
    appdata.write_atomic(&dir.join(MARKER), &serde_json::to_vec_pretty(&marker)?)
}

/// Offline install: copies model files from a folder the user picked (e.g.
/// downloaded on another machine). Accepts the final names (`det.onnx`), the
/// upstream names (`vision_model_quantized.onnx`), or the archive itself.
/// Everything is hash-verified. Returns the models that became installed.
pub fn import_from_dir(appdata: &AppData, src: &Path) -> Result<Vec<ModelId>> {
    let models_dir = appdata.models_dir();
    let mut installed = Vec::new();
    let candidates: Vec<PathBuf> = walkdir::WalkDir::new(src)
        .max_depth(3)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_file())
        .map(|e| e.into_path())
        .collect();
    let find = |names: &[String], sha: &str| -> Option<PathBuf> {
        candidates
            .iter()
            .find(|p| {
                let n = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                names.contains(&n)
                    && is_hex64(sha)
                    && file_sha256(p).map(|h| h.eq_ignore_ascii_case(sha)).unwrap_or(false)
            })
            .cloned()
    };
    for s in manifest() {
        if is_installed(&models_dir, s.id) || !s.is_pinned() {
            continue;
        }
        let dir = model_dir(&models_dir, s.id);
        let mut ok = true;
        for d in &s.downloads {
            if d.extract.is_empty() {
                let name = d.file.clone().unwrap_or_default();
                match find(&[name.clone(), file_name_of_url(&d.url)], &d.sha256) {
                    Some(p) => appdata.write_atomic(&dir.join(&name), &std::fs::read(p)?)?,
                    None => ok = false,
                }
            } else if let Some(archive) = find(&[file_name_of_url(&d.url)], &d.sha256) {
                extract_members(appdata, &archive, &d.extract, &dir)?;
            } else {
                for e in &d.extract {
                    let upstream = e.member.rsplit('/').next().unwrap_or("").to_string();
                    match find(&[e.file.clone(), upstream], &e.sha256) {
                        Some(p) => appdata.write_atomic(&dir.join(&e.file), &std::fs::read(p)?)?,
                        None => ok = false,
                    }
                }
            }
        }
        if ok {
            write_marker(appdata, s, &dir)?;
            installed.push(s.id);
        }
    }
    Ok(installed)
}

pub fn file_sha256(p: &Path) -> Result<String> {
    let mut f = std::fs::File::open(p)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}

/// Removes an installed model.
pub fn uninstall(appdata: &AppData, id: ModelId) -> Result<()> {
    appdata.remove_dir_all(&model_dir(&appdata.models_dir(), id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_parses_and_is_https() {
        let m = manifest();
        assert_eq!(m.len(), 2);
        for s in m {
            for d in &s.downloads {
                assert!(d.url.starts_with("https://"), "{}", d.url);
                assert!(d.sha256.is_empty() || is_hex64(&d.sha256), "{}", d.sha256);
            }
        }
        assert!(spec(ModelId::Ocr).is_pinned());
        assert_eq!(spec(ModelId::Ocr).files().len(), 2);
    }

    #[test]
    fn import_verifies_hashes() {
        let dir = tempfile::tempdir().unwrap();
        let ad = AppData::open(dir.path().join("data")).unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        // Wrong content under the right names is rejected.
        std::fs::write(src.join("det.onnx"), b"not a model").unwrap();
        std::fs::write(src.join("rec.onnx"), b"not a model").unwrap();
        assert!(import_from_dir(&ad, &src).unwrap().is_empty());
        assert!(!is_installed(&ad.models_dir(), ModelId::Ocr));
    }
}
