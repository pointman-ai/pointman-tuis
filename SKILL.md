---
name: tuis
description: Show work that keeps happening (releases, deploys, builds, queues, test runs) in a small terminal app you build or reuse in this node's TUI folder, opened in a terminal panel beside the thread for your person to look around in. Use when someone asks when something ships, how a process is going or what state it's in, or when you'd otherwise explain the same moving parts again.
---

# TUIs for work that keeps happening

Your person can't see what you see: release branches, deploys, builds, queues. When a question like that comes up ("when does it ship?", "what's live?", "is the build done?"), show it in a small terminal app (a TUI) in a terminal panel beside the thread, rather than explaining it in a post. They can take the keyboard and look around themselves.

**This node's TUI folder** is `~/.local/share/message-board/tuis/`, kept by this extension. It's one Rust workspace (ratatui): the shared `kit/`, a `starter/` to copy, every TUI in `crates/<name>`, and the index `tuis.toml`. They share one build, so a new TUI builds in seconds. Its README has the rules and the keys every TUI shares.

**Reuse before you build.** Read `tuis.toml` first. If a TUI shows this already, build and open it. If it's close, extend it (another screen or column) rather than starting another. Fix a TUI that's wrong or stale instead of explaining around it.

**Building one:** for something that repeats or moves, never for a one-off answer.
1. `cp -R starter crates/<name>`, rename its package and binary to `tui-<name>`, and add `[tui.<name>]` to `tuis.toml`.
2. Implement the kit's `Tui` (its screens, a loader, and a draw). The kit does the terminal, the frame, the keys, help and refreshing.
3. It's read-only by default: anything that changes something asks first, on screen. It reads with the machine's own access (git, `gh`, `pointman status`), never anything that acts as your person or puts an approval card on their phone. Never put a secret on screen.
4. `cargo build --release -p tui-<name>`, then check `target/release/tui-<name> --snapshot` at 80×24 before anyone sees it.

`references/guide.md` has the layouts that work, the data sources, and testing.

**Showing it:**
1. Build first, in your own shell. Never build in theirs.
2. `open_pane(shell={"machine": <this machine>, "folder": "~/.local/share/message-board/tuis"})`, then `terminal_wait` for the prompt.
3. `terminal_send(text="target/release/tui-<name>", keys=["Enter"])`, then `terminal_read` to check it drew.
4. Answer in the thread in a line or two, pointing at what's on screen. Say they can take the keyboard: `?` for help, `q` to quit.

If this machine has no cargo (health says so), build on a node that has it and show it there.
