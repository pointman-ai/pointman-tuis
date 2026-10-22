---
name: tuis
description: Show work that keeps happening (releases, deploys, builds, queues, test runs) in a small terminal app you build or reuse in this node's TUI folder, opened in a terminal panel beside the thread for your person to look around in. Use when someone asks when something ships, how a process is going or what state it's in, or when you'd otherwise explain the same moving parts again.
---

# TUIs for work that keeps happening

Your person can't see what you see: release branches, deploys, builds, queues. When a question like that comes up ("when does it ship?", "what's live?", "is the build done?"), show it in a small terminal app (a TUI) in a terminal panel beside the thread, rather than explaining it in a post. They can take the keyboard and look around themselves.

**This node's TUI folder** is `~/.local/share/message-board/tuis/`, kept by this extension. It's one Go module (Bubble Tea): the shared `kit/`, a starter to copy in `cmd/_starter`, every app in `cmd/<name>` built to `bin/<name>`, and the index `tuis.toml`. A new app builds in about 0.2 s. Its README has the rules and the keys every app shares.

**Reuse before you build.** Read `tuis.toml` first. If a TUI shows this already, build and open it. If it's close, extend it (another screen or column) rather than starting another. Fix a TUI that's wrong or stale instead of explaining around it.

**Building one:** for something that repeats or moves, never for a one-off answer.
1. `cp -R cmd/_starter cmd/<name>`, and add `[tui.<name>]` to `tuis.toml`.
2. Make the kit's `Tui` yours: `Title`, `Screens`, `Load` (runs in the background), `Rows` and `Draw`. The kit does the terminal, the frame, the keys, help and refreshing. Keep parsing in plain functions and test them. Embed `kit.Kept` beside `kit.Base`, so it opens at once with the data it last loaded and refreshes behind it (its data needs exported fields).
3. It's read-only by default: anything that changes something asks first, on screen. It reads with the machine's own access (git, `gh`, `pointman status`), never anything that acts as your person or puts an approval card on their phone. Never put a secret on screen.
4. `go vet ./cmd/<name>` and `go test ./cmd/<name>`, then `go build -o bin/<name> ./cmd/<name>`, and check `bin/<name> --snapshot` at 80×24 before anyone sees it.

`references/guide.md` has the layouts that work, the data sources, and testing.

**Showing it** is one call, so they see it fast (James, thread 17421: every step you take is time they wait):
1. `open_pane(shell={"machine": <this machine>, "folder": "~/.local/share/message-board/tuis", "command": "bin/<name>"})`. The panel opens with the app already running.
2. Answer in the thread straight away, in a line or two, pointing at what's on screen. Say they can take the keyboard: `?` for help, `q` to quit (which closes the panel).

An app from before 22 Oct opens it at a prompt instead (the panel's command came in then): if they say so, or `terminal_read` shows one, `terminal_send(text="bin/<name>", keys=["Enter"])`.

Rebuild first, in your own shell, only when you changed the app (`go build -o bin/<name> ./cmd/<name>`), never in theirs.

No `go` on the PATH? The extension fetches Go into `~/.local/share/message-board/go`: use `~/.local/share/message-board/go/bin/go`. Its health check says which Go this machine builds with, or how far the fetch has got. Go needs no C compiler, on NixOS too.
