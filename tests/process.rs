//! The process under a stand-in node: it makes the TUI folder as it starts, and health says so.

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
        .start()
        .expect("started");

    // done once health has a verdict on the folder (it says "unknown" until the first install has finished)
    let until = Instant::now() + Duration::from_secs(10);
    while health(&node)[0]["state"] == "unknown" && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(25));
    }
    for made in ["Cargo.toml", ".cargo/config.toml", "README.md", "tuis.toml", "crates"] {
        assert!(dest.join(made).exists(), "{made} in {}", dest.display());
    }
    // the workspace this release ships: the kit and every TUI in crates/
    let cargo = std::fs::read_to_string(dest.join("Cargo.toml")).unwrap();
    assert!(cargo.contains(r#"members = ["kit", "crates/*"]"#), "{cargo}");

    let parts = health(&node);
    let folder = &parts[0];
    assert_eq!((folder["id"].as_str(), folder["state"].as_str()), (Some("folder"), Some("ok")), "{parts}");
    assert!(folder["why"].as_str().unwrap().ends_with("no TUIs yet"), "{parts}");

    // an agent adds one: health counts it, and a restart leaves it alone
    std::fs::create_dir_all(dest.join("crates/releases")).unwrap();
    std::fs::write(dest.join("crates/releases/Cargo.toml"), "[package]\nname = \"tui-releases\"\n").unwrap();
    assert!(health(&node)[0]["why"].as_str().unwrap().ends_with("1 TUI: releases"));
    node.stop();
    assert!(dest.join("crates/releases/Cargo.toml").is_file());
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
