# pointman-tuis

A Pointman extension: agents build small terminal apps (TUIs) that show work that keeps happening, such as releases, deploys, builds, queues and test runs. They open them in a terminal panel beside the thread, so you can see what's going on and take the keyboard to look around yourself.

| Part | What |
|---|---|
| [SKILL.md](SKILL.md) | What every agent on the node is told: when a TUI helps, reusing one, building one, and showing it |
| [references/guide.md](references/guide.md) | The details: layouts, data sources, testing |
| [workspace/](workspace) | The node's TUI folder as this release makes it: the Go module, the shared `kit/` (Bubble Tea) and the starter in `cmd/_starter` |
| `bin/tuis` | A small process that keeps `~/.local/share/message-board/tuis/` up to date from `workspace/`, and fetches Go from go.dev where a machine has none. Agents' apps in `cmd/` and their builds in `bin/` are never touched. Health shows the folder, its apps and which Go builds them |

## Add it

```sh
pointman add tuis
```

Every agent on a node with it gets the guidance in each run.

## Develop

`cargo test` runs the folder's install, the Go fetch (against an archive it makes) and the process under a stand-in node; `cargo test --lib -- --ignored from_go_dev` fetches the real Go. CI also runs the kit's tests and builds the starter from `workspace/`, as a node would.

MIT licensed.
