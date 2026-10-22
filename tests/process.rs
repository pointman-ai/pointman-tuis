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
