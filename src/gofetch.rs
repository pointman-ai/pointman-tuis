//! Go for a machine that has none (EXT-41): the newest stable release from go.dev, checked against the sha256 go.dev's
//! own index gives, then unpacked as the extension's Go folder (`crate::go_dir`), so `<go_dir>/bin/go` builds the TUI
//! folder's apps. Nothing is unpacked before the download matches, and the folder is moved in whole.

use std::fs;
use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// go.dev's list of releases, newest first, each file with its os, arch, kind, size and sha256.
pub const INDEX: &str = "https://go.dev/dl/?mode=json";
const DOWNLOADS: &str = "https://go.dev/dl/";

/// One release's archive for this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Archive {
    pub version: String,
    pub filename: String,
    pub sha256: String,
    pub size: u64,
}

impl Archive {
    pub fn url(&self) -> String {
        format!("{DOWNLOADS}{}", self.filename)
    }
}

/// Go's names for this machine (`darwin`/`linux`, `arm64`/`amd64`), or None where Go has no release for it.
pub fn platform() -> Option<(&'static str, &'static str)> {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        "linux" => "linux",
        _ => return None,
    };
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "amd64",
        _ => return None,
    };
    Some((os, arch))
}

/// The newest stable release's archive for `os`/`arch` in go.dev's index.
pub fn pick(index: &Value, os: &str, arch: &str) -> Option<Archive> {
    index.as_array()?.iter().filter(|r| r["stable"].as_bool() == Some(true)).find_map(|r| {
        let f = r["files"].as_array()?.iter().find(|f| f["os"] == os && f["arch"] == arch && f["kind"] == "archive")?;
        Some(Archive {
            version: r["version"].as_str()?.into(),
            filename: f["filename"].as_str()?.into(),
            sha256: f["sha256"].as_str()?.into(),
            size: f["size"].as_u64().unwrap_or(0),
        })
    })
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(20)).timeout_read(Duration::from_secs(60)).build()
}

/// The newest stable Go for this machine.
pub fn newest() -> Result<Archive> {
    let (os, arch) = platform().context("Go has no release for this machine's platform")?;
    let index: Value = agent().get(INDEX).call().context("asking go.dev for its releases")?.into_json()?;
    pick(&index, os, arch).with_context(|| format!("go.dev lists no stable Go for {os}-{arch}"))
}

/// Download `a`, check it, and make `dir` of it. `progress(bytes so far)` as it comes.
pub fn fetch(a: &Archive, dir: &Path, progress: impl FnMut(u64)) -> Result<()> {
    let body = agent().get(&a.url()).call().with_context(|| format!("downloading {}", a.url()))?.into_reader();
    install_from(body, a, dir, progress)
}

/// [`fetch`] from any reader (tests hand it an archive they made): the download is written beside `dir` and hashed
/// as it comes; only a match is unpacked, beside too, and its `go/` then moved in as `dir`.
pub fn install_from(mut body: impl Read, a: &Archive, dir: &Path, mut progress: impl FnMut(u64)) -> Result<()> {
    let parent = dir.parent().context("the Go folder has no parent")?;
    fs::create_dir_all(parent)?;
    let name = dir.file_name().context("the Go folder has no name")?.to_string_lossy().into_owned();
    let (download, unpacking) = (parent.join(format!("{name}.download")), parent.join(format!("{name}.unpacking")));
    let got = (|| -> Result<()> {
        let mut hash = Sha256::new();
        let mut file = fs::File::create(&download)?;
        let (mut buf, mut done) = (vec![0u8; 1 << 16], 0u64);
        loop {
            let n = body.read(&mut buf).context("reading the download")?;
            if n == 0 {
                break;
            }
            hash.update(&buf[..n]);
            file.write_all(&buf[..n])?;
            done += n as u64;
            progress(done);
        }
        file.sync_all()?;
        let sum = format!("{:x}", hash.finalize());
        if !sum.eq_ignore_ascii_case(&a.sha256) {
            bail!("{} didn't match go.dev's sha256, so it isn't used", a.filename);
        }
        let _ = fs::remove_dir_all(&unpacking);
        tar::Archive::new(flate2::read::GzDecoder::new(fs::File::open(&download)?))
            .unpack(&unpacking)
            .with_context(|| format!("unpacking {}", a.filename))?;
        let go = unpacking.join("go");
        if !go.join("bin/go").is_file() {
            bail!("{} has no go/bin/go", a.filename);
        }
        if dir.exists() {
            fs::remove_dir_all(dir)?;
        }
        fs::rename(&go, dir)?;
        Ok(())
    })();
    let _ = fs::remove_file(&download);
    let _ = fs::remove_dir_all(&unpacking);
    got
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A go.dev-shaped archive: go/bin/go (executable) and go/VERSION.
    fn archive() -> Vec<u8> {
        let mut tar = tar::Builder::new(flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast()));
        for (path, body, mode) in
            [("go/bin/go", &b"#!/bin/sh\necho go\n"[..], 0o755), ("go/VERSION", b"go1.27.3\n", 0o644)]
        {
            let mut h = tar::Header::new_gnu();
            h.set_size(body.len() as u64);
            h.set_mode(mode);
            h.set_cksum();
            tar.append_data(&mut h, path, body).unwrap();
        }
        tar.into_inner().unwrap().finish().unwrap()
    }

    fn described(bytes: &[u8]) -> Archive {
        Archive {
            version: "go1.27.3".into(),
            filename: "go1.27.3.linux-amd64.tar.gz".into(),
            sha256: format!("{:x}", Sha256::digest(bytes)),
            size: bytes.len() as u64,
        }
    }

    #[test]
    fn the_newest_stable_archive_for_the_machine_is_picked() {
        let file = |os: &str, arch: &str, kind: &str| json!({"filename": format!("go-{os}-{arch}.{kind}"), "os": os, "arch": arch, "kind": kind, "sha256": "ab", "size": 9});
        let index = json!([
            {"version": "go1.28rc1", "stable": false, "files": [file("linux", "amd64", "archive")]},
            {"version": "go1.27.3", "stable": true, "files": [file("linux", "amd64", "source"), file("darwin", "arm64", "archive"), file("linux", "amd64", "archive")]},
            {"version": "go1.26.9", "stable": true, "files": [file("linux", "arm64", "archive")]},
        ]);
        let got = pick(&index, "linux", "amd64").unwrap();
        assert_eq!((got.version.as_str(), got.filename.as_str()), ("go1.27.3", "go-linux-amd64.archive"));
        assert_eq!(pick(&index, "linux", "arm64").unwrap().version, "go1.26.9", "the newest that has one");
        assert_eq!(pick(&index, "freebsd", "amd64"), None);
        assert_eq!(got.url(), "https://go.dev/dl/go-linux-amd64.archive");
    }

    #[test]
    fn a_matching_download_becomes_the_go_folder_with_go_runnable() {
        let dir = tempfile::tempdir().unwrap();
        let go = dir.path().join("go");
        fs::create_dir_all(go.join("old")).unwrap(); // a Go fetched before: replaced whole
        let bytes = archive();
        let mut seen = 0;
        install_from(&bytes[..], &described(&bytes), &go, |n| seen = n).unwrap();
        assert_eq!(seen, bytes.len() as u64);
        assert_eq!(fs::read_to_string(go.join("VERSION")).unwrap(), "go1.27.3\n");
        assert!(!go.join("old").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(go.join("bin/go")).unwrap().permissions().mode() & 0o111, 0o111);
        }
        let left: Vec<_> = fs::read_dir(dir.path()).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(left, ["go"], "nothing left beside it");
    }

    #[test]
    fn a_download_that_doesnt_match_is_never_unpacked() {
        let dir = tempfile::tempdir().unwrap();
        let go = dir.path().join("go");
        let bytes = archive();
        let mut a = described(&bytes);
        a.sha256 = "0".repeat(64);
        let e = install_from(&bytes[..], &a, &go, |_| {}).unwrap_err().to_string();
        assert!(e.contains("didn't match go.dev's sha256"), "{e}");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0, "no Go folder, no download left");
    }

    /// The real thing, from go.dev (about 70 MB): `cargo test --lib -- --ignored from_go_dev`.
    #[test]
    #[ignore]
    fn the_newest_go_comes_from_go_dev_and_runs() {
        let dir = tempfile::tempdir().unwrap();
        let a = newest().unwrap();
        fetch(&a, &dir.path().join("go"), |_| {}).unwrap();
        let out = std::process::Command::new(dir.path().join("go/bin/go")).arg("version").output().unwrap();
        let said = String::from_utf8_lossy(&out.stdout);
        assert!(said.contains(&format!("{} ", a.version)), "{said}");
    }
}
