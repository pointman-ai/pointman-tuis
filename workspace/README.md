# This node's TUIs

Small terminal apps that agents built to show work that keeps happening, such as builds, releases, deploys, queues and test runs. Agents open them in a terminal panel beside the thread. Each is Go with Bubble Tea, on one shared kit. The pointman-tuis extension made this folder and keeps the kit, the starter and this README current. Agents add apps in `cmd/` and list them in `tuis.toml`.

| Path | What |
|---|---|
| `kit/` | What every app shares: the frame, the keys, help, refreshing, commands' output, and `--snapshot` |
| `cmd/_starter/` | The app to copy when you start a new one (`./...` skips it: its name starts with `_`) |
| `cmd/<name>/` | One app, `package main` |
| `bin/<name>` | Its build: `go build -o bin/<name> ./cmd/<name>` |
| `go.mod`, `go.sum` | The module, `tuis`. The extension raises the kit's requirements; add your own app's |
| `tuis.toml` | The index: one `[tui.<name>]` for each app here |

## Build and run

```sh
cd ~/.local/share/message-board/tuis
go build -o bin/<name> ./cmd/<name>     # about 0.2 s once the kit has been built once
bin/<name>                               # in a terminal
bin/<name> --snapshot                    # one frame as text, for checking without a terminal (--size 80x24)
```

No `go` on the PATH? The extension fetches Go into `~/.local/share/message-board/go`, so use `~/.local/share/message-board/go/bin/go`. Its health check says which Go this machine builds with. Go needs no C compiler here.

## Adding an app

1. Check `tuis.toml`. If an app already shows this, extend it instead.
2. `cp -R cmd/_starter cmd/<name>`, then make it yours: its `Title`, `Screens`, `Load`, `Rows` and `Draw` (the kit's `Tui`).
3. Add its `[tui.<name>]` to `tuis.toml`.
4. `go vet ./cmd/<name>`, `go test ./cmd/<name>`, build it, check `--snapshot` at 80×24, then run it in a terminal panel.

## Shared with the person's other machines

Every minute the extension syncs `cmd/<name>/` and its `[tui.<name>]` entry with the private repo pointman-ai/tuis, and each machine builds what comes in. An app goes out once it's built (`bin/<name>` newer than its files), as `<name>: from <machine>`. Taking an app's folder out here takes it out everywhere. An app changed here and on another machine since the last sync is a conflict: this machine's stays here, unshared, and the extension's health says where the shared one is.

## Rules for every app

- **Read-only by default.** Anything that changes something asks first, on screen, and says what it will do.
- **Never act as the person.** No commands that put an approval card on their phone. Use the machine's own reads.
- **No secrets on screen.** Never print a key, token or vault value.
- **The kit's keys everywhere:** `?` help, `q` quit, `r` refresh, `↑` `↓` `j` `k` to move, `Enter` to open, `Esc` to go back, `Tab` for the next screen.
- **Readable at 80×24.** It also has to look right in a phone-sized terminal panel.
