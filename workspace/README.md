# This node's TUIs

Small terminal apps that agents built to show work that keeps happening, such as builds, releases, deploys, queues and test runs. Agents open them in a terminal panel beside the thread. The pointman-tuis extension made this folder and keeps the kit, the starter and this README current. Agents add TUIs in `crates/` and list them in `tuis.toml`.

| Path | What |
|---|---|
| `kit/` | What every TUI shares: the app loop, the frame, the keys, help, commands' output, and `--snapshot` |
| `crates/_starter/` | The TUI to copy when you start a new one. It builds with the rest, which keeps the shared build warm |
| `crates/<name>/` | One TUI. Its package and binary are `tui-<name>` |
| `tuis.toml` | The index: one `[tui.<name>]` for each TUI here |
| `target/` | One build folder for all of them, so a new TUI builds in seconds |

## Build and run

```sh
cd ~/.local/share/message-board/tuis
cargo build --release -p tui-<name>
target/release/tui-<name>                 # in a terminal
target/release/tui-<name> --snapshot      # one frame as text, for checking without a terminal
```

## Adding a TUI

1. Check `tuis.toml`. If a TUI already shows this, extend it instead.
2. `cp -R crates/_starter crates/<name>`, then rename its package and binary to `tui-<name>`.
3. Add its `[tui.<name>]` to `tuis.toml`.
4. Build it, check `--snapshot` at 80×24, then run it in a terminal panel.

## Rules for every TUI

- **Read-only by default.** Anything that changes something asks first, on screen, and says what it will do.
- **Never act as the person.** No commands that put an approval card on their phone. Use the machine's own reads.
- **No secrets on screen.** Never print a key, token or vault value.
- **The kit's keys everywhere:** `?` help, `q` quit, `r` refresh, `↑` `↓` `j` `k` to move, `Enter` to open, `Esc` to go back, `Tab` for the next screen.
- **Readable at 80×24.** It also has to look right in a phone-sized terminal panel.
