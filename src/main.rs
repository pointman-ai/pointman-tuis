//! The extension's process: the node starts it as extension.toml's `[run]` says (`bin/tuis`) and talks to it over
//! stdin and stdout (the SDK handles that). Log with `h.log(…)`: stdout is the node's.
//!
//! As it starts, and every hour after, it brings the node's TUI folder up to date from the release's `workspace/`
//! (tuis::install). Health says where the folder is, which TUIs it holds, and whether this machine has cargo to
//! build them.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pointman_extension::{Extension, Handle};
use serde_json::{json, Value};

/// How often the folder is checked again, in case something removed a file of the extension's.
const EVERY: Duration = Duration::from_secs(3600);

#[derive(Default)]
struct State {
    /// The last install's problem, if it had one.
    problem: Option<String>,
    installed: bool,
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
        .find(|s| s.join("Cargo.toml").is_file())
        .ok_or_else(|| anyhow::anyhow!("no workspace/ beside {}", exe.display()))
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

fn dest() -> PathBuf {
    tuis::folder(std::env::var("POINTMAN_TUIS_DIR").ok().as_deref(), &home())
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

fn health(state: &State) -> Value {
    let part = |id: &str, name: &str, state: &str, why: String| json!({"id": id, "kind": id, "name": name, "state": state, "needs": [], "why": why});
    let dest = dest();
    let folder = match (&state.problem, state.installed) {
        (Some(p), _) => part("folder", "The TUI folder", "failing", format!("{}: {p}", dest.display())),
        (None, false) => part("folder", "The TUI folder", "unknown", "not made yet".into()),
        (None, true) => {
            let names = tuis::tuis(&dest);
            let which = match names.len() {
                0 => "no TUIs yet".to_string(),
                n => format!("{n} TUI{}: {}", if n == 1 { "" } else { "s" }, names.join(", ")),
            };
            part("folder", "The TUI folder", "ok", format!("{}: {which}", dest.display()))
        }
    };
    let cargo = match tuis::cargo(std::env::var("PATH").ok().as_deref(), &home()) {
        Some(c) => part("cargo", "Rust (cargo)", "ok", format!("cargo at {}: TUIs build here", c.display())),
        None => part(
            "cargo",
            "Rust (cargo)",
            "failing",
            "no cargo on this machine, so TUIs can't be built here: build them on a node that has Rust, or install it \
             (https://rustup.rs)"
                .into(),
        ),
    };
    json!([folder, cargo])
}

fn main() -> anyhow::Result<()> {
    let mut ext = Extension::new()?;
    let state = Arc::new(Mutex::new(State::default()));

    let st = state.clone();
    ext.on_start(move |h: &Handle| install(h, &st));
    let st = state.clone();
    ext.every(EVERY, move |h: &Handle| install(h, &st), "folder");

    let st = state.clone();
    ext.method("health.check", move |_| Ok(health(&st.lock().unwrap_or_else(|e| e.into_inner()))));
    ext.method("health.fix", move |_| {
        Ok(json!({"message": "Nothing to fix from here: for cargo, install Rust on this machine (https://rustup.rs)."}))
    });
    ext.run()
}
