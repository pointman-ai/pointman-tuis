# pointman-tuis

A Pointman extension: agents build small terminal apps (TUIs) that show work that keeps happening, such as releases, deploys, builds, queues and test runs. They open them in a terminal panel beside the thread, so you can see what's going on and take the keyboard to look around yourself.

| Part | What |
|---|---|
| [SKILL.md](SKILL.md) | What every agent on the node is told: when a TUI helps, reusing one, building one, and showing it |
| [references/guide.md](references/guide.md) | The details: layouts, data sources, testing |
| [workspace/](workspace) | The node's TUI folder as this release makes it: the Rust workspace, the shared `kit/` (ratatui) and the starter in `crates/_starter` |
| `bin/tuis` | A small process that keeps `~/.local/share/message-board/tuis/` up to date from `workspace/`. Agents' TUIs in `crates/` are never touched. Health shows the folder, its TUIs and whether there's cargo |

## Add it

```sh
pointman add tuis
```

Every agent on a node with it gets the guidance in each run.

## Develop

`cargo test` runs the folder's install and the process under a stand-in node. CI also builds the starter from `workspace/`, as a node would.

MIT licensed.
