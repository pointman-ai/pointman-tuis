//! Sharing the apps between machines, each machine building what it gets (EXT-36).
//!
//! The shared home is a git repo (pointman-ai/tuis, branch `main`) that holds each app's `cmd/<name>/` and the index
//! `tuis.toml`; the kit and go.mod stay the release's. Every minute each node fetches it into a clone of its own,
//! and for each app compares three things: the node's copy, the repo's, and what both were at this node's last sync
//! (its base, a hash per app: the app's files and its `[tui.<name>]` entry).
//!
//! - Changed only in the repo: copied into the node's folder, its entry merged into the node's tuis.toml, and built
//!   there (`go build -o bin/<name> ./cmd/<name>`).
//! - Changed only here: committed as `<name>: from <machine>`, by the machine, and pushed, once it's built (`bin/<name>`
//!   no older than its files), so nothing half-written goes out.
//! - Changed on both sides since the last sync: a conflict. This machine's is kept and not pushed; the repo's stays
//!   in the clone. Neither is lost, and it's said until the two are the same again.
//! - Taken out on one side and unchanged on the other: taken out on the other too (it stays in the repo's history).
//!   Taken out on one side and changed on the other: the change wins.
//!
//! A folder with no apps at all (new, or emptied) takes the repo's in and deletes nothing from it.
//!
//! The vault's GitHub token reaches git only in its environment, as a header for the shared repo's URL: never on
//! disk, in the clone's config, in a log or in an error.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant, SystemTime};

use anyhow::{bail, Context};
use base64::Engine;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use toml_edit::{DocumentMut, Item, Table};

/// The shared home: the only remote the vault's GitHub token is ever handed to.
pub const REPO: &str = "https://github.com/pointman-ai/tuis.git";
const BRANCH: &str = "main";
/// Longest a fetch or push may take, and a build.
const GIT_LIMIT: Duration = Duration::from_secs(120);
const BUILD_LIMIT: Duration = Duration::from_secs(600);

/// One node's side of the sharing. (Not Debug: it holds the token.)
#[derive(Clone)]
pub struct Sharing {
    /// The node's TUI folder.
    pub folder: PathBuf,
    /// The extension's clone of the shared repo, in its state folder: reset to the repo's `main` each round.
    pub clone: PathBuf,
    /// The repo: [`REPO`], or another one (a local one under tests).
    pub remote: String,
    /// This machine's name: commits are `<app>: from <machine>`, by it.
    pub machine: String,
    pub git: PathBuf,
    /// The index the repo starts from when it has none: the release's tuis.toml.
    pub index_seed: String,
    /// The vault's GitHub token, for [`REPO`] only, kept in memory.
    pub token: Option<String>,
    /// More git config for its runs, as `key=value` in its environment (tests point [`REPO`] at a local repo).
    pub git_config: Vec<(String, String)>,
}

/// What this node knew of each app at its last sync, kept in its state folder between rounds.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Base {
    /// Each app as it was on both sides: its hash.
    pub apps: BTreeMap<String, String>,
    /// Apps that came in and didn't build here: the hash they had, and the first line of go's error.
    pub failed: BTreeMap<String, (String, String)>,
}

impl Base {
    pub fn load(path: &Path) -> Base {
        let Ok(v) = fs::read(path).map_err(anyhow::Error::from).and_then(|b| Ok(serde_json::from_slice::<Value>(&b)?))
        else {
            return Base::default();
        };
        let text = |v: &Value| v.as_str().unwrap_or_default().to_string();
        let apps = v["apps"].as_object().into_iter().flatten().map(|(k, h)| (k.clone(), text(h))).collect();
        let failed = v["failed"]
            .as_object()
            .into_iter()
            .flatten()
            .map(|(k, f)| (k.clone(), (text(&f["hash"]), text(&f["error"]))))
            .collect();
        Base { apps, failed }
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        let failed: serde_json::Map<String, Value> =
            self.failed.iter().map(|(k, (hash, error))| (k.clone(), json!({"hash": hash, "error": error}))).collect();
        let body = serde_json::to_vec_pretty(&json!({"apps": self.apps, "failed": failed}))?;
        write_atomic(path, &body)
    }
}

/// What a sync did, by app.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Synced {
    /// Copied in from the repo (and built).
    pub came_in: Vec<String>,
    /// Pushed to the repo.
    pub went_out: Vec<String>,
    /// Taken out of the repo, so taken out here.
    pub removed_here: Vec<String>,
    /// Taken out here, so taken out of the repo.
    pub removed_there: Vec<String>,
    /// Changed here and in the repo since the last sync: this machine's kept, and not pushed.
    pub conflicts: Vec<String>,
    /// Changed here since it was last built: it goes out once it's built.
    pub unbuilt: Vec<String>,
    /// The apps in the repo after it.
    pub shared: Vec<String>,
}

/// What's done with one app, from its hash here, in the repo and at the last sync.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Act {
    Same,
    In,
    RemoveHere,
    Out,
    RemoveThere,
    Conflict,
}

pub fn decide(here: Option<&str>, there: Option<&str>, base: Option<&str>) -> Act {
    if here == there {
        Act::Same
    } else if here == base {
        if there.is_some() {
            Act::In
        } else {
            Act::RemoveHere
        }
    } else if there == base {
        if here.is_some() {
            Act::Out
        } else {
            Act::RemoveThere
        }
    } else {
        match (here, there) {
            // taken out on one side, changed on the other: the change wins, and nothing is lost
            (None, Some(_)) => Act::In,
            (Some(_), None) => Act::Out,
            _ => Act::Conflict,
        }
    }
}

/// A push the repo refused because another machine pushed first: the round is done again from its new `main`.
#[derive(Debug)]
struct Rejected;

impl std::fmt::Display for Rejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("another machine pushed first")
    }
}

impl std::error::Error for Rejected {}

/// Sync the node's folder with the repo: one round, done again (up to twice more) when another machine pushed
/// between its fetch and its push. `build` builds one app in the node's folder, or says why it couldn't.
pub fn sync(s: &Sharing, base: &mut Base, build: &mut dyn FnMut(&str) -> Result<(), String>) -> anyhow::Result<Synced> {
    let mut synced = Synced::default();
    for attempt in 0.. {
        match round(s, base, build, &mut synced) {
            Err(e) if e.is::<Rejected>() && attempt < 2 => continue,
            Err(e) => return Err(anyhow::anyhow!(scrub(&format!("{e:#}"), s.token.as_deref()))),
            Ok(()) => break,
        }
    }
    Ok(synced)
}

fn round(
    s: &Sharing,
    base: &mut Base,
    build: &mut dyn FnMut(&str) -> Result<(), String>,
    synced: &mut Synced,
) -> anyhow::Result<()> {
    let git = Git::new(s);
    git.fetch()?;
    synced.unbuilt.clear();
    let here = Side::read(&s.folder)?;
    let mut there = Side::read(&s.clone)?;
    if there.index.as_table().is_empty() && !s.clone.join("tuis.toml").exists() {
        there.index = s.index_seed.parse().unwrap_or_default();
    }

    // a folder with no apps at all is new or emptied: the repo's come in, and nothing is taken out of it
    let empty = Base::default();
    let known = if here.apps.is_empty() { &empty } else { &*base };
    let names: BTreeSet<&String> = here.apps.keys().chain(there.apps.keys()).chain(known.apps.keys()).collect();
    let mut acts = BTreeMap::new();
    for name in names {
        let get = |m: &BTreeMap<String, String>| m.get(name).cloned();
        let (h, t, b) = (get(&here.apps), get(&there.apps), get(&known.apps));
        let act = match decide(h.as_deref(), t.as_deref(), b.as_deref()) {
            Act::Out if !built(&s.folder, name) => {
                synced.unbuilt.push(name.clone());
                continue;
            }
            act => act,
        };
        acts.insert(name.clone(), (act, h, t));
    }
    synced.conflicts = acts.iter().filter(|(_, (a, _, _))| *a == Act::Conflict).map(|(n, _)| n.clone()).collect();

    // the repo's changes, into the node's folder: each app's files, then its entry, then what it's known as
    let none = DocumentMut::new();
    let mut came_in = Vec::new();
    for (name, (act, h, t)) in &acts {
        match act {
            Act::Same => {
                match h {
                    Some(hash) => base.apps.insert(name.clone(), hash.clone()),
                    None => base.apps.remove(name),
                };
            }
            Act::In => {
                mirror(&s.clone.join("cmd").join(name), &s.folder.join("cmd").join(name))
                    .with_context(|| format!("copying {name} in"))?;
                put_entry(&s.folder, &there.index, name)?;
                base.apps.insert(name.clone(), t.clone().unwrap_or_default());
                came_in.push(name.clone());
            }
            Act::RemoveHere => {
                remove(&s.folder.join("cmd").join(name)).with_context(|| format!("taking {name} out"))?;
                remove(&s.folder.join("bin").join(name)).with_context(|| format!("taking {name} out"))?;
                put_entry(&s.folder, &none, name)?;
                base.apps.remove(name);
                base.failed.remove(name);
                synced.removed_here.push(name.clone());
            }
            _ => {}
        }
    }

    // built here: what came in, and what didn't build before (a Go still being fetched, a module go couldn't get)
    let retry: Vec<String> = base
        .failed
        .iter()
        .filter(|(name, (hash, _))| !came_in.contains(name) && here.apps.get(*name) == Some(hash))
        .map(|(name, _)| name.clone())
        .collect();
    base.failed.retain(|name, _| retry.contains(name));
    for name in came_in.iter().chain(&retry) {
        match build(name) {
            Ok(()) => {
                base.failed.remove(name);
            }
            Err(e) => {
                let hash = base.apps.get(name).cloned().unwrap_or_default();
                base.failed.insert(name.clone(), (hash, e));
            }
        }
    }
    synced.came_in.extend(came_in);

    // this machine's changes, out to the repo: a commit each
    let mut out = Vec::new();
    for (name, (act, h, _)) in &acts {
        let (dest, gone) = (s.clone.join("cmd").join(name), matches!(act, Act::RemoveThere));
        match act {
            Act::Out => mirror(&s.folder.join("cmd").join(name), &dest)?,
            Act::RemoveThere => remove(&dest)?,
            _ => continue,
        }
        set_entry(&mut there.index, if gone { &none } else { &here.index }, name);
        write_atomic(&s.clone.join("tuis.toml"), there.index.to_string().as_bytes())?;
        let verb = if gone { "taken out on" } else { "from" };
        if git.commit(name, &format!("{name}: {verb} {}", s.machine))? {
            out.push((name.clone(), h.clone(), gone));
        }
    }
    if !out.is_empty() {
        git.push()?;
    }
    for (name, hash, gone) in out {
        if gone {
            // taken out here: its entry goes with it
            put_entry(&s.folder, &none, &name)?;
        }
        match hash {
            Some(hash) => base.apps.insert(name.clone(), hash),
            None => base.apps.remove(&name),
        };
        if gone {
            synced.removed_there.push(name);
        } else {
            synced.went_out.push(name);
        }
    }
    synced.shared = crate::tuis(&s.clone);
    Ok(())
}

/// One app's entry in the node's index made `from`'s (or taken out), the index read just before it's written, so an
/// agent's edit since the round began stays.
fn put_entry(folder: &Path, from: &DocumentMut, name: &str) -> anyhow::Result<()> {
    let path = folder.join("tuis.toml");
    let text = fs::read_to_string(&path).unwrap_or_default();
    let mut index: DocumentMut = text.parse().with_context(|| format!("{} doesn't parse", path.display()))?;
    set_entry(&mut index, from, name);
    if index.to_string() != text {
        write_atomic(&path, index.to_string().as_bytes())?;
    }
    Ok(())
}

/// The token's every form, out of a text (git's errors don't carry it, but nothing that's shown may).
fn scrub(text: &str, token: Option<&str>) -> String {
    match token.filter(|t| !t.is_empty()) {
        Some(t) => text.replace(&basic(t), "…").replace(t, "…"),
        None => text.to_string(),
    }
}

fn basic(token: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(format!("x-access-token:{token}"))
}

// ------------------------------------------------------------------------------------------------- the two sides

/// A folder's apps (each with its hash) and its index.
struct Side {
    apps: BTreeMap<String, String>,
    index: DocumentMut,
}

impl Side {
    fn read(root: &Path) -> anyhow::Result<Side> {
        let path = root.join("tuis.toml");
        let text = fs::read_to_string(&path).unwrap_or_default();
        let index: DocumentMut = text.parse().with_context(|| format!("{} doesn't parse", path.display()))?;
        let mut apps = BTreeMap::new();
        for name in crate::tuis(root) {
            let hash = app_hash(&root.join("cmd").join(&name), &entry_text(&index, &name))?;
            apps.insert(name, hash);
        }
        Ok(Side { apps, index })
    }
}

/// An app's hash: its files (by path, not hidden ones) and its entry in the index.
pub fn app_hash(dir: &Path, entry: &str) -> io::Result<String> {
    let mut hash = Sha256::new();
    for rel in files(dir)? {
        let body = fs::read(dir.join(&rel))?;
        hash.update(format!("{rel}\0{}\0", body.len()));
        hash.update(&body);
    }
    hash.update(format!("tuis.toml\0{entry}"));
    Ok(hash.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// The files in an app's folder, relative and sorted: no hidden ones (an editor's, the Finder's) and no links.
fn files(dir: &Path) -> io::Result<Vec<String>> {
    fn walk(dir: &Path, rel: &str, out: &mut Vec<String>) -> io::Result<()> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || name.ends_with(".part") {
                continue;
            }
            let child = if rel.is_empty() { name } else { format!("{rel}/{name}") };
            let kind = entry.file_type()?;
            if kind.is_dir() {
                walk(&entry.path(), &child, out)?;
            } else if kind.is_file() {
                out.push(child);
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(dir, "", &mut out)?;
    out.sort();
    Ok(out)
}

/// `to` made like `from`: each file written only if it differs (so an unchanged one keeps its time, and go has
/// nothing to rebuild for it), and what `from` hasn't got taken out. Hidden files are left alone on both sides.
fn mirror(from: &Path, to: &Path) -> io::Result<()> {
    let want = files(from)?;
    for rel in &want {
        let body = fs::read(from.join(rel))?;
        if fs::read(to.join(rel)).ok().as_deref() != Some(&body[..]) {
            write_atomic(&to.join(rel), &body)?;
        }
    }
    if to.is_dir() {
        for rel in files(to)? {
            if !want.contains(&rel) {
                fs::remove_file(to.join(&rel))?;
            }
        }
        prune(to)?;
    }
    Ok(())
}

/// Empty folders left in an app once its files have gone.
fn prune(dir: &Path) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            prune(&entry.path())?;
            if fs::read_dir(entry.path())?.next().is_none() {
                fs::remove_dir(entry.path())?;
            }
        }
    }
    Ok(())
}

fn remove(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
        Ok(m) if m.is_dir() => fs::remove_dir_all(path),
        Ok(_) => fs::remove_file(path),
    }
}

fn write_atomic(to: &Path, body: &[u8]) -> io::Result<()> {
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut part = to.as_os_str().to_owned();
    part.push(".part");
    fs::write(&part, body)?;
    fs::rename(&part, to)
}

/// Built since it last changed: `bin/<name>` is no older than any of its files.
fn built(folder: &Path, name: &str) -> bool {
    let modified = |p: &Path| fs::metadata(p).and_then(|m| m.modified()).ok();
    let Some(bin) = modified(&folder.join("bin").join(name)) else { return false };
    let dir = folder.join("cmd").join(name);
    let newest: Option<SystemTime> =
        files(&dir).unwrap_or_default().iter().filter_map(|rel| modified(&dir.join(rel))).max();
    newest.is_none_or(|t| t <= bin)
}

// --------------------------------------------------------------------------------------------------- the index

/// An app's table in an index: `[tui.<name>]`, or an inline one.
fn entry<'d>(index: &'d DocumentMut, name: &str) -> Option<&'d Item> {
    index.get("tui")?.get(name)
}

/// An app's entry as it's compared: its keys and values, without comments or layout. Empty when it has none.
pub fn entry_text(index: &DocumentMut, name: &str) -> String {
    let Some(item) = entry(index, name) else { return String::new() };
    let mut table = match item.clone().into_table() {
        Ok(t) => t,
        Err(other) => return other.to_string(),
    };
    table.fmt();
    table.decor_mut().clear();
    let mut tui = Table::new();
    tui.set_implicit(true);
    tui.insert(name, Item::Table(table));
    let mut doc = DocumentMut::new();
    doc.insert("tui", Item::Table(tui));
    doc.to_string()
}

/// `name`'s entry in `into` made `from`'s (taken out when `from` has none), and the rest of `into` left as it was:
/// a changed entry stays where it was, with the comments above it; a new one goes after the others.
pub fn set_entry(into: &mut DocumentMut, from: &DocumentMut, name: &str) {
    let new = entry(from, name).cloned();
    if new.is_none() && entry(into, name).is_none() {
        return;
    }
    // an index of only comments keeps them after its last table: the first one goes below them
    let mut above = "\n".to_string();
    if into.is_empty() {
        let comments = into.trailing().as_str().unwrap_or_default().trim_end_matches('\n').to_string();
        if !comments.is_empty() {
            above = format!("{comments}\n\n");
            into.set_trailing("");
        }
    }
    let tui = into.entry("tui").or_insert_with(|| {
        let mut t = Table::new();
        t.set_implicit(true);
        Item::Table(t)
    });
    let Some(tui) = tui.as_table_mut() else { return };
    match new {
        Some(mut item) => {
            if let Some(table) = item.as_table_mut() {
                let old = tui.get(name).and_then(Item::as_table);
                table.set_position(old.and_then(Table::position));
                let prefix = old.and_then(|o| o.decor().prefix()).and_then(|p| p.as_str()).unwrap_or(&above);
                table.decor_mut().set_prefix(prefix.to_string());
            }
            tui.insert(name, item);
        }
        None => {
            let Some(Item::Table(old)) = tui.remove(name) else { return };
            // the comments above it (the index's own, when it was the first) stay, on the next one or at the end
            let prefix = old.decor().prefix().and_then(|p| p.as_str()).unwrap_or_default().to_string();
            if !prefix.contains('#') {
                return;
            }
            let at = old.position().unwrap_or(isize::MAX);
            let next = tui
                .iter_mut()
                .filter_map(|(_, item)| item.as_table_mut())
                .filter(|t| t.position().is_some_and(|p| p > at))
                .min_by_key(|t| t.position());
            match next {
                Some(t) => t.decor_mut().set_prefix(prefix),
                None => {
                    let end = into.trailing().as_str().unwrap_or_default().to_string();
                    into.set_trailing(format!("{}\n{end}", prefix.trim_end_matches('\n')));
                }
            }
        }
    }
}

// ------------------------------------------------------------------------------------------------------- git

struct Git<'a> {
    s: &'a Sharing,
    env: Vec<(String, String)>,
}

impl<'a> Git<'a> {
    fn new(s: &'a Sharing) -> Git<'a> {
        let mut env: Vec<(String, String)> = [
            // only what's given here: no system or user config, helpers or hooks, and never a prompt
            ("GIT_CONFIG_NOSYSTEM", "1"),
            ("GIT_CONFIG_GLOBAL", "/dev/null"),
            ("GIT_TERMINAL_PROMPT", "0"),
            ("GIT_HTTP_LOW_SPEED_LIMIT", "1000"),
            ("GIT_HTTP_LOW_SPEED_TIME", "30"),
            ("GIT_AUTHOR_NAME", &s.machine),
            ("GIT_COMMITTER_NAME", &s.machine),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        let email = format!("tuis@{}", s.machine.replace(char::is_whitespace, "-"));
        env.push(("GIT_AUTHOR_EMAIL".into(), email.clone()));
        env.push(("GIT_COMMITTER_EMAIL".into(), email));
        let mut config = s.git_config.clone();
        config.extend(auth_config(&s.remote, s.token.as_deref()));
        env.push(("GIT_CONFIG_COUNT".into(), config.len().to_string()));
        for (i, (k, v)) in config.into_iter().enumerate() {
            env.push((format!("GIT_CONFIG_KEY_{i}"), k));
            env.push((format!("GIT_CONFIG_VALUE_{i}"), v));
        }
        Git { s, env }
    }

    fn run(&self, args: &[&str], limit: Duration) -> anyhow::Result<Output> {
        let mut cmd = Command::new(&self.s.git);
        cmd.current_dir(&self.s.clone).args(args).envs(self.env.iter().map(|(k, v)| (k, v)));
        for k in [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_INDEX_FILE",
            "GIT_ASKPASS",
            "SSH_ASKPASS",
            "GIT_TRACE",
            "GIT_TRACE_CURL",
            "GIT_TRACE_PACKET",
            "GIT_CURL_VERBOSE",
        ] {
            cmd.env_remove(k);
        }
        run(&mut cmd, limit).with_context(|| format!("git {}", args[0]))
    }

    /// git, and its output; an error with git's own first words when it fails.
    fn git(&self, args: &[&str]) -> anyhow::Result<String> {
        let out = self.run(args, GIT_LIMIT)?;
        if !out.status.success() {
            bail!("git {}: {}", args[0], said(&out.stderr));
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    /// The repo's `main` into the clone, and the clone's files made exactly its (or none, when it has no `main`).
    fn fetch(&self) -> anyhow::Result<()> {
        let s = self.s;
        if !s.clone.join(".git").is_dir() {
            fs::create_dir_all(&s.clone).with_context(|| format!("making {}", s.clone.display()))?;
            self.git(&["init", "-q"])?;
            self.git(&["remote", "add", "origin", &s.remote])?;
        }
        if self.git(&["remote", "get-url", "origin"])?.trim() != s.remote {
            self.git(&["remote", "set-url", "origin", &s.remote])?;
        }
        let refspec = format!("+refs/heads/{BRANCH}:refs/remotes/origin/{BRANCH}");
        let out = self.run(&["fetch", "-q", "--no-tags", "origin", &refspec], GIT_LIMIT)?;
        let missing = String::from_utf8_lossy(&out.stderr).contains("couldn't find remote ref");
        if out.status.success() {
            self.git(&["checkout", "-q", "-f", "-B", BRANCH, &format!("refs/remotes/origin/{BRANCH}")])?;
        } else if missing {
            // a new repo: start main from nothing
            self.git(&["symbolic-ref", "HEAD", &format!("refs/heads/{BRANCH}")])?;
            let _ = self.run(&["update-ref", "-d", &format!("refs/heads/{BRANCH}")], GIT_LIMIT);
            self.git(&["read-tree", "--empty"])?;
        } else {
            bail!("fetching {}: {}", s.remote, said(&out.stderr));
        }
        self.git(&["clean", "-q", "-f", "-d", "-x"])?;
        Ok(())
    }

    /// One app's change committed; false when there was nothing to commit.
    fn commit(&self, name: &str, message: &str) -> anyhow::Result<bool> {
        self.git(&["add", "-A", "--", &format!("cmd/{name}"), "tuis.toml"])?;
        if self.run(&["diff", "--cached", "--quiet"], GIT_LIMIT)?.status.success() {
            return Ok(false);
        }
        self.git(&["commit", "-q", "--no-verify", "-m", message])?;
        Ok(true)
    }

    fn push(&self) -> anyhow::Result<()> {
        let out = self.run(&["push", "-q", "origin", &format!("HEAD:refs/heads/{BRANCH}")], GIT_LIMIT)?;
        if out.status.success() {
            return Ok(());
        }
        let err = String::from_utf8_lossy(&out.stderr);
        if ["[rejected]", "non-fast-forward", "fetch first", "cannot lock ref"].iter().any(|w| err.contains(w)) {
            return Err(Rejected.into());
        }
        bail!("pushing to {}: {}", self.s.remote, said(&out.stderr));
    }
}

/// The token as git's config, for [`REPO`] only: a header on its requests, in git's environment.
pub fn auth_config(remote: &str, token: Option<&str>) -> Vec<(String, String)> {
    match token.filter(|t| !t.is_empty() && remote == REPO) {
        Some(t) => vec![(format!("http.{REPO}.extraHeader"), format!("Authorization: Basic {}", basic(t)))],
        None => vec![],
    }
}

/// A program's own words for what went wrong: its first line that says something.
fn said(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#'))
        .unwrap_or("failed, and said nothing");
    line.trim_start_matches("fatal: ").trim_start_matches("error: ").to_string()
}

/// A program run to its end, or stopped after `limit`.
fn run(cmd: &mut Command, limit: Duration) -> anyhow::Result<Output> {
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
    let drain = |pipe: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut p) = pipe {
                let _ = p.read_to_end(&mut buf);
            }
            buf
        })
    };
    let out = drain(child.stdout.take().map(|p| Box::new(p) as Box<dyn Read + Send>));
    let err = drain(child.stderr.take().map(|p| Box::new(p) as Box<dyn Read + Send>));
    let until = Instant::now() + limit;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() > until {
            let _ = child.kill();
            let _ = child.wait();
            bail!("stopped after {} s", limit.as_secs());
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    Ok(Output { status, stdout: out.join().unwrap_or_default(), stderr: err.join().unwrap_or_default() })
}

/// One app built in the node's folder with this machine's Go, as an agent builds it; else go's first words.
pub fn go_build(go: &Path, folder: &Path, name: &str) -> Result<(), String> {
    let mut cmd = Command::new(go);
    cmd.current_dir(folder).args(["build", "-o", &format!("bin/{name}"), &format!("./cmd/{name}")]);
    match run(&mut cmd, BUILD_LIMIT) {
        Ok(out) if out.status.success() => Ok(()),
        Ok(out) => Err(said(&out.stderr)),
        Err(e) => Err(format!("{e:#}")),
    }
}

/// git on this machine: on the PATH, else where Homebrew, Nix and the system put it.
pub fn find_git(path_var: Option<&str>, home: &Path) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = path_var.map(|p| std::env::split_paths(p).collect()).unwrap_or_default();
    dirs.extend([
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        home.join(".nix-profile/bin"),
        PathBuf::from("/run/current-system/sw/bin"),
        PathBuf::from("/usr/bin"),
    ]);
    dirs.into_iter().map(|d| d.join("git")).find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sh(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .current_dir(dir)
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    const HEADER: &str = "# This node's TUIs, one table each.\n#\n# [tui.<name>]\n# title = \"What it shows\"\n";

    /// A machine: its TUI folder, its clone, its base, and the apps its builds were asked for.
    struct Machine {
        s: Sharing,
        base: Base,
        built: Vec<String>,
    }

    impl Machine {
        fn new(root: &Path, name: &str, remote: &Path) -> Machine {
            let folder = root.join(name).join("tuis");
            fs::create_dir_all(folder.join("cmd/_starter")).unwrap();
            fs::create_dir_all(folder.join("bin")).unwrap();
            fs::write(folder.join("cmd/_starter/main.go"), "package main // starter\n").unwrap();
            fs::write(folder.join("tuis.toml"), HEADER).unwrap();
            let s = Sharing {
                folder,
                clone: root.join(name).join("state/shared"),
                remote: remote.display().to_string(),
                machine: name.into(),
                git: PathBuf::from("git"),
                index_seed: HEADER.into(),
                token: None,
                git_config: vec![],
            };
            Machine { s, base: Base::default(), built: vec![] }
        }

        /// An app made (or changed) here, its entry in the index, and built.
        fn make(&self, name: &str, body: &str) {
            let dir = self.s.folder.join("cmd").join(name);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("main.go"), format!("package main // {body}\n")).unwrap();
            let path = self.s.folder.join("tuis.toml");
            let mut index: DocumentMut = fs::read_to_string(&path).unwrap().parse().unwrap();
            let from: DocumentMut =
                format!("[tui.{name}]\ntitle = \"{body}\"\ncommand = \"bin/{name}\"\n").parse().unwrap();
            set_entry(&mut index, &from, name);
            fs::write(&path, index.to_string()).unwrap();
            self.build(name);
        }

        fn build(&self, name: &str) {
            std::thread::sleep(Duration::from_millis(5));
            fs::write(self.s.folder.join("bin").join(name), "binary").unwrap();
        }

        fn sync(&mut self) -> Synced {
            let folder = self.s.folder.clone();
            let built = &mut self.built;
            let mut build = |name: &str| {
                built.push(name.to_string());
                if name.contains("broken") {
                    return Err("cmd/broken/main.go:1:1: expected 'package'".to_string());
                }
                fs::write(folder.join("bin").join(name), "binary").map_err(|e| e.to_string())
            };
            sync(&self.s, &mut self.base, &mut build).unwrap()
        }

        fn app(&self, name: &str) -> Option<String> {
            fs::read_to_string(self.s.folder.join("cmd").join(name).join("main.go")).ok()
        }

        fn index(&self) -> String {
            fs::read_to_string(self.s.folder.join("tuis.toml")).unwrap()
        }
    }

    fn remote(root: &Path) -> PathBuf {
        let r = root.join("remote.git");
        fs::create_dir_all(&r).unwrap();
        sh(&r, &["init", "-q", "--bare", "-b", "main"]);
        r
    }

    /// The repo's files on main, and its commits' subjects and authors, newest first.
    fn repo(remote: &Path) -> (Vec<String>, Vec<String>) {
        let has_main = Command::new("git").current_dir(remote).args(["rev-parse", "-q", "--verify", "main"]).output();
        if !has_main.unwrap().status.success() {
            return (vec![], vec![]);
        }
        let files = sh(remote, &["ls-tree", "-r", "--name-only", "main"]).lines().map(String::from).collect();
        let log = sh(remote, &["log", "--format=%s by %an <%ae>", "main"]).lines().map(String::from).collect();
        (files, log)
    }

    #[test]
    fn decisions() {
        use Act::*;
        let (a, b, c) = (Some("a"), Some("b"), Some("c"));
        for (here, there, base, want) in [
            (a, a, None, Same),
            (None, None, a, Same),
            (a, b, a, In),
            (None, a, None, In),
            (a, None, a, RemoveHere),
            (b, a, a, Out),
            (a, None, None, Out),
            (None, a, a, RemoveThere),
            (b, c, a, Conflict),
            (a, b, None, Conflict),
            (None, b, a, In),
            (b, None, a, Out),
        ] {
            assert_eq!(decide(here, there, base), want, "{here:?} {there:?} {base:?}");
        }
    }

    #[test]
    fn an_app_made_here_goes_out_and_comes_in_built_on_the_other_machine() {
        let dir = tempfile::tempdir().unwrap();
        let r = remote(dir.path());
        let (mut mac, mut nixos) = (Machine::new(dir.path(), "mac-m4", &r), Machine::new(dir.path(), "nixos", &r));

        mac.make("hello", "hi");
        let got = mac.sync();
        assert_eq!(got.went_out, ["hello"], "{got:?}");
        assert_eq!(got.shared, ["hello"]);
        let (files, log) = repo(&r);
        assert_eq!(
            files,
            ["cmd/hello/main.go", "tuis.toml"],
            "only the app and the index: never the starter or a build"
        );
        assert_eq!(log, ["hello: from mac-m4 by mac-m4 <tuis@mac-m4>"]);
        assert!(mac.built.is_empty(), "its own app isn't built again");

        // nixos has an app of its own, with its own entry
        nixos.make("disks", "df");
        let got = nixos.sync();
        assert_eq!(
            (got.came_in.as_slice(), got.went_out.as_slice()),
            (&["hello".to_string()][..], &["disks".to_string()][..])
        );
        assert_eq!(nixos.built, ["hello"], "what came in is built there");
        assert_eq!(nixos.app("hello").unwrap(), "package main // hi\n");
        let index = nixos.index();
        assert!(index.starts_with(HEADER), "the node's own comments stay: {index}");
        assert!(
            index.contains("[tui.disks]\ntitle = \"df\"") && index.contains("[tui.hello]\ntitle = \"hi\""),
            "{index}"
        );

        // and back on the Mac
        let got = mac.sync();
        assert_eq!(got.came_in, ["disks"]);
        assert_eq!(got.shared, ["disks", "hello"]);
        assert_eq!(mac.app("disks").unwrap(), "package main // df\n");
        assert!(mac.index().contains("[tui.disks]"));

        // nothing changed: nothing happens, on either
        assert_eq!(mac.sync(), Synced { shared: vec!["disks".into(), "hello".into()], ..Synced::default() });
        assert_eq!(nixos.sync().went_out, Vec::<String>::new());
        assert_eq!(repo(&r).1.len(), 2);
    }

    #[test]
    fn a_change_comes_in_and_only_the_changed_files_are_written() {
        let dir = tempfile::tempdir().unwrap();
        let r = remote(dir.path());
        let (mut mac, mut nixos) = (Machine::new(dir.path(), "mac-m4", &r), Machine::new(dir.path(), "nixos", &r));
        mac.make("hello", "hi");
        fs::write(mac.s.folder.join("cmd/hello/parse.go"), "package main // parse\n").unwrap();
        mac.build("hello");
        mac.sync();
        nixos.sync();
        let kept = nixos.s.folder.join("cmd/hello/parse.go");
        let before = fs::metadata(&kept).unwrap().modified().unwrap();

        // the Mac changes main.go and drops a file
        std::thread::sleep(Duration::from_millis(20));
        fs::write(mac.s.folder.join("cmd/hello/main.go"), "package main // hi again\n").unwrap();
        fs::create_dir_all(mac.s.folder.join("cmd/hello/testdata")).unwrap();
        fs::write(mac.s.folder.join("cmd/hello/testdata/a.txt"), "a").unwrap();
        mac.build("hello");
        assert_eq!(mac.sync().went_out, ["hello"]);
        let got = nixos.sync();
        assert_eq!(got.came_in, ["hello"]);
        assert_eq!(nixos.app("hello").unwrap(), "package main // hi again\n");
        assert!(nixos.s.folder.join("cmd/hello/testdata/a.txt").is_file());
        assert_eq!(fs::metadata(&kept).unwrap().modified().unwrap(), before, "an unchanged file isn't written again");

        // and a file taken out goes on the other side too
        fs::remove_dir_all(mac.s.folder.join("cmd/hello/testdata")).unwrap();
        mac.build("hello");
        mac.sync();
        nixos.sync();
        assert!(!nixos.s.folder.join("cmd/hello/testdata").exists());
    }

    #[test]
    fn a_change_on_both_sides_is_a_conflict_and_this_machines_is_kept_and_not_pushed() {
        let dir = tempfile::tempdir().unwrap();
        let r = remote(dir.path());
        let (mut mac, mut nixos) = (Machine::new(dir.path(), "mac-m4", &r), Machine::new(dir.path(), "nixos", &r));
        mac.make("hello", "one");
        mac.sync();
        nixos.sync();

        mac.make("hello", "mac's");
        assert_eq!(mac.sync().went_out, ["hello"]);
        nixos.make("hello", "nixos's");
        let got = nixos.sync();
        assert_eq!(got.conflicts, ["hello"], "{got:?}");
        assert!(got.went_out.is_empty() && got.came_in.is_empty(), "{got:?}");
        assert_eq!(nixos.app("hello").unwrap(), "package main // nixos's\n", "this machine's is kept");
        assert!(nixos.index().contains("title = \"nixos's\""));
        let shared = fs::read_to_string(nixos.s.clone.join("cmd/hello/main.go")).unwrap();
        assert_eq!(shared, "package main // mac's\n", "and the shared one is in the clone");
        assert_eq!(repo(&r).1[0], "hello: from mac-m4 by mac-m4 <tuis@mac-m4>", "nothing pushed");
        assert_eq!(nixos.sync().conflicts, ["hello"], "said until it's settled");

        // settled by taking the shared one: in sync again, and nothing goes out
        fs::copy(nixos.s.clone.join("cmd/hello/main.go"), nixos.s.folder.join("cmd/hello/main.go")).unwrap();
        let mut index: DocumentMut = nixos.index().parse().unwrap();
        let shared: DocumentMut = fs::read_to_string(nixos.s.clone.join("tuis.toml")).unwrap().parse().unwrap();
        set_entry(&mut index, &shared, "hello");
        fs::write(nixos.s.folder.join("tuis.toml"), index.to_string()).unwrap();
        let got = nixos.sync();
        assert!(got.conflicts.is_empty() && got.went_out.is_empty() && got.came_in.is_empty(), "{got:?}");

        // and from then on its changes go out again
        nixos.make("hello", "nixos's again");
        assert_eq!(nixos.sync().went_out, ["hello"]);
        assert_eq!(mac.sync().came_in, ["hello"]);
    }

    #[test]
    fn an_app_taken_out_here_goes_from_the_repo_and_the_other_machines() {
        let dir = tempfile::tempdir().unwrap();
        let r = remote(dir.path());
        let (mut mac, mut nixos) = (Machine::new(dir.path(), "mac-m4", &r), Machine::new(dir.path(), "nixos", &r));
        mac.make("hello", "hi");
        mac.make("keep", "stays");
        mac.sync();
        nixos.sync();
        assert!(nixos.s.folder.join("bin/hello").is_file());

        fs::remove_dir_all(mac.s.folder.join("cmd/hello")).unwrap();
        let got = mac.sync();
        assert_eq!(got.removed_there, ["hello"], "{got:?}");
        assert_eq!(repo(&r).0, ["cmd/keep/main.go", "tuis.toml"]);
        assert_eq!(repo(&r).1[0], "hello: taken out on mac-m4 by mac-m4 <tuis@mac-m4>");
        assert!(!fs::read_to_string(mac.s.clone.join("tuis.toml")).unwrap().contains("[tui.hello]"));

        let got = nixos.sync();
        assert_eq!(got.removed_here, ["hello"], "{got:?}");
        assert!(nixos.app("hello").is_none() && !nixos.s.folder.join("bin/hello").exists());
        let index = nixos.index();
        assert!(!index.contains("[tui.hello]") && index.contains("[tui.keep]") && index.starts_with(HEADER), "{index}");
    }

    #[test]
    fn an_app_changed_since_it_was_built_waits_to_go_out() {
        let dir = tempfile::tempdir().unwrap();
        let r = remote(dir.path());
        let mut mac = Machine::new(dir.path(), "mac-m4", &r);
        let app = mac.s.folder.join("cmd/half");
        fs::create_dir_all(&app).unwrap();
        fs::write(app.join("main.go"), "package main // half-written\n").unwrap();
        let got = mac.sync();
        assert_eq!((got.unbuilt.as_slice(), got.went_out.len()), (&["half".to_string()][..], 0));
        assert_eq!(repo(&r).0, Vec::<String>::new());
        mac.build("half");
        assert_eq!(mac.sync().went_out, ["half"]);
    }

    #[test]
    fn an_emptied_folder_takes_the_apps_back_and_takes_nothing_out_of_the_repo() {
        let dir = tempfile::tempdir().unwrap();
        let r = remote(dir.path());
        let mut mac = Machine::new(dir.path(), "mac-m4", &r);
        mac.make("one", "1");
        mac.make("two", "2");
        mac.sync();
        fs::remove_dir_all(mac.s.folder.join("cmd/one")).unwrap();
        fs::remove_dir_all(mac.s.folder.join("cmd/two")).unwrap();
        let got = mac.sync();
        assert!(got.removed_there.is_empty(), "{got:?}");
        assert_eq!(got.came_in, ["one", "two"]);
        assert!(mac.app("one").is_some() && mac.app("two").is_some());
    }

    #[test]
    fn an_app_that_doesnt_build_is_said_and_tried_again() {
        let dir = tempfile::tempdir().unwrap();
        let r = remote(dir.path());
        let (mut mac, mut nixos) = (Machine::new(dir.path(), "mac-m4", &r), Machine::new(dir.path(), "nixos", &r));
        mac.make("broken", "x");
        mac.sync();
        nixos.sync();
        let (hash, error) = &nixos.base.failed["broken"];
        assert_eq!(error, "cmd/broken/main.go:1:1: expected 'package'");
        assert_eq!(hash, &nixos.base.apps["broken"]);
        nixos.sync();
        assert_eq!(nixos.built, ["broken", "broken"], "tried again each round");
        // the base survives a restart
        let path = dir.path().join("base.json");
        nixos.base.save(&path).unwrap();
        assert_eq!(Base::load(&path), nixos.base);
        assert_eq!(Base::load(&dir.path().join("none.json")), Base::default());
    }

    #[test]
    fn a_push_that_loses_the_race_is_made_again_on_the_other_machines() {
        let dir = tempfile::tempdir().unwrap();
        let r = remote(dir.path());
        let (mut mac, mut nixos) = (Machine::new(dir.path(), "mac-m4", &r), Machine::new(dir.path(), "nixos", &r));
        nixos.make("first", "1");
        nixos.sync();
        nixos.make("second", "2");
        mac.make("mine", "m");
        // while the Mac builds what came in (after its fetch, before its push), nixos pushes
        let nixos = std::cell::RefCell::new(nixos);
        let folder = mac.s.folder.clone();
        let mut build = |name: &str| {
            if name == "first" {
                nixos.borrow_mut().sync();
            }
            fs::write(folder.join("bin").join(name), "binary").map_err(|e| e.to_string())
        };
        let got = sync(&mac.s, &mut mac.base, &mut build).unwrap();
        assert_eq!(got.went_out, ["mine"], "{got:?}");
        assert_eq!(got.came_in, ["first", "second"]);
        let (files, _) = repo(&r);
        assert_eq!(files, ["cmd/first/main.go", "cmd/mine/main.go", "cmd/second/main.go", "tuis.toml"]);
    }

    #[test]
    fn the_token_is_only_in_gits_environment_and_only_for_the_shared_repo() {
        let token = "ghp_testtokentesttokentesttokentesttoken00";
        assert!(auth_config("https://github.com/someone/else.git", Some(token)).is_empty());
        assert!(auth_config(REPO, None).is_empty());
        let got = auth_config(REPO, Some(token));
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, "http.https://github.com/pointman-ai/tuis.git.extraHeader");
        assert_eq!(got[0].1, format!("Authorization: Basic {}", basic(token)));

        // a sync with REPO as its remote (fetched from a local one in its place): nothing it leaves has the token
        let dir = tempfile::tempdir().unwrap();
        let r = remote(dir.path());
        let mut mac = Machine::new(dir.path(), "mac-m4", &r);
        mac.s.remote = REPO.into();
        mac.s.token = Some(token.into());
        mac.s.git_config = vec![(format!("url.{}.insteadOf", r.display()), REPO.into())];
        mac.make("hello", "hi");
        assert_eq!(mac.sync().went_out, ["hello"]);
        let config = fs::read_to_string(mac.s.clone.join(".git/config")).unwrap();
        assert!(config.contains(REPO), "{config}");
        let mut leaks = vec![];
        fn walk(dir: &Path, needles: &[String], leaks: &mut Vec<PathBuf>) {
            for e in fs::read_dir(dir).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, needles, leaks);
                } else if let Ok(body) = fs::read(&p) {
                    let body = String::from_utf8_lossy(&body);
                    if needles.iter().any(|n| body.contains(n.as_str())) {
                        leaks.push(p);
                    }
                }
            }
        }
        walk(dir.path(), &[token.to_string(), basic(token)], &mut leaks);
        assert!(leaks.is_empty(), "the token is in {leaks:?}");

        // nor in an error
        mac.s.git_config = vec![(format!("url.{}.insteadOf", dir.path().join("nowhere").display()), REPO.into())];
        let e = sync(&mac.s, &mut mac.base, &mut |_: &str| Ok(())).unwrap_err().to_string();
        assert!(e.contains("fetching") && !e.contains(token) && !e.contains(&basic(token)), "{e}");
        assert_eq!(scrub(&format!("a {token} b {}", basic(token)), Some(token)), "a … b …");
    }

    #[test]
    fn entries_compare_by_their_values_and_merge_where_they_were() {
        let a: DocumentMut = "# mine\n\n[tui.x]\ntitle = \"X\" # a comment\ncommand    = \"bin/x\"\n".parse().unwrap();
        let b: DocumentMut = "[tui]\nx = { title = \"X\", command = \"bin/x\" }\n".parse().unwrap();
        assert_eq!(entry_text(&a, "x"), entry_text(&b, "x"), "layout and comments aren't changes");
        assert_eq!(entry_text(&a, "y"), "");
        assert_ne!(entry_text(&a, "x"), entry_text(&"[tui.x]\ntitle = \"Y\"\n".parse().unwrap(), "x"));

        let mut node: DocumentMut =
            format!("{HEADER}\n[tui.a]\ntitle = \"A\"\n\n# b's note\n[tui.b]\ntitle = \"B\"\n").parse().unwrap();
        let theirs: DocumentMut =
            "# their header\n\n[tui.b]\ntitle = \"B2\"\n\n[tui.c]\ntitle = \"C\"\n".parse().unwrap();
        set_entry(&mut node, &theirs, "b");
        set_entry(&mut node, &theirs, "c");
        assert_eq!(
            node.to_string(),
            format!(
                "{HEADER}\n[tui.a]\ntitle = \"A\"\n\n# b's note\n[tui.b]\ntitle = \"B2\"\n\n[tui.c]\ntitle = \"C\"\n"
            ),
            "b changed in place with its note, c after the others, and none of their comments"
        );
        // taking out the first keeps the index's header
        set_entry(&mut node, &theirs, "a");
        assert_eq!(
            node.to_string(),
            format!("{HEADER}\n[tui.b]\ntitle = \"B2\"\n\n[tui.c]\ntitle = \"C\"\n"),
            "a gone, its header kept"
        );
        // and the last one too
        let mut only: DocumentMut = format!("{HEADER}\n[tui.a]\ntitle = \"A\"\n").parse().unwrap();
        set_entry(&mut only, &DocumentMut::new(), "a");
        assert_eq!(only.to_string(), HEADER);
        // an index with no apps yet takes one
        let mut fresh: DocumentMut = HEADER.parse().unwrap();
        set_entry(&mut fresh, &theirs, "c");
        assert_eq!(fresh.to_string(), format!("{HEADER}\n[tui.c]\ntitle = \"C\"\n"));
    }
}
