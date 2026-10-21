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

    let until = Instant::now() + Duration::from_secs(10);
    while !dest.join("Cargo.toml").is_file() && Instant::now() < until {
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
