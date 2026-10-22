//! The extension's process: the node starts it as extension.toml's `[run]` says (`bin/tuis`) and talks to it over
//! stdin and stdout (the SDK handles that). Log with `h.log(…)`: stdout is the node's.
//!
//! As it starts, and every hour after, it brings the node's TUI folder up to date from the release's `workspace/`
//! (tuis::install), and makes sure this machine has Go to build the apps there: one it already has, else the newest
//! stable from go.dev, fetched into `~/.local/share/message-board/go` (tuis::gofetch). Health says where the folder
//! is, which apps it holds, and which Go builds them.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pointman_extension::{Extension, Handle};
use serde_json::{json, Value};

/// How often the folder, and Go, are checked again, in case something removed a file of the extension's.
const EVERY: Duration = Duration::from_secs(3600);

/// Where fetching Go is at, for a machine that had none.
#[derive(Default, Clone)]
enum Go {
    #[default]
    Idle,
    Fetching {
        version: String,
        done: u64,
        of: u64,
    },
    Failed(String),
}

#[derive(Default)]
struct State {
    /// The last install's problem, if it had one.
    problem: Option<String>,
    installed: bool,
    go: Go,
}

/// The release's `workspace/`, in the extension's own folder: beside `bin/`, or in the folder it was started in (a
/// checkout, under tests).
fn seed() -> anyhow::Result<PathBuf> {
    let exe = std::env::current_exe()?;
    let beside_bin = exe.parent().and_then(|bin| bin.parent()).map(|root| root.join("workspace"));
    let here = std::env::current_dir().ok().map(|d| d.join("workspace"));
    [beside_bin, here]
        .into_iter()
        .flatten()
        .find(|s| s.join("go.mod").is_file())
        .ok_or_else(|| anyhow::anyhow!("no workspace/ beside {}", exe.display()))
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

fn dest() -> PathBuf {
    tuis::folder(std::env::var("POINTMAN_TUIS_DIR").ok().as_deref(), &home())
}

fn go_dir() -> PathBuf {
    tuis::go_dir(std::env::var("POINTMAN_TUIS_GO_DIR").ok().as_deref(), &home())
}

fn find_go() -> Option<PathBuf> {
    tuis::go(std::env::var("PATH").ok().as_deref(), &home(), &go_dir())
}

fn install(h: &Handle, state: &Mutex<State>) {
    // held through the install: the start and the hourly lane both run one as it starts, and two at once would
    // trip over each other's files
    let mut st = state.lock().unwrap_or_else(|e| e.into_inner());
    let got = seed().and_then(|s| Ok(tuis::install(&s, &dest())?));
    match got {
        Ok(r) => {
            if !r.written.is_empty() || !r.removed.is_empty() {
                h.log(format!(
                    "the TUI folder {}: wrote {}{}",
                    dest().display(),
                    r.written.join(", "),
                    if r.removed.is_empty() { String::new() } else { format!("; took out {}", r.removed.join(", ")) }
                ));
            }
            if !r.problems.is_empty() {
                h.log(format!("the TUI folder {}: couldn't write {}", dest().display(), r.problems.join("; ")));
            }
            st.problem =
                (!r.problems.is_empty()).then(|| format!("parts couldn't be written: {}", r.problems.join("; ")));
            st.installed = true;
        }
        Err(e) => {
            h.log(format!("couldn't make the TUI folder {}: {e:#}", dest().display()));
            st.problem = Some(format!("{e:#}"));
        }
    }
}

/// Go for this machine: nothing to do when it has one; else the newest stable, fetched (in its own lane, so the
/// folder never waits on a download).
fn ensure_go(h: &Handle, state: &Mutex<State>) {
    let set = |go: Go| state.lock().unwrap_or_else(|e| e.into_inner()).go = go;
    if find_go().is_some() {
        set(Go::Idle);
        return;
    }
    if std::env::var("POINTMAN_TUIS_FETCH_GO").is_ok_and(|v| v == "off") {
        set(Go::Failed("fetching it is off here (POINTMAN_TUIS_FETCH_GO=off)".into()));
        return;
    }
    let got = tuis::gofetch::newest().and_then(|a| {
        h.log(format!("no Go on this machine: fetching {} into {}", a.version, go_dir().display()));
        set(Go::Fetching { version: a.version.clone(), done: 0, of: a.size });
        tuis::gofetch::fetch(&a, &go_dir(), |done| set(Go::Fetching { version: a.version.clone(), done, of: a.size }))
            .map(|()| a)
    });
    match got {
        Ok(a) => {
            h.log(format!("{} is in {}", a.version, go_dir().display()));
            set(Go::Idle);
        }
        Err(e) => {
            h.log(format!("couldn't fetch Go: {e:#}"));
            set(Go::Failed(format!("{e:#}")));
        }
    }
}

fn health(state: &State) -> Value {
    let part = |id: &str, name: &str, state: &str, why: String| json!({"id": id, "kind": id, "name": name, "state": state, "needs": [], "why": why});
    let dest = dest();
    let folder = match (&state.problem, state.installed) {
        (Some(p), _) => part("folder", "The TUI folder", "failing", format!("{}: {p}", dest.display())),
        (None, false) => part("folder", "The TUI folder", "unknown", "not made yet".into()),
        (None, true) => {
            let names = tuis::tuis(&dest);
            let which = match names.len() {
                0 => "no apps yet".to_string(),
                n => format!("{n} app{}: {}", if n == 1 { "" } else { "s" }, names.join(", ")),
            };
            part("folder", "The TUI folder", "ok", format!("{}: {which}", dest.display()))
        }
    };
    let go = match (find_go(), &state.go) {
        (Some(go), _) => part(
            "go",
            "Go",
            "ok",
            format!("go at {}: apps build with `go build -o bin/<name> ./cmd/<name>`", go.display()),
        ),
        (None, Go::Fetching { version, done, of }) => part(
            "go",
            "Go",
            "unknown",
            format!("fetching {version} for this machine: {}% of {} MB", done * 100 / (*of).max(1), of / 1_000_000),
        ),
        (None, Go::Failed(e)) => part(
            "go",
            "Go",
            "failing",
            format!("no Go on this machine, and fetching it failed ({e}): install it (https://go.dev/dl)"),
        ),
        (None, Go::Idle) => part("go", "Go", "unknown", "no Go found yet: the extension fetches it".into()),
    };
    json!([folder, go])
}

fn main() -> anyhow::Result<()> {
    let mut ext = Extension::new()?;
    let state = Arc::new(Mutex::new(State::default()));

    let st = state.clone();
    ext.on_start(move |h: &Handle| install(h, &st));
    let st = state.clone();
    ext.every(EVERY, move |h: &Handle| install(h, &st), "folder");
    let st = state.clone();
    ext.every(EVERY, move |h: &Handle| ensure_go(h, &st), "go");

    let st = state.clone();
    ext.method("health.check", move |_| Ok(health(&st.lock().unwrap_or_else(|e| e.into_inner()))));
    ext.method("health.fix", move |_| {
        Ok(json!({"message": "Nothing to fix from here: the extension fetches Go itself (hourly), or install it (https://go.dev/dl)."}))
    });
    ext.run()
}
