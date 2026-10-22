//! The process under a stand-in node: it makes the TUI folder as it starts, and health says so (and which Go).

use std::time::{Duration, Instant};

use pointman_extension::testing::StandIn;
use serde_json::{json, Value};

const ROOT: &str = env!("CARGO_MANIFEST_DIR");

fn health(node: &StandIn) -> Value {
    node.call("health.check", json!({})).expect("health.check")
}

#[test]
fn it_makes_the_tui_folder_as_it_starts_and_health_says_where() {
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("tuis");
    let node = StandIn::new(ROOT)
        .binary(env!("CARGO_BIN_EXE_tuis"))
        .settings(json!({}))
        .env("POINTMAN_TUIS_DIR", &dest)
        // no 70 MB of Go from go.dev in a test, and no Go of this machine's either: only the one it would fetch
        .env("POINTMAN_TUIS_GO_DIR", dir.path().join("go"))
        .env("POINTMAN_TUIS_FETCH_GO", "off")
        .start()
        .expect("started");

    // done once health has a verdict on the folder (it says "unknown" until the first install has finished)
    let until = Instant::now() + Duration::from_secs(10);
    while health(&node)[0]["state"] == "unknown" && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(25));
    }
    for made in ["go.mod", "go.sum", "README.md", "tuis.toml", "kit", "cmd/_starter", "bin"] {
        assert!(dest.join(made).exists(), "{made} in {}", dest.display());
    }
    // the module this release ships, which the kit is imported from
    let go_mod = std::fs::read_to_string(dest.join("go.mod")).unwrap();
    assert!(go_mod.contains("\nmodule tuis\n") && go_mod.contains("charm.land/bubbletea/v2"), "{go_mod}");

    let parts = health(&node);
    let folder = &parts[0];
    assert_eq!((folder["id"].as_str(), folder["state"].as_str()), (Some("folder"), Some("ok")), "{parts}");
    assert!(folder["why"].as_str().unwrap().ends_with("no apps yet"), "{parts}");
    assert_eq!(parts[1]["id"], "go", "{parts}");
    // with no GitHub account in the vault it doesn't share, says how to start, and the folder works as before
    let until = Instant::now() + Duration::from_secs(10);
    while health(&node)[2]["state"] == "unknown" && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(25));
    }
    let sharing = &health(&node)[2];
    assert_eq!((sharing["id"].as_str(), sharing["state"].as_str()), (Some("sharing"), Some("failing")), "{sharing}");
    assert!(sharing["why"].as_str().unwrap().contains("`/github add`"), "{sharing}");

    // an agent adds one: health counts it, and a restart leaves it alone
    std::fs::create_dir_all(dest.join("cmd/releases")).unwrap();
    std::fs::write(dest.join("cmd/releases/main.go"), "package main\n").unwrap();
    assert!(health(&node)[0]["why"].as_str().unwrap().ends_with("1 app: releases"));
    node.stop();
    assert!(dest.join("cmd/releases/main.go").is_file());
}

/// The node leaves hidden files out of an extension it unpacks, so nothing in workspace/ may be one (v0.1.0 shipped
/// `.cargo/config.toml` there, and nixos's folder stopped at it: Pointman thread 17421).
#[test]
fn nothing_in_workspace_is_hidden() {
    fn walk(dir: &std::path::Path, hidden: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                hidden.push(entry.path().display().to_string());
            } else if entry.path().is_dir() {
                walk(&entry.path(), hidden);
            }
        }
    }
    let mut hidden = vec![];
    walk(&std::path::Path::new(ROOT).join("workspace"), &mut hidden);
    assert!(hidden.is_empty(), "hidden in workspace/: {hidden:?}");
}

fn git(dir: &std::path::Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "nixos")
        .env("GIT_AUTHOR_EMAIL", "tuis@nixos")
        .env("GIT_COMMITTER_NAME", "nixos")
        .env("GIT_COMMITTER_EMAIL", "tuis@nixos")
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// EXT-36: an app another machine shared comes into the folder, with its entry, and one made here goes out, as this
/// machine. A local repo stands in for pointman-ai/tuis.
#[test]
fn it_shares_the_apps_through_the_repo() {
    let dir = tempfile::tempdir().unwrap();
    let (dest, remote, other) = (dir.path().join("tuis"), dir.path().join("remote.git"), dir.path().join("other"));
    std::fs::create_dir_all(&remote).unwrap();
    git(&remote, &["init", "-q", "--bare", "-b", "main"]);
    git(dir.path(), &["clone", "-q", remote.to_str().unwrap(), other.to_str().unwrap()]);
    std::fs::create_dir_all(other.join("cmd/disks")).unwrap();
    std::fs::write(other.join("cmd/disks/main.go"), "package main\n\nfunc main() {}\n").unwrap();
    std::fs::write(other.join("tuis.toml"), "[tui.disks]\ntitle = \"Disks\"\ncommand = \"bin/disks\"\n").unwrap();
    git(&other, &["add", "-A"]);
    git(&other, &["commit", "-q", "-m", "disks: from nixos"]);
    git(&other, &["push", "-q", "origin", "HEAD:main"]);

    let node = StandIn::new(ROOT)
        .binary(env!("CARGO_BIN_EXE_tuis"))
        .settings(json!({}))
        .machine("mac-m4")
        .env("POINTMAN_TUIS_DIR", &dest)
        .env("POINTMAN_TUIS_GO_DIR", dir.path().join("go"))
        .env("POINTMAN_TUIS_FETCH_GO", "off")
        .env("POINTMAN_TUIS_SHARE", &remote)
        .env("POINTMAN_TUIS_SHARE_EVERY", "1")
        // what comes in is built with this machine's Go, if it has one: never a toolchain or module fetched in a test
        .env("GOTOOLCHAIN", "local")
        .env("GOPROXY", "off")
        .start()
        .expect("started");
    let wait = |what: &str, done: &dyn Fn() -> bool| {
        let until = Instant::now() + Duration::from_secs(20);
        while !done() {
            assert!(Instant::now() < until, "{what}: {}", health(&node));
            std::thread::sleep(Duration::from_millis(50));
        }
    };
    wait("the shared app comes in", &|| dest.join("cmd/disks/main.go").is_file());
    let index = std::fs::read_to_string(dest.join("tuis.toml")).unwrap();
    assert!(index.starts_with("# This node's TUIs") && index.contains("[tui.disks]\ntitle = \"Disks\""), "{index}");
    wait("health says so", &|| health(&node)[2]["why"].as_str().unwrap_or_default().contains("1 app shared (disks)"));

    // an app made here, and built: it goes out as this machine
    std::fs::create_dir_all(dest.join("cmd/hello")).unwrap();
    std::fs::write(dest.join("cmd/hello/main.go"), "package main\n\nfunc main() {}\n").unwrap();
    std::thread::sleep(Duration::from_millis(10));
    std::fs::write(dest.join("bin/hello"), "built").unwrap();
    wait("it goes out", &|| git(&remote, &["ls-tree", "-r", "--name-only", "main"]).contains("cmd/hello/main.go"));
    assert_eq!(git(&remote, &["log", "-1", "--format=%s by %an", "main"]).trim(), "hello: from mac-m4 by mac-m4");
    wait("health counts it", &|| health(&node)[2]["why"].as_str().unwrap_or_default().contains("2 apps shared"));
    node.stop();
}
