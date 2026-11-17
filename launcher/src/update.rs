//! Updates from the project's GitHub releases.
//!
//! On start (and on "Check for updates") the launcher asks GitHub for the
//! newest release of `echo-vrce/Echo-RS`, takes this platform's zip
//! (`echo-rust-server-windows.zip` / `-linux.zip`) and, when it is not the one
//! already installed, downloads it and replaces the server and launcher
//! executables next to the launcher. Nothing else in the folder is touched
//! (settings, logs, `server-data`, `echocombat.ini`).
//!
//! A release is re-uploaded under the same tag, so "installed" is the asset's
//! upload time, kept in `update-state.json`. The files are swapped only while
//! the server is stopped; a running executable is renamed aside (`*.old`) and
//! removed at the next start.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};

const LATEST: &str = "https://api.github.com/repos/echo-vrce/Echo-RS/releases/latest";
const DOWNLOAD_PREFIX: &str = "https://github.com/echo-vrce/Echo-RS/releases/download/";
const MAX_ZIP: u64 = 300 * 1024 * 1024;
const AGENT: &str = "echo-server-launcher";

fn asset_name() -> &'static str {
    if cfg!(windows) { "echo-rust-server-windows.zip" } else { "echo-rust-server-linux.zip" }
}

/// The only files an update may write.
fn wanted(name: &str) -> bool {
    let names: &[&str] =
        if cfg!(windows) { &["echo-rust-server.exe", "echo-server-launcher.exe"] } else { &["echo-rust-server", "echo-server-launcher"] };
    names.contains(&name)
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Status {
    #[default]
    Idle,
    Checking,
    UpToDate(String),
    Downloading(String),
    /// Downloaded; waiting for the server to stop.
    Ready(String),
    Installed { tag: String, restart: bool },
    Failed(String),
}

#[derive(Serialize, Deserialize, Default)]
struct Installed {
    tag: String,
    asset_updated_at: String,
}

#[derive(Default)]
struct Inner {
    status: Status,
    /// A downloaded release waiting to be installed: (tag, upload time, zip).
    pending: Option<(String, String, Vec<u8>)>,
    busy: bool,
}

#[derive(Clone, Default)]
pub struct Updater {
    inner: Arc<Mutex<Inner>>,
}

fn state_path(dir: &Path) -> PathBuf {
    dir.join("update-state.json")
}

/// Leftovers of the last update (the executables that were running).
pub fn clean_old(dir: &Path) {
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "old") && p.file_stem().and_then(|s| s.to_str()).is_some_and(wanted) {
                let _ = std::fs::remove_file(p);
            }
        }
    }
}

fn get(url: &str) -> Result<ureq::Response, String> {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout(Duration::from_secs(300))
        .build()
        .get(url)
        .set("User-Agent", AGENT)
        .set("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| e.to_string())
}

/// (tag, asset upload time, download url) of the newest release's zip for this platform.
fn latest() -> Result<(String, String, String), String> {
    let text = get(LATEST)?.into_string().map_err(|e| e.to_string())?;
    let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let tag = v["tag_name"].as_str().unwrap_or("").to_string();
    let asset = v["assets"]
        .as_array()
        .and_then(|a| a.iter().find(|x| x["name"].as_str() == Some(asset_name())))
        .ok_or_else(|| format!("the newest release ({tag}) has no {}", asset_name()))?;
    let url = asset["browser_download_url"].as_str().unwrap_or("").to_string();
    if !url.starts_with(DOWNLOAD_PREFIX) {
        return Err("unexpected download address".into());
    }
    Ok((tag, asset["updated_at"].as_str().unwrap_or("").to_string(), url))
}

fn download(url: &str) -> Result<Vec<u8>, String> {
    let mut zip = Vec::new();
    get(url)?.into_reader().take(MAX_ZIP + 1).read_to_end(&mut zip).map_err(|e| e.to_string())?;
    if zip.len() as u64 > MAX_ZIP {
        return Err("the download is too large".into());
    }
    Ok(zip)
}

/// Write the zip's executables into `dir`. Returns whether the launcher itself changed.
fn install(dir: &Path, zip: &[u8]) -> Result<bool, String> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip)).map_err(|e| e.to_string())?;
    let me = std::env::current_exe().ok().and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()));
    let mut written = 0;
    let mut launcher = false;
    for i in 0..archive.len() {
        let mut f = archive.by_index(i).map_err(|e| e.to_string())?;
        // Only the base name is used: nothing can land outside the folder.
        let Some(name) = f.enclosed_name().and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())) else { continue };
        if !f.is_file() || !wanted(&name) {
            continue;
        }
        let mut data = Vec::new();
        f.read_to_end(&mut data).map_err(|e| e.to_string())?;
        let target = dir.join(&name);
        let fresh = dir.join(format!("{name}.new"));
        std::fs::write(&fresh, &data).map_err(|e| format!("{}: {e}", fresh.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&fresh, std::fs::Permissions::from_mode(0o755));
        }
        if target.exists() {
            let old = dir.join(format!("{name}.old"));
            let _ = std::fs::remove_file(&old);
            std::fs::rename(&target, &old).map_err(|e| format!("{}: {e}", target.display()))?;
        }
        std::fs::rename(&fresh, &target).map_err(|e| format!("{}: {e}", target.display()))?;
        launcher |= me.as_deref() == Some(name.as_str());
        written += 1;
    }
    if written == 0 {
        return Err("the release zip has no server files".into());
    }
    Ok(launcher)
}

/// Seconds since 1970 of a GitHub time (`2026-11-16T02:47:01Z`).
fn unix_time(t: &str) -> Option<u64> {
    let n = |r: std::ops::Range<usize>| t.get(r)?.parse::<i64>().ok();
    let (y, m, d, h, mi, s) = (n(0..4)?, n(5..7)?, n(8..10)?, n(11..13)?, n(14..16)?, n(17..19)?);
    // Days from the civil date (Howard Hinnant's algorithm).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let days = era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468;
    u64::try_from(days * 86_400 + h * 3600 + mi * 60 + s).ok()
}

/// `v1.2.3` as numbers, or none if the tag is not a version.
fn version(tag: &str) -> Option<Vec<u64>> {
    tag.trim().trim_start_matches(['v', 'V']).split('.').map(|p| p.parse().ok()).collect()
}

/// The release `tag` is not newer than this build's version.
fn not_newer(tag: &str, mine: &str) -> bool {
    matches!((version(tag), version(mine)), (Some(t), Some(m)) if t <= m)
}

/// The server executable here was built after the release was uploaded.
fn local_is_newer(dir: &Path, asset_updated_at: &str) -> bool {
    let name = if cfg!(windows) { "echo-rust-server.exe" } else { "echo-rust-server" };
    let local = std::fs::metadata(dir.join(name))
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs());
    matches!((local, unix_time(asset_updated_at)), (Some(l), Some(a)) if l >= a)
}

impl Updater {
    pub fn status(&self) -> Status {
        self.inner.lock().expect("updater").status.clone()
    }

    pub fn busy(&self) -> bool {
        self.inner.lock().expect("updater").busy
    }

    fn set(&self, s: Status) {
        self.inner.lock().expect("updater").status = s;
    }

    /// Look for a newer release in the background and download it.
    ///
    /// With no record of an installed release (a first run, or a build made
    /// here), an automatic check leaves executables alone that are newer than
    /// the release: it must never put an older server back. `force` (the
    /// button) installs the newest release regardless.
    pub fn check(&self, dir: PathBuf, force: bool, wake: impl Fn() + Send + 'static) {
        {
            let mut i = self.inner.lock().expect("updater");
            if i.busy {
                return;
            }
            i.busy = true;
            i.status = Status::Checking;
        }
        let this = self.clone();
        std::thread::spawn(move || {
            let result = (|| -> Result<Status, String> {
                let (tag, updated, url) = latest()?;
                let have: Installed = std::fs::read_to_string(state_path(&dir)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
                if have.tag == tag && have.asset_updated_at == updated {
                    return Ok(Status::UpToDate(tag));
                }
                if !force && have.asset_updated_at.is_empty() && (not_newer(&tag, env!("CARGO_PKG_VERSION")) || local_is_newer(&dir, &updated)) {
                    return Ok(Status::UpToDate(tag));
                }
                this.set(Status::Downloading(tag.clone()));
                wake();
                let zip = download(&url)?;
                this.inner.lock().expect("updater").pending = Some((tag.clone(), updated, zip));
                Ok(Status::Ready(tag))
            })();
            let mut i = this.inner.lock().expect("updater");
            i.busy = false;
            i.status = result.unwrap_or_else(Status::Failed);
            drop(i);
            wake();
        });
    }

    /// Install a downloaded release. Call only while the server is stopped.
    pub fn install_pending(&self, dir: &Path) {
        let Some((tag, updated, zip)) = self.inner.lock().expect("updater").pending.take() else { return };
        let status = match install(dir, &zip) {
            Ok(restart) => {
                let state = Installed { tag: tag.clone(), asset_updated_at: updated };
                let _ = std::fs::write(state_path(dir), serde_json::to_string_pretty(&state).unwrap_or_default());
                Status::Installed { tag, restart }
            }
            Err(e) => Status::Failed(e),
        };
        self.set(status);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_times_parse() {
        assert_eq!(unix_time("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(unix_time("2024-02-29T12:00:00Z"), Some(1_709_208_000));
        assert_eq!(unix_time("nonsense"), None);
    }

    #[test]
    fn each_platform_takes_its_own_zip_and_files() {
        let name = asset_name();
        assert_eq!(name, if cfg!(windows) { "echo-rust-server-windows.zip" } else { "echo-rust-server-linux.zip" });
        // A zip carrying both platforms' files (and a settings file).
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            use std::io::Write;
            let mut z = zip::ZipWriter::new(&mut buf);
            let o = zip::write::SimpleFileOptions::default();
            for f in ["echo-rust-server.exe", "echo-server-launcher.exe", "echo-rust-server", "echo-server-launcher", "launcher-setup.json"] {
                z.start_file(format!("echo-rust-server/{f}"), o).expect("start");
                z.write_all(f.as_bytes()).expect("write");
            }
            z.finish().expect("finish");
        }
        let dir = std::env::temp_dir().join(format!("ers-update-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        install(&dir, buf.get_ref()).expect("install");
        let mut got: Vec<String> = std::fs::read_dir(&dir).expect("read").flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        got.sort();
        let want: Vec<&str> = if cfg!(windows) { vec!["echo-rust-server.exe", "echo-server-launcher.exe"] } else { vec!["echo-rust-server", "echo-server-launcher"] };
        assert_eq!(got, want);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn release_tags_compare_as_versions() {
        assert!(not_newer("v0.1.0", "0.1.1"));
        assert!(not_newer("v0.1.1", "0.1.1"));
        assert!(!not_newer("v0.1.2", "0.1.1"));
        assert!(!not_newer("v0.10.0", "0.9.9"));
        assert!(!not_newer("nightly", "0.1.1"));
    }

    #[test]
    fn only_the_executables_are_written() {
        assert!(wanted(if cfg!(windows) { "echo-rust-server.exe" } else { "echo-rust-server" }));
        assert!(!wanted("launcher-setup.json"));
        assert!(!wanted("echocombat.ini"));
    }
}
