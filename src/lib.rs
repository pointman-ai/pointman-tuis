//! TUIs for Pointman: the node's TUI folder. The extension ships `workspace/` (the Go module's go.mod and go.sum,
//! the kit, the starter, the README and an empty index) and keeps a copy of it on the node, at
//! `~/.local/share/message-board/tuis/`, where every agent builds its apps (James, Pointman thread 17421; Go and
//! Bubble Tea since [17682]).
//!
//! What's the extension's is written again whenever it differs (so a release reaches every node): `README.md`, and
//! the folders `kit/` and `cmd/_starter/`, made exactly like the release's. `go.mod` and `go.sum` are shared with the
//! agents, who add the modules their apps need: go.mod gets each module the kit needs at least at the release's
//! version (never lowering one an agent raised, never dropping one an agent added), go.sum every line the release
//! has. `tuis.toml` is written only when it isn't there: agents add to it. Nothing else is touched: the agents' apps
//! (`cmd/<name>/`) and what they built (`bin/`) are theirs. One part that can't be written doesn't stop the others:
//! each is said.
//!
//! Releases before 0.2.0 kept a cargo workspace here: what they made is taken out (and the build folder and lock
//! with it), never an agent's own `crates/<name>`.

pub mod gofetch;

use std::cmp::Ordering;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Files the extension keeps as the release has them.
pub const KEPT_FILES: [&str; 1] = ["README.md"];
/// Folders the extension keeps exactly as the release has them (files it no longer has go).
pub const KEPT_DIRS: [&str; 2] = ["kit", "cmd/_starter"];
/// Written once, then the agents'.
pub const SEEDED: [&str; 1] = ["tuis.toml"];
/// What the cargo releases (0.1.x) made, and what building with them left: taken out. `crates/` goes too once
/// nothing is left in it; an agent's `crates/<name>` stays until its owner moves it.
pub const GONE_FILES: [&str; 3] = ["Cargo.toml", "Cargo.lock", "rustfmt.toml"];
pub const GONE_DIRS: [&str; 4] = [".cargo", "crates/_starter", "starter", "target"];

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

/// Where the extension puts the Go it fetches for a machine that has none: `$POINTMAN_TUIS_GO_DIR`, else
/// `~/.local/share/message-board/go` (beside the TUI folder, never in it: `go build ./...` would walk Go's own source).
pub fn go_dir(env_dir: Option<&str>, home: &Path) -> PathBuf {
    match env_dir.filter(|d| !d.is_empty()) {
        Some(d) => PathBuf::from(d),
        None => home.join(".local/share/message-board/go"),
    }
}

/// Make `dest` from `seed` (the release's `workspace/`), or bring it up to date. Only a folder that can't be made at
/// all is an error; a part that can't be written is in the report's problems, and the rest go on.
pub fn install(seed: &Path, dest: &Path) -> io::Result<Report> {
    let mut report = Report::default();
    fs::create_dir_all(dest.join("cmd"))?;
    fs::create_dir_all(dest.join("bin"))?;
    let note = |what: &str, got: io::Result<()>, report: &mut Report| {
        if let Err(e) = got {
            report.problems.push(format!("{what}: {e}"));
        }
    };
    for rel in KEPT_FILES {
        let got = fs::read(seed.join(rel)).and_then(|new| write_if_changed(&new, &dest.join(rel), rel, &mut report));
        note(rel, got, &mut report);
    }
    let got = merged(seed, dest, "go.mod", go_mod, &mut report);
    note("go.mod", got, &mut report);
    let got = merged(seed, dest, "go.sum", go_sum, &mut report);
    note("go.sum", got, &mut report);
    for rel in SEEDED {
        if !dest.join(rel).exists() {
            let got =
                fs::read(seed.join(rel)).and_then(|new| write_if_changed(&new, &dest.join(rel), rel, &mut report));
            note(rel, got, &mut report);
        }
    }
    for dir in KEPT_DIRS {
        let got = sync_dir(&seed.join(dir), &dest.join(dir), dir, &mut report);
        note(dir, got, &mut report);
    }
    for rel in GONE_FILES {
        if dest.join(rel).is_file() {
            let got = fs::remove_file(dest.join(rel));
            if got.is_ok() {
                report.removed.push(rel.to_string());
            }
            note(rel, got, &mut report);
        }
    }
    for rel in GONE_DIRS {
        if dest.join(rel).is_dir() {
            let got = fs::remove_dir_all(dest.join(rel));
            if got.is_ok() {
                report.removed.push(rel.to_string());
            }
            note(rel, got, &mut report);
        }
    }
    // crates/ once only the cargo releases' parts were in it
    if fs::read_dir(dest.join("crates")).is_ok_and(|mut d| d.next().is_none()) {
        let got = fs::remove_dir(dest.join("crates"));
        if got.is_ok() {
            report.removed.push("crates".into());
        }
        note("crates", got, &mut report);
    }
    Ok(report)
}

/// A file the release and the agents share: the release's and the node's put together by `merge`.
fn merged(seed: &Path, dest: &Path, rel: &str, merge: fn(&str, &str) -> String, report: &mut Report) -> io::Result<()> {
    let release = fs::read_to_string(seed.join(rel))?;
    let node = fs::read_to_string(dest.join(rel)).unwrap_or_default();
    write_if_changed(merge(&node, &release).as_bytes(), &dest.join(rel), rel, report)
}

fn write_if_changed(new: &[u8], to: &Path, rel: &str, report: &mut Report) -> io::Result<()> {
    if fs::read(to).ok().as_deref() == Some(new) {
        return Ok(());
    }
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)?;
    }
    // written beside, then moved in, so an app being built never reads half a file
    let mut part = to.as_os_str().to_owned();
    part.push(".part");
    fs::write(&part, new)?;
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
            write_if_changed(&fs::read(entry.path())?, &to.join(&name), &child, report)?;
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

// ------------------------------------------------------------------------------------------------- go.mod and go.sum

/// One `require` in a go.mod: the module, its version, whether it's `// indirect`, and the line it's on.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Require {
    module: String,
    version: String,
    indirect: bool,
    line: usize,
}

fn requires(lines: &[String]) -> Vec<Require> {
    let mut out = Vec::new();
    let mut in_block = false;
    for (i, raw) in lines.iter().enumerate() {
        let (code, comment) = raw.split_once("//").unwrap_or((raw, ""));
        let words: Vec<&str> = code.split_whitespace().collect();
        let entry = match (in_block, words.as_slice()) {
            (false, ["require", "("]) => {
                in_block = true;
                None
            }
            (true, [")"]) => {
                in_block = false;
                None
            }
            (false, ["require", module, version]) | (true, [module, version]) => Some((*module, *version)),
            _ => None,
        };
        if let Some((module, version)) = entry {
            let indirect = comment.trim() == "indirect";
            out.push(Require { module: module.into(), version: version.into(), indirect, line: i });
        }
    }
    out
}

/// The value of a one-word directive (`go 1.26`, `toolchain go1.26.2`) and its line.
fn directive(lines: &[String], name: &str) -> Option<(usize, String)> {
    lines.iter().enumerate().find_map(|(i, l)| {
        let words: Vec<&str> = l.split("//").next().unwrap_or("").split_whitespace().collect();
        (words.len() == 2 && words[0] == name).then(|| (i, words[1].to_string()))
    })
}

/// The node's go.mod with the release's needs in: the module and Go versions the release asks for when they're
/// higher, each module the kit needs at least at the release's version, and everything else the agents put in
/// (their own modules, replaces, comments) as they left it. No go.mod yet: the release's.
pub fn go_mod(node: &str, release: &str) -> String {
    if node.trim().is_empty() {
        return release.to_string();
    }
    let mut lines: Vec<String> = node.lines().map(str::to_string).collect();
    let theirs: Vec<String> = release.lines().map(str::to_string).collect();

    // the module is the release's: the kit is imported as tuis/kit
    if let (Some((i, _)), Some((_, want))) = (directive(&lines, "module"), directive(&theirs, "module")) {
        lines[i] = format!("module {want}");
    }
    for name in ["go", "toolchain"] {
        match (directive(&lines, name), directive(&theirs, name)) {
            (Some((i, have)), Some((_, want))) if go_version_cmp(&want, &have) == Ordering::Greater => {
                lines[i] = format!("{name} {want}")
            }
            (None, Some((_, want))) => {
                // after the go line (the module's, for go itself)
                let after = directive(&lines, "go").or_else(|| directive(&lines, "module")).map_or(0, |(i, _)| i + 1);
                lines.insert(after, format!("{name} {want}"));
            }
            _ => {}
        }
    }

    let mut added = Vec::new();
    for want in requires(&theirs) {
        let have = requires(&lines);
        match have.iter().find(|r| r.module == want.module) {
            Some(r) if version_cmp(&want.version, &r.version) == Ordering::Greater => {
                lines[r.line] = lines[r.line].replacen(&r.version, &want.version, 1);
            }
            Some(_) => {}
            None => added.push(want),
        }
    }
    for r in &added {
        let entry = format!("\t{} {}{}", r.module, r.version, if r.indirect { " // indirect" } else { "" });
        // a direct one into the first require block, an indirect one into the block of indirect ones (go's own
        // layout), else the last; with no block, a new one at the end
        let blocks = blocks(&lines);
        let into = if r.indirect {
            blocks.iter().rev().find(|(s, e)| lines[*s..*e].iter().any(|l| l.contains("// indirect"))).or(blocks.last())
        } else {
            blocks.first()
        };
        match into {
            Some(&(_, end)) => lines.insert(end, entry),
            None => lines.extend([String::new(), "require (".into(), entry, ")".into()]),
        }
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

/// Each `require ( … )` block: the line it opens on and the line of its `)`.
fn blocks(lines: &[String]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, l) in lines.iter().enumerate() {
        let code = l.split("//").next().unwrap_or("");
        match (start, code.split_whitespace().collect::<Vec<_>>().as_slice()) {
            (None, ["require", "("]) => start = Some(i),
            (Some(s), [")"]) => {
                out.push((s, i));
                start = None;
            }
            _ => {}
        }
    }
    out
}

/// go.sum: every line of both, once, sorted as go writes it.
pub fn go_sum(node: &str, release: &str) -> String {
    let mut lines: Vec<&str> = node.lines().chain(release.lines()).filter(|l| !l.trim().is_empty()).collect();
    lines.sort_unstable();
    lines.dedup();
    if lines.is_empty() {
        return String::new();
    }
    lines.join("\n") + "\n"
}

/// Module versions as Go orders them (semver: `v1.2.3`, prereleases and pseudo-versions below their release).
pub fn version_cmp(a: &str, b: &str) -> Ordering {
    fn parts(v: &str) -> (Vec<u64>, Option<&str>) {
        let v = v.trim_start_matches('v').split('+').next().unwrap_or("");
        let (core, pre) = match v.split_once('-') {
            Some((c, p)) => (c, Some(p)),
            None => (v, None),
        };
        (core.split('.').map(|n| n.parse().unwrap_or(0)).collect(), pre)
    }
    let ((ca, pa), (cb, pb)) = (parts(a), parts(b));
    if let Some(o) = (0..3).map(|i| ca.get(i).unwrap_or(&0).cmp(cb.get(i).unwrap_or(&0))).find(|o| o.is_ne()) {
        return o;
    }
    match (pa, pb) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(x), Some(y)) => {
            for (i, j) in x.split('.').zip(y.split('.')) {
                let o = match (i.parse::<u64>(), j.parse::<u64>()) {
                    (Ok(m), Ok(n)) => m.cmp(&n),
                    (Ok(_), Err(_)) => Ordering::Less,
                    (Err(_), Ok(_)) => Ordering::Greater,
                    (Err(_), Err(_)) => i.cmp(j),
                };
                if o.is_ne() {
                    return o;
                }
            }
            x.split('.').count().cmp(&y.split('.').count())
        }
    }
}

/// Go's own versions (`1.26`, `1.26.2`, `go1.27rc1`): by number, a release candidate or beta below its release.
pub fn go_version_cmp(a: &str, b: &str) -> Ordering {
    fn parts(v: &str) -> (Vec<u64>, bool) {
        let v = v.trim_start_matches("go");
        let cut = v.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(v.len());
        (v[..cut].split('.').map(|n| n.parse().unwrap_or(0)).collect(), cut < v.len())
    }
    let ((na, pre_a), (nb, pre_b)) = (parts(a), parts(b));
    (0..3)
        .map(|i| na.get(i).unwrap_or(&0).cmp(nb.get(i).unwrap_or(&0)))
        .find(|o| o.is_ne())
        .unwrap_or_else(|| pre_b.cmp(&pre_a))
}

// --------------------------------------------------------------------------------------------- the apps, and Go

/// The apps in the folder: each `cmd/<name>` with Go in it, sorted, but not the extension's own `_starter`.
pub fn tuis(dest: &Path) -> Vec<String> {
    let has_go = |dir: &Path| {
        fs::read_dir(dir).into_iter().flatten().flatten().any(|f| f.path().extension().is_some_and(|e| e == "go"))
    };
    let mut names: Vec<String> = fs::read_dir(dest.join("cmd"))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| has_go(&e.path()))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| !n.starts_with(['_', '.']))
        .collect();
    names.sort();
    names
}

/// Go on this machine: on the PATH, else the one this extension fetched (`go_dir`), else where Homebrew, Go's own
/// installer, Nix and the system put it.
pub fn go(path_var: Option<&str>, home: &Path, fetched: &Path) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = path_var.map(|p| std::env::split_paths(p).collect()).unwrap_or_default();
    dirs.extend([
        fetched.join("bin"),
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/go/bin"),
        PathBuf::from("/usr/local/bin"),
        home.join(".nix-profile/bin"),
        PathBuf::from("/run/current-system/sw/bin"),
        PathBuf::from("/usr/bin"),
    ]);
    dirs.into_iter().map(|d| d.join("go")).find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    const GO_MOD: &str = "module tuis\n\ngo 1.26\n\nrequire (\n\tcharm.land/bubbletea/v2 v2.1.0\n\tcharm.land/lipgloss/v2 v2.0.0\n\tgithub.com/x/ansi v0.4.0 // indirect\n)\n";

    fn seed(root: &Path) -> PathBuf {
        let s = root.join("seed");
        fs::create_dir_all(s.join("kit")).unwrap();
        fs::create_dir_all(s.join("cmd/_starter")).unwrap();
        fs::write(s.join("go.mod"), GO_MOD).unwrap();
        fs::write(
            s.join("go.sum"),
            "charm.land/bubbletea/v2 v2.1.0 h1:a=\ncharm.land/bubbletea/v2 v2.1.0/go.mod h1:b=\n",
        )
        .unwrap();
        fs::write(s.join("README.md"), "readme\n").unwrap();
        fs::write(s.join("tuis.toml"), "# index\n").unwrap();
        fs::write(s.join("kit/kit.go"), "package kit // 1\n").unwrap();
        fs::write(s.join("cmd/_starter/main.go"), "package main\n").unwrap();
        s
    }

    #[test]
    fn a_new_node_gets_the_whole_folder_and_a_second_start_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (s, d) = (seed(dir.path()), dir.path().join("tuis"));
        let first = install(&s, &d).unwrap();
        assert_eq!(first.written, ["README.md", "go.mod", "go.sum", "tuis.toml", "kit/kit.go", "cmd/_starter/main.go"]);
        assert!(first.problems.is_empty(), "{first:?}");
        assert_eq!(fs::read_to_string(d.join("go.mod")).unwrap(), GO_MOD, "a new node's go.mod is the release's");
        assert!(d.join("bin").is_dir() && d.join("cmd").is_dir());
        assert!(tuis(&d).is_empty(), "the starter isn't one of the agents' apps");
        assert_eq!(install(&s, &d).unwrap(), Report::default(), "nothing changed, nothing written");
    }

    #[test]
    fn a_release_updates_the_kit_and_never_touches_the_agents_apps_builds_or_index() {
        let dir = tempfile::tempdir().unwrap();
        let (s, d) = (seed(dir.path()), dir.path().join("tuis"));
        install(&s, &d).unwrap();
        // an agent's app, its build, its index entry
        fs::create_dir_all(d.join("cmd/releases")).unwrap();
        fs::write(d.join("cmd/releases/main.go"), "package main\n").unwrap();
        fs::write(d.join("bin/releases"), "binary").unwrap();
        fs::write(d.join("tuis.toml"), "[tui.releases]\n").unwrap();
        // a new release: the kit changed, and lost a file
        fs::write(s.join("kit/kit.go"), "package kit // 2\n").unwrap();
        fs::write(d.join("kit/old.go"), "package kit // gone in this release\n").unwrap();
        let r = install(&s, &d).unwrap();
        assert_eq!(r.written, ["kit/kit.go"]);
        assert_eq!(r.removed, ["kit/old.go"]);
        assert_eq!(fs::read_to_string(d.join("tuis.toml")).unwrap(), "[tui.releases]\n", "the index is the agents'");
        assert!(d.join("cmd/releases/main.go").is_file() && d.join("bin/releases").is_file());
        assert_eq!(tuis(&d), ["releases"]);
    }

    /// 0.1.x kept a cargo workspace in the folder (Pointman thread 17421 [17682]: James moved the apps to Go): what
    /// it made goes, and so do its build folder and lock, but an agent's own crate stays.
    #[test]
    fn the_cargo_workspace_goes_but_an_agents_crate_stays() {
        let dir = tempfile::tempdir().unwrap();
        let (s, d) = (seed(dir.path()), dir.path().join("tuis"));
        for f in [
            "Cargo.toml",
            "Cargo.lock",
            "rustfmt.toml",
            ".cargo/config.toml",
            "crates/_starter/Cargo.toml",
            "crates/releases/Cargo.toml",
            "target/release/tui-releases",
            "kit/src/lib.rs",
            "kit/Cargo.toml",
        ] {
            fs::create_dir_all(d.join(f).parent().unwrap()).unwrap();
            fs::write(d.join(f), "rust").unwrap();
        }
        let r = install(&s, &d).unwrap();
        assert!(r.problems.is_empty(), "{r:?}");
        for gone in [
            "Cargo.toml",
            "Cargo.lock",
            "rustfmt.toml",
            ".cargo",
            "crates/_starter",
            "target",
            "kit/src",
            "kit/Cargo.toml",
        ] {
            assert!(!d.join(gone).exists(), "{gone} should be gone: {r:?}");
        }
        assert!(d.join("crates/releases/Cargo.toml").is_file(), "an agent's crate is its owner's to move");
        assert!(d.join("kit/kit.go").is_file());
        // and once its owner has moved it, crates/ goes too
        fs::remove_dir_all(d.join("crates/releases")).unwrap();
        assert_eq!(install(&s, &d).unwrap().removed, ["crates"]);
    }

    #[test]
    fn an_edited_kit_file_is_put_back() {
        let dir = tempfile::tempdir().unwrap();
        let (s, d) = (seed(dir.path()), dir.path().join("tuis"));
        install(&s, &d).unwrap();
        fs::write(d.join("kit/kit.go"), "package kit // changed by hand\n").unwrap();
        assert_eq!(install(&s, &d).unwrap().written, ["kit/kit.go"]);
        assert_eq!(fs::read_to_string(d.join("kit/kit.go")).unwrap(), "package kit // 1\n");
    }

    #[test]
    fn a_part_missing_from_the_release_is_said_and_the_rest_still_come() {
        let dir = tempfile::tempdir().unwrap();
        let (s, d) = (seed(dir.path()), dir.path().join("tuis"));
        fs::remove_file(s.join("go.sum")).unwrap();
        let r = install(&s, &d).unwrap();
        assert_eq!(r.problems.len(), 1, "{r:?}");
        assert!(r.problems[0].starts_with("go.sum: "), "{r:?}");
        for made in ["go.mod", "README.md", "tuis.toml", "kit/kit.go", "cmd/_starter/main.go"] {
            assert!(d.join(made).is_file(), "{made}");
        }
    }

    #[test]
    fn go_mod_raises_the_kits_needs_and_keeps_the_agents_own() {
        let node = "module tuis\n\ngo 1.25\n\nrequire (\n\tcharm.land/bubbletea/v2 v2.0.5\n\tcharm.land/lipgloss/v2 v2.3.0\n\tgithub.com/agent/thing v1.0.0\n)\n\nreplace github.com/agent/thing => ../thing\n";
        let got = go_mod(node, GO_MOD);
        assert_eq!(
            got,
            "module tuis\n\ngo 1.26\n\nrequire (\n\tcharm.land/bubbletea/v2 v2.1.0\n\tcharm.land/lipgloss/v2 v2.3.0\n\tgithub.com/agent/thing v1.0.0\n\tgithub.com/x/ansi v0.4.0 // indirect\n)\n\nreplace github.com/agent/thing => ../thing\n",
            "bubbletea raised, the agent's newer lipgloss kept, their module and replace kept, the missing one added"
        );
        assert_eq!(go_mod(&got, GO_MOD), got, "a second merge changes nothing");
        assert_eq!(go_mod("", GO_MOD), GO_MOD);
        // one-line requires, and no block to add to
        let single = "module tuis\n\ngo 1.27\n\nrequire charm.land/bubbletea/v2 v2.0.0\n";
        let got = go_mod(single, GO_MOD);
        assert!(got.contains("go 1.27\n"), "never a lower Go: {got}");
        assert!(got.contains("require charm.land/bubbletea/v2 v2.1.0\n"), "{got}");
        assert!(
            got.ends_with("require (\n\tcharm.land/lipgloss/v2 v2.0.0\n\tgithub.com/x/ansi v0.4.0 // indirect\n)\n"),
            "{got}"
        );
        // go's own layout, a block of direct ones and one of indirect ones: each new one goes into its own kind's
        let two = "module tuis\n\ngo 1.26.0\n\nrequire (\n\tcharm.land/bubbletea/v2 v2.1.0\n)\n\nrequire (\n\tgithub.com/y/z v1.0.0 // indirect\n)\n";
        assert_eq!(
            go_mod(two, GO_MOD),
            "module tuis\n\ngo 1.26.0\n\nrequire (\n\tcharm.land/bubbletea/v2 v2.1.0\n\tcharm.land/lipgloss/v2 v2.0.0\n)\n\nrequire (\n\tgithub.com/y/z v1.0.0 // indirect\n\tgithub.com/x/ansi v0.4.0 // indirect\n)\n",
            "and 1.26.0 is no lower than 1.26"
        );
    }

    #[test]
    fn go_sum_has_every_line_once() {
        assert_eq!(go_sum("b h1:2\na h1:1\n", "a h1:1\nc h1:3\n"), "a h1:1\nb h1:2\nc h1:3\n");
        assert_eq!(go_sum("", ""), "");
    }

    #[test]
    fn versions_order_as_go_orders_them() {
        use Ordering::*;
        for (a, b, want) in [
            ("v2.1.0", "v2.0.9", Greater),
            ("v1.10.0", "v1.9.0", Greater),
            ("v1.0.0", "v1.0.0-rc.1", Greater),
            ("v0.0.0-20251001120000-abcdef123456", "v0.0.0-20240101000000-123456abcdef", Greater),
            ("v1.2.3+incompatible", "v1.2.3", Equal),
            ("v1.0.0-alpha", "v1.0.0-alpha.1", Less),
        ] {
            assert_eq!(version_cmp(a, b), want, "{a} vs {b}");
        }
        for (a, b, want) in [("1.26", "1.25.7", Greater), ("1.26.0", "1.26", Equal), ("go1.27rc1", "1.27", Less)] {
            assert_eq!(go_version_cmp(a, b), want, "{a} vs {b}");
        }
    }

    #[test]
    fn the_folder_go_dir_and_go_are_found() {
        let home = Path::new("/home/x");
        assert_eq!(folder(None, home), home.join(".local/share/message-board/tuis"));
        assert_eq!(folder(Some(""), home), home.join(".local/share/message-board/tuis"));
        assert_eq!(folder(Some("/tmp/t"), home), PathBuf::from("/tmp/t"));
        assert_eq!(go_dir(None, home), home.join(".local/share/message-board/go"));
        let dir = tempfile::tempdir().unwrap();
        let (bin, fetched) = (dir.path().join("bin"), dir.path().join("go"));
        fs::create_dir_all(&bin).unwrap();
        fs::create_dir_all(fetched.join("bin")).unwrap();
        let path = bin.to_str().unwrap();
        fs::write(fetched.join("bin/go"), "").unwrap();
        assert_eq!(
            go(Some(path), dir.path(), &fetched),
            Some(fetched.join("bin/go")),
            "the fetched one, before the system's"
        );
        fs::write(bin.join("go"), "").unwrap();
        assert_eq!(go(Some(path), dir.path(), &fetched), Some(bin.join("go")), "the PATH's first");
    }
}
