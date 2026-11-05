//! The never-modify guarantee: indexing, watching, re-indexing, wiping and
//! removing folders must leave every user file byte-for-byte identical, with
//! the same mtime and permissions, and must not create anything in user folders.
mod common;

use common::*;
use shotfind_core::appdata::AppData;
use shotfind_core::indexer::Indexer;
use std::time::Duration;

#[test]
fn user_files_are_never_modified() {
    let fx = Fixture::new();
    fx.standard_set();
    for i in 0..10 {
        write_png(&fx.user.join(format!("batch/{i}.png")), &synth_image(500, 300, &[&format!("row {i}")], "chart"));
    }
    // Make the whole user tree read-only: any attempted write would fail loudly.
    set_readonly_tree(&fx.user, true);
    let before = snapshot(&fx.user);

    let ix = Indexer::start(AppData::open(&fx.data).unwrap(), fx.settings(), FakeProvider::new()).unwrap();
    assert!(ix.wait_idle(Duration::from_secs(60)));
    assert_eq!(ix.status().counts.done, 14, "{:?}", ix.status());
    assert_eq!(ix.status().counts.failed, 0, "read-only files must index fine: {:?}", ix.status());
    ix.rescan();
    assert!(ix.wait_idle(Duration::from_secs(60)));
    ix.set_provider(FakeProvider::new()).unwrap();
    assert!(ix.wait_idle(Duration::from_secs(60)));
    ix.wipe().unwrap();
    assert!(ix.wait_idle(Duration::from_secs(60)));
    let mut s = fx.settings();
    s.folders.clear();
    ix.update_settings(s).unwrap();
    assert!(ix.wait_idle(Duration::from_secs(60)));
    drop(ix);

    let after = snapshot(&fx.user);
    set_readonly_tree(&fx.user, false);
    assert_eq!(before.len(), after.len(), "no files may be added or removed in user folders");
    for (path, b) in &before {
        assert_eq!(Some(b), after.get(path), "{} changed", path.display());
    }
    // And everything we wrote is inside the app data dir.
    for e in walkdir::WalkDir::new(fx.dir.path()).into_iter().flatten() {
        let p = e.path();
        assert!(p.starts_with(&fx.user) || p.starts_with(&fx.data) || p == fx.dir.path(), "stray file {}", p.display());
    }
}

fn set_readonly_tree(root: &std::path::Path, ro: bool) {
    // Files first, then directories bottom-up (so we can still traverse).
    let entries: Vec<_> = walkdir::WalkDir::new(root).contents_first(true).into_iter().flatten().collect();
    for e in entries {
        let mut perm = std::fs::metadata(e.path()).unwrap().permissions();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = if e.file_type().is_dir() {
                if ro {
                    0o555
                } else {
                    0o755
                }
            } else if ro {
                0o444
            } else {
                0o644
            };
            perm.set_mode(mode);
        }
        #[cfg(not(unix))]
        {
            if !e.file_type().is_dir() {
                perm.set_readonly(ro);
            }
        }
        std::fs::set_permissions(e.path(), perm).unwrap();
    }
}

/// Static audit: only `appdata.rs` may contain file-writing APIs. Anything else
/// that writes must go through `AppData`, which refuses paths outside app data.
#[test]
fn only_appdata_module_writes_files() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    const FORBIDDEN: &[&str] = &[
        "fs::write(",
        "File::create(",
        "fs::remove_file(",
        "fs::remove_dir",
        "fs::rename(",
        "fs::copy(",
        ".write(true)",
        ".append(true)",
        "set_permissions(",
        "set_modified(",
        "OpenOptions::new().create",
    ];
    // Allowed exceptions outside appdata.rs: (file, needle) pairs, each justified.
    const ALLOWED: &[(&str, &str)] = &[];
    let mut violations = Vec::new();
    for e in walkdir::WalkDir::new(&src).into_iter().flatten() {
        if e.path().extension().and_then(|x| x.to_str()) != Some("rs") {
            continue;
        }
        let name = e.file_name().to_string_lossy().to_string();
        if name == "appdata.rs" {
            continue;
        }
        let text = std::fs::read_to_string(e.path()).unwrap();
        for (ln, line) in text.lines().enumerate() {
            if line.trim_start().starts_with("#[cfg(test)]") {
                break; // unit tests write fixtures into temp dirs
            }
            let code = line.split("//").next().unwrap_or("");
            for f in FORBIDDEN {
                if code.contains(f) && !ALLOWED.iter().any(|(file, n)| *file == name && *n == *f) {
                    violations.push(format!("{}:{}: {}", name, ln + 1, line.trim()));
                }
            }
        }
    }
    assert!(violations.is_empty(), "file-writing calls outside appdata.rs:\n{}", violations.join("\n"));
}
