# pointman-tuis

A Pointman extension: agents build small terminal apps (TUIs) that show work that keeps happening, such as releases, deploys, builds, queues and test runs. They open them in a terminal panel beside the thread, so you can see what's going on and take the keyboard to look around yourself.

| Part | What |
|---|---|
| [SKILL.md](SKILL.md) | What every agent on the node is told: when a TUI helps, reusing one, building one, and showing it |
| [references/guide.md](references/guide.md) | The details: layouts, data sources, testing |
| [workspace/](workspace) | The node's TUI folder as this release makes it: the Go module, the shared `kit/` (Bubble Tea) and the starter in `cmd/_starter` |
| `bin/tuis` | A small process that keeps `~/.local/share/message-board/tuis/` up to date from `workspace/`, and fetches Go from go.dev where a machine has none. Agents' apps in `cmd/` and their builds in `bin/` are never touched. Health shows the folder, its apps and which Go builds them |
| Sharing | Every minute the process syncs the apps with the private repo [pointman-ai/tuis](https://github.com/pointman-ai/tuis) (`cmd/<name>/` and `tuis.toml` only), so an app made on one machine shows up on the others, built there. Details below |

## Add it

```sh
pointman add tuis
```

Every agent on a node with it gets the guidance in each run.

## Sharing apps between machines

Each node keeps a clone of pointman-ai/tuis in the extension's state folder and, every 60 s, compares each app (its files and its `[tui.<name>]` entry) here, in the repo, and as both were at its last sync:

| Changed | What happens |
|---|---|
| Only in the repo | Copied into the folder, its entry merged into `tuis.toml`, built with `go build -o bin/<name> ./cmd/<name>` |
| Only here, and built since | Committed and pushed as the machine: `<name>: from <machine>` |
| Here and in the repo | A conflict: this machine's is kept and not pushed; health names it |
| Taken out on one side | Taken out on the other, unless it changed there (then the change wins) |

It signs in with the vault's GitHub account (`/github add`, the github extension's; the first by label), asked for at each sync and handed to git only in its environment, for this repo only. With none, health says so and the folder works on its own. `POINTMAN_TUIS_SHARE=off` turns sharing off on a machine.

## Develop

`cargo test` runs the folder's install, the Go fetch (against an archive it makes), the sharing (between machines that share a local bare repo) and the process under a stand-in node; `cargo test --lib -- --ignored from_go_dev` fetches the real Go. CI also runs the kit's tests and builds the starter from `workspace/`, as a node would.

MIT licensed.
