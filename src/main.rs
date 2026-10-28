//! The extension's process: the node starts it as extension.toml's `[run]` says (`bin/tuis`) and talks to it over
//! stdin and stdout (the SDK handles that). Log with `h.log(…)`: stdout is the node's.
//!
//! As it starts, and every hour after, it brings the node's TUI folder up to date from the release's `workspace/`
//! (tuis::install), and makes sure this machine has Go to build the apps there: one it already has, else the newest
//! stable from go.dev, fetched into `~/.local/share/message-board/go` (tuis::gofetch). Every minute, in a lane of its
//! own, it shares the apps with this person's other machines through the repo pointman-ai/tuis (tuis::share), with
//! the vault's GitHub account. Health says where the folder is, which apps it holds, which Go builds them, and how
//! the sharing went.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pointman_extension::protocol::{HealthCheckRequest, HealthCheckResponse, HealthFixRequest, HealthFixResponse};
use pointman_extension::v1::health::{part, Part};
use pointman_extension::{Extension, Handle};
use tuis::share::{self, Base, Sharing, Synced, REPO};

/// How often the folder, and Go, are checked again, in case something removed a file of the extension's.
const EVERY: Duration = Duration::from_secs(3600);
/// How often the apps are shared: `$POINTMAN_TUIS_SHARE_EVERY` seconds under tests.
const SHARE_EVERY: Duration = Duration::from_secs(60);
const NO_ACCOUNT: &str = "no GitHub account in the vault: add one with `/github add` in any thread (the github \
     extension), and this machine shares its apps within a minute. Until then its folder works on its own";

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

/// How sharing the apps is going.
#[derive(Default)]
struct Share {
    /// Turned off on this machine (`POINTMAN_TUIS_SHARE=off`).
    off: bool,
    /// The repo, as said in health.
    repo: String,
    /// The extension's clone of it.
    clone: PathBuf,
    /// When it last synced, and what that did.
    last: Option<(Instant, Synced)>,
    /// Apps that came in and didn't build here, with go's first words.
    failed: BTreeMap<String, String>,
    /// Why the last round didn't sync, and how to fix it.
    problem: Option<String>,
}

#[derive(Default)]
struct State {
    /// The last install's problem, if it had one.
    problem: Option<String>,
    installed: bool,
    go: Go,
    share: Share,
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

/// The vault's GitHub token for this machine, picked as the github extension picks it (`github.<label>`, the first
/// by label). Asked for each round and kept only in memory.
fn github_token(h: &Handle) -> Result<String, String> {
    let mut names = h.secrets("github.").map_err(|e| format!("couldn't list the vault's GitHub accounts ({e:#})"))?;
    names.sort_unstable();
    let name = names.first().ok_or(NO_ACCOUNT)?;
    h.secret(name).map_err(|e| format!("couldn't open the vault's {name} ({e:#})"))
}

/// One round of sharing the apps with the repo: what came in is built here, what changed here goes out.
fn share(h: &Handle, state: &Mutex<State>) {
    let lock = || state.lock().unwrap_or_else(|e| e.into_inner());
    let remote = match std::env::var("POINTMAN_TUIS_SHARE") {
        Ok(v) if v == "off" => {
            lock().share.off = true;
            return;
        }
        Ok(v) if !v.is_empty() => v,
        _ => REPO.to_string(),
    };
    let node = h.node();
    let state_dir = PathBuf::from(&node.state_dir);
    let clone = state_dir.join("shared");
    {
        let mut st = lock();
        st.share.repo = if remote == REPO { "pointman-ai/tuis".into() } else { remote.clone() };
        st.share.clone = clone.clone();
    }
    let problem = |p: String| {
        let mut st = lock();
        if st.share.problem.as_ref() != Some(&p) {
            h.log(format!("sharing the TUIs: {p}"));
        }
        st.share.problem = Some(p);
    };
    // the folder first: the start's install runs beside this lane's first round
    let until = Instant::now() + Duration::from_secs(30);
    while !lock().installed && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(100));
    }
    if !lock().installed {
        return problem("waiting for the TUI folder to be made".into());
    }
    if !state_dir.is_absolute() {
        return problem("the node gave no state folder for the shared repo's clone".into());
    }
    let Some(git) = share::find_git(std::env::var("PATH").ok().as_deref(), &home()) else {
        return problem("no git on this machine: install it, and the apps are shared within a minute".into());
    };
    let token = match remote == REPO {
        true => match github_token(h) {
            Ok(t) => Some(t),
            Err(e) => return problem(e),
        },
        false => None,
    };
    let s = Sharing {
        folder: dest(),
        clone,
        remote,
        machine: node
            .machine
            .as_option()
            .map(|m| m.name.as_str())
            .filter(|m| !m.is_empty())
            .unwrap_or("a machine")
            .to_string(),
        git,
        index_seed: seed().ok().and_then(|s| std::fs::read_to_string(s.join("tuis.toml")).ok()).unwrap_or_default(),
        token,
        git_config: vec![],
    };
    let base_file = state_dir.join("shared.json");
    let mut base = Base::load(&base_file);
    let before = base.failed.clone();
    let mut build = |name: &str| match find_go() {
        Some(go) => share::go_build(&go, &s.folder, name),
        None => Err("no Go on this machine yet: the extension is fetching it".into()),
    };
    let got = share::sync(&s, &mut base, &mut build);
    if let Err(e) = base.save(&base_file) {
        h.log(format!("sharing the TUIs: couldn't keep what was synced in {}: {e}", base_file.display()));
    }
    let synced = match got {
        Ok(synced) => synced,
        Err(e) => return problem(format!("{e:#}")),
    };
    let said = |what: &str, names: &[String]| (!names.is_empty()).then(|| format!("{what} {}", names.join(", ")));
    let done: Vec<String> = [
        said("came in and built:", &synced.came_in),
        said("went out:", &synced.went_out),
        said("taken out here, as the repo has:", &synced.removed_here),
        said("taken out of the repo, as here:", &synced.removed_there),
    ]
    .into_iter()
    .flatten()
    .collect();
    if !done.is_empty() {
        h.log(format!("sharing the TUIs with {}: {}", s.remote, done.join("; ")));
    }
    for (name, (hash, error)) in &base.failed {
        if before.get(name).map(|(h, _)| h) != Some(hash) {
            h.log(format!("sharing the TUIs: {name} came in but didn't build here: {error}"));
        }
    }
    let mut st = lock();
    let had = st.share.last.as_ref().map(|(_, l)| l.conflicts.clone()).unwrap_or_default();
    if synced.conflicts != had && !synced.conflicts.is_empty() {
        h.log(format!(
            "sharing the TUIs: {} changed here and in the repo since the last sync: kept this machine's, not pushed",
            synced.conflicts.join(", ")
        ));
    }
    st.share.failed = base.failed.into_iter().map(|(name, (_, error))| (name, error)).collect();
    st.share.last = Some((Instant::now(), synced));
    st.share.problem = None;
}

fn ago(d: Duration) -> String {
    match d.as_secs() {
        s if s < 90 => format!("{s} s"),
        s if s < 90 * 60 => format!("{} min", s / 60),
        s => format!("{} h", s / 3600),
    }
}

/// One of health's parts: `ok`, `failing`, or anything else for not known yet.
fn health_part(id: &str, name: &str, state: &str, why: String) -> Part {
    let state = match state {
        "ok" => part::State::STATE_OK,
        "failing" => part::State::STATE_FAILING,
        _ => part::State::STATE_UNSPECIFIED,
    };
    Part {
        id: id.into(),
        kind: id.into(),
        name: Some(name.into()),
        state: state.into(),
        why: Some(why),
        ..Default::default()
    }
}

fn sharing(share: &Share) -> Part {
    let part = |state: &str, why: String| health_part("sharing", "Sharing", state, why);
    if share.off {
        return part("ok", "off on this machine (POINTMAN_TUIS_SHARE=off): its apps stay here".into());
    }
    let last = share.last.as_ref().map(|(at, _)| format!("last synced {} ago", ago(at.elapsed())));
    let (at, s) = match (&share.problem, &share.last) {
        (Some(p), _) => return part("failing", last.map_or(p.clone(), |l| format!("{p} ({l})"))),
        (None, None) => return part("unknown", "not synced yet".into()),
        (None, Some(got)) => got,
    };
    let mut why = format!("with {}, {} ago: ", share.repo, ago(at.elapsed()));
    why += &match s.shared.len() {
        0 => "no apps shared yet".to_string(),
        n => format!("{n} app{} shared ({})", if n == 1 { "" } else { "s" }, s.shared.join(", ")),
    };
    if !share.failed.is_empty() {
        let each: Vec<String> = share.failed.iter().map(|(name, e)| format!("{name} ({e})")).collect();
        why += &format!(". Didn't build here: {}", each.join("; "));
    }
    if !s.conflicts.is_empty() {
        why += &format!(
            ". Changed here and on another machine since the last sync, so this machine's is kept and not shared: {}. \
             The shared one is in {}: copy it over this one to take it",
            s.conflicts.join(", "),
            share.clone.join("cmd").display()
        );
    }
    if !s.unbuilt.is_empty() {
        why += &format!(". Changed since it was built, so it's shared once it's built: {}", s.unbuilt.join(", "));
    }
    let state = if share.failed.is_empty() && s.conflicts.is_empty() { "ok" } else { "failing" };
    part(state, why)
}

fn health(state: &State) -> Vec<Part> {
    let part = health_part;
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
    vec![folder, go, sharing(&state.share)]
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
    let every = std::env::var("POINTMAN_TUIS_SHARE_EVERY").ok().and_then(|s| s.parse().ok()).map(Duration::from_secs);
    let st = state.clone();
    ext.every(every.unwrap_or(SHARE_EVERY), move |h: &Handle| share(h, &st), "sharing");

    let st = state.clone();
    ext.on(move |_: HealthCheckRequest| {
        Ok(HealthCheckResponse { parts: health(&st.lock().unwrap_or_else(|e| e.into_inner())), ..Default::default() })
    });
    ext.on(move |_: HealthFixRequest| {
        Ok(HealthFixResponse { message: "Nothing to fix from here: the extension fetches Go itself (hourly), or install it (https://go.dev/dl). Sharing the apps needs a GitHub account in the vault: `/github add` in any thread.".into(), ..Default::default() })
    });
    ext.run()
}
