//! TUIs for Pointman: the node's TUI folder. The extension ships `workspace/` (the workspace's Cargo.toml, its cargo
//! config, the README, the kit, the starter and an empty index) and keeps a copy of it on the node, at
//! `~/.local/share/message-board/tuis/`, where every agent builds its TUIs (James, Pointman thread 17421).
//!
//! What's the extension's is written again whenever it differs (so a release reaches every node): `Cargo.toml`,
//! `.cargo/config.toml` (shipped as `cargo-config.toml`: the node leaves hidden files out of an extension it unpacks),
//! `README.md`, `rustfmt.toml` (the repo's style, so `cargo fmt` in the folder leaves the kit as it ships), and the folders `kit/` and `crates/_starter/` (the starter, a member so `crates/*` is never empty: cargo
//! refuses a workspace whose glob matches nothing), which are made exactly like the release's. `tuis.toml` is written
//! only when it isn't there: agents add to it. Nothing else is touched: the rest of `crates/` (the agents' TUIs),
//! `Cargo.lock` and `target/` are theirs. One part that can't be written doesn't stop the others: each is said.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Files the extension keeps as the release has them: where each is in the release's `workspace/`, and on the node.
pub const KEPT_FILES: [(&str, &str); 4] = [
    ("Cargo.toml", "Cargo.toml"),
    ("cargo-config.toml", ".cargo/config.toml"),
    ("README.md", "README.md"),
    ("rustfmt.toml", "rustfmt.toml"),
];
/// Folders the extension keeps exactly as the release has them (files it no longer has go).
pub const KEPT_DIRS: [&str; 2] = ["kit", "crates/_starter"];
/// Folders an earlier release made that this one doesn't: taken out (0.1.0 and 0.1.1 kept the starter at the top).
pub const GONE_DIRS: [&str; 1] = ["starter"];
/// Written once, then the agents'.
pub const SEEDED: [&str; 1] = ["tuis.toml"];

/// What an install did: the files it wrote and the ones it took out, relative to the folder, and each part it
/// couldn't write, with why.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub written: Vec<String>,
    pub removed: Vec<String>,
    pub problems: Vec<String>,
}

/// The node's TUI folder: `$POINTMAN_TUIS_DIR`, else `~/.local/share/message-board/tuis`.
pub fn folder(env_dir: Option<&str>, home: &Path) -> PathBuf {
    match env_dir.filter(|d| !d.is_empty()) {
        Some(d) => PathBuf::from(d),
        None => home.join(".local/share/message-board/tuis"),
    }
}

/// Make `dest` from `seed` (the release's `workspace/`), or bring it up to date. Only a folder that can't be made at
/// all is an error; a part that can't be written is in the report's problems, and the rest go on.
pub fn install(seed: &Path, dest: &Path) -> io::Result<Report> {
    let mut report = Report::default();
    fs::create_dir_all(dest.join("crates"))?;
    let note = |what: &str, got: io::Result<()>, report: &mut Report| {
        if let Err(e) = got {
            report.problems.push(format!("{what}: {e}"));
        }
    };
    for (from, to) in KEPT_FILES {
        let got = write_if_changed(&seed.join(from), &dest.join(to), to, &mut report);
        note(to, got, &mut report);
    }
    for rel in SEEDED {
        if !dest.join(rel).exists() {
            let got = write_if_changed(&seed.join(rel), &dest.join(rel), rel, &mut report);
            note(rel, got, &mut report);
        }
    }
    for dir in KEPT_DIRS {
        let got = sync_dir(&seed.join(dir), &dest.join(dir), dir, &mut report);
        note(dir, got, &mut report);
    }
    for dir in GONE_DIRS {
        if dest.join(dir).is_dir() {
            let got = fs::remove_dir_all(dest.join(dir));
            if got.is_ok() {
                report.removed.push(dir.to_string());
            }
            note(dir, got, &mut report);
        }
    }
    Ok(report)
}

fn write_if_changed(from: &Path, to: &Path, rel: &str, report: &mut Report) -> io::Result<()> {
    let new = fs::read(from)?;
    if fs::read(to).ok().as_deref() == Some(new.as_slice()) {
        return Ok(());
    }
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)?;
    }
    // written beside, then moved in, so a TUI being built never reads half a file
    let part = to.with_extension("part");
    fs::write(&part, &new)?;
    fs::rename(&part, to)?;
    report.written.push(rel.to_string());
    Ok(())
}

/// `to` made exactly like `from`: each file written if it differs, and what `from` hasn't got taken out.
fn sync_dir(from: &Path, to: &Path, rel: &str, report: &mut Report) -> io::Result<()> {
    fs::create_dir_all(to)?;
    let mut wanted = Vec::new();
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let child = format!("{rel}/{name}");
        if entry.file_type()?.is_dir() {
            sync_dir(&entry.path(), &to.join(&name), &child, report)?;
        } else {
            write_if_changed(&entry.path(), &to.join(&name), &child, report)?;
        }
        wanted.push(name);
    }
    for entry in fs::read_dir(to)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if wanted.contains(&name) {
            continue;
        }
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            fs::remove_dir_all(&path)?;
        } else {
            fs::remove_file(&path)?;
        }
        report.removed.push(format!("{rel}/{name}"));
    }
    Ok(())
}

/// The TUIs in the folder: each `crates/<name>` with a Cargo.toml, sorted, but not the extension's own `_starter`.
pub fn tuis(dest: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dest.join("crates"))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().join("Cargo.toml").is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| !n.starts_with('_'))
        .collect();
    names.sort();
    names
}

/// Where a tool is on this machine: on the PATH, else where rustup, Homebrew, Nix and the system put it, else in a
/// rustup toolchain (nixos has `~/.rustup/toolchains/stable-…/bin/cargo` and no proxies).
fn find(tool: &str, path_var: Option<&str>, home: &Path) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = path_var.map(|p| std::env::split_paths(p).collect()).unwrap_or_default();
    dirs.extend([
        home.join(".cargo/bin"),
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        home.join(".nix-profile/bin"),
        PathBuf::from("/run/current-system/sw/bin"),
        PathBuf::from("/usr/bin"),
    ]);
    let mut toolchains: Vec<PathBuf> = fs::read_dir(home.join(".rustup/toolchains"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path().join("bin"))
        .collect();
    // stable first, then the rest by name
    toolchains.sort_by_key(|p| (!p.to_string_lossy().contains("/stable-"), p.clone()));
    dirs.extend(toolchains);
    dirs.into_iter().map(|d| d.join(tool)).find(|p| p.is_file())
}

/// Cargo on this machine.
pub fn cargo(path_var: Option<&str>, home: &Path) -> Option<PathBuf> {
    find("cargo", path_var, home)
}

/// The C compiler cargo links with (`cc`). macOS has one in /usr/bin; nixos has none outside a nix-shell, so plain
/// cargo there stops at the first link (nixtest, Pointman thread 17421).
pub fn cc(path_var: Option<&str>, home: &Path) -> Option<PathBuf> {
    find("cc", path_var, home)
}

/// Nix's shell, which brings a C compiler for one command where the machine has none.
pub fn nix_shell(path_var: Option<&str>, home: &Path) -> Option<PathBuf> {
    find("nix-shell", path_var, home)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed(root: &Path) -> PathBuf {
        let s = root.join("seed");
        fs::create_dir_all(s.join("kit/src")).unwrap();
        fs::create_dir_all(s.join("crates/_starter/src")).unwrap();
        fs::write(s.join("Cargo.toml"), "[workspace]\n").unwrap();
        fs::write(s.join("cargo-config.toml"), "[build]\n").unwrap();
        fs::write(s.join("README.md"), "readme\n").unwrap();
        fs::write(s.join("rustfmt.toml"), "max_width = 120\n").unwrap();
        fs::write(s.join("tuis.toml"), "# index\n").unwrap();
        fs::write(s.join("kit/Cargo.toml"), "[package]\nname = \"kit\"\n").unwrap();
        fs::write(s.join("kit/src/lib.rs"), "// kit 1\n").unwrap();
        fs::write(s.join("crates/_starter/Cargo.toml"), "[package]\nname = \"tui-starter\"\n").unwrap();
        fs::write(s.join("crates/_starter/src/main.rs"), "fn main() {}\n").unwrap();
        s
    }

    #[test]
    fn a_new_node_gets_the_whole_folder_and_a_second_start_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (s, d) = (seed(dir.path()), dir.path().join("tuis"));
        let first = install(&s, &d).unwrap();
        assert_eq!(first.written.len(), 9, "{first:?}");
        assert!(d.join("crates").is_dir() && d.join("kit/src/lib.rs").is_file() && d.join("tuis.toml").is_file());
        assert_eq!(fs::read_to_string(d.join(".cargo/config.toml")).unwrap(), "[build]\n", "from cargo-config.toml");
        assert!(first.problems.is_empty(), "{first:?}");
        assert_eq!(install(&s, &d).unwrap(), Report::default(), "nothing changed, nothing written");
    }

    #[test]
    fn a_release_updates_the_kit_and_never_touches_the_agents_tuis_or_index() {
        let dir = tempfile::tempdir().unwrap();
        let (s, d) = (seed(dir.path()), dir.path().join("tuis"));
        install(&s, &d).unwrap();
        // an agent's TUI, its index entry, the lock and the build folder
        fs::create_dir_all(d.join("crates/releases/src")).unwrap();
        fs::write(d.join("crates/releases/Cargo.toml"), "[package]\nname = \"tui-releases\"\n").unwrap();
        fs::write(d.join("tuis.toml"), "[tui.releases]\n").unwrap();
        fs::write(d.join("Cargo.lock"), "lock\n").unwrap();
        fs::create_dir_all(d.join("target/release")).unwrap();
        // a new release: the kit changed, and lost a file
        fs::write(s.join("kit/src/lib.rs"), "// kit 2\n").unwrap();
        fs::write(d.join("kit/src/old.rs"), "// gone in this release\n").unwrap();
        let r = install(&s, &d).unwrap();
        assert_eq!(r.written, ["kit/src/lib.rs"]);
        assert_eq!(r.removed, ["kit/src/old.rs"]);
        assert_eq!(fs::read_to_string(d.join("kit/src/lib.rs")).unwrap(), "// kit 2\n");
        assert_eq!(fs::read_to_string(d.join("tuis.toml")).unwrap(), "[tui.releases]\n", "the index is the agents'");
        assert!(d.join("crates/releases/Cargo.toml").is_file() && d.join("Cargo.lock").is_file());
        assert!(d.join("target/release").is_dir());
        assert_eq!(tuis(&d), ["releases"], "the starter isn't one of the agents' TUIs");
    }

    /// nixtest on nixos (Pointman thread 17421): with no TUI yet, `crates/*` matched nothing and cargo refused the
    /// whole workspace. The starter is a member in crates/ now, and the top-level starter/ 0.1.0 made goes.
    #[test]
    fn crates_is_never_empty_and_the_old_starter_folder_goes() {
        let dir = tempfile::tempdir().unwrap();
        let (s, d) = (seed(dir.path()), dir.path().join("tuis"));
        fs::create_dir_all(d.join("starter/src")).unwrap();
        fs::write(d.join("starter/src/main.rs"), "// 0.1.1's\n").unwrap();
        let r = install(&s, &d).unwrap();
        assert!(d.join("crates/_starter/Cargo.toml").is_file() && !d.join("starter").exists(), "{r:?}");
        assert!(r.removed.contains(&"starter".to_string()), "{r:?}");
        assert!(tuis(&d).is_empty());
    }

    #[test]
    fn an_edited_workspace_file_is_put_back() {
        let dir = tempfile::tempdir().unwrap();
        let (s, d) = (seed(dir.path()), dir.path().join("tuis"));
        install(&s, &d).unwrap();
        fs::write(d.join("Cargo.toml"), "[workspace]\nmembers = [\"elsewhere\"]\n").unwrap();
        assert_eq!(install(&s, &d).unwrap().written, ["Cargo.toml"]);
        assert_eq!(fs::read_to_string(d.join("Cargo.toml")).unwrap(), "[workspace]\n");
    }

    /// v0.1.0 on nixos: the node had left the release's hidden .cargo/ out, the install stopped there, and the
    /// kit, the starter, the README and the index never came (nixtest, Pointman thread 17421).
    #[test]
    fn a_part_missing_from_the_release_is_said_and_the_rest_still_come() {
        let dir = tempfile::tempdir().unwrap();
        let (s, d) = (seed(dir.path()), dir.path().join("tuis"));
        fs::remove_file(s.join("cargo-config.toml")).unwrap();
        let r = install(&s, &d).unwrap();
        assert_eq!(r.problems.len(), 1, "{r:?}");
        assert!(r.problems[0].starts_with(".cargo/config.toml: "), "{r:?}");
        for made in ["Cargo.toml", "README.md", "tuis.toml", "kit/src/lib.rs", "crates/_starter/src/main.rs"] {
            assert!(d.join(made).is_file(), "{made}");
        }
    }

    #[test]
    fn cargo_in_a_rustup_toolchain_is_found() {
        let home = tempfile::tempdir().unwrap();
        let bin = home.path().join(".rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin");
        fs::create_dir_all(&bin).unwrap();
        fs::write(bin.join("cargo"), "").unwrap();
        assert_eq!(cargo(Some("/nowhere"), home.path()), Some(bin.join("cargo")));
    }

    #[test]
    fn a_c_compiler_and_nix_shell_are_found_on_the_path() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        fs::create_dir_all(&bin).unwrap();
        fs::write(bin.join("cc"), "").unwrap();
        fs::write(bin.join("nix-shell"), "").unwrap();
        let path = bin.to_str().unwrap();
        assert_eq!(cc(Some(path), dir.path()), Some(bin.join("cc")));
        assert_eq!(nix_shell(Some(path), dir.path()), Some(bin.join("nix-shell")));
    }

    #[test]
    fn the_folder_and_cargo_are_found() {
        let home = Path::new("/home/x");
        assert_eq!(folder(None, home), home.join(".local/share/message-board/tuis"));
        assert_eq!(folder(Some(""), home), home.join(".local/share/message-board/tuis"));
        assert_eq!(folder(Some("/tmp/t"), home), PathBuf::from("/tmp/t"));
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        fs::create_dir_all(&bin).unwrap();
        assert_eq!(cargo(Some(bin.to_str().unwrap()), dir.path()), None);
        fs::write(bin.join("cargo"), "").unwrap();
        assert_eq!(cargo(Some(bin.to_str().unwrap()), dir.path()), Some(bin.join("cargo")));
    }
}
