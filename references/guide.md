# Building and showing TUIs: the details

SKILL.md has the short version. This is for when you build or change one.

## When a TUI is worth it

| Build or extend a TUI | Answer in a post |
|---|---|
| The question comes back: "when does it ship?", "what's live?" | A one-off question |
| Several moving parts: branches, nodes, CI, queues | One fact: a version or a link |
| State that changes while they watch | Something that's already finished |
| They want to look around themselves | They just want the answer |

## Layouts that work

- **A list and its detail.** Releases on the left, the selected one's commits on the right. `Enter` opens it, `Esc` goes back.
- **A board of states.** One row per thing (a node, a branch, a job), with columns that each hold one word or number: version, state, age.
- **A timeline.** What happened, newest first, with the time and who did it.
- **The top bar** (from the kit) always shows when it last refreshed and what failed to load. The kit's `List`, `ListAndDetail` and `Boxed` draw the first two layouts; its styles (`kit.Dim`, `kit.Warn` and the rest) keep every app alike.

Colour means state: green is done or live, yellow is waiting or running, red is failed. Always add a word too, because colour alone doesn't reach everyone.

## Data sources

Read with the machine's own access. Run commands with the kit's `RunOut`, `RunLines` and `RunJSON[T]`: each has a time limit, and a failure says the command's first line of stderr.

| What | Read it with |
|---|---|
| Branches, commits, what's merged | `git` in the repo's checkout (`git log --format`, `git branch -r`) |
| CI, PRs, releases | `gh … --json` |
| This node, its agents and runs | `pointman status`, `pointman list` |
| Other nodes | `pointman status --nodes all` |

**Never** call a command that acts as your person. Those put an approval card on their phone (for example `pointman health`, `pointman secrets`, `pointman machine list`). If you need something only those give, ask in the thread.

## Testing

- Keep data loading apart from drawing. Parse in plain functions, and test them with saved command output in `cmd/<name>/testdata/`.
- Test the frame at 80×24 with the kit's `Snapshot` (as the starter's `main_test.go` does), and check the key lines, so a change that breaks the layout fails a test.
- Before you show it, run `bin/<name> --snapshot` and read the frame (`--size WxH`, `--screen N`, `--row N`, `--open` for other views).

## Showing it, step by step

```text
go build -o bin/<name> ./cmd/<name>              # in your own shell, in ~/.local/share/message-board/tuis
open_pane(shell={"machine": "mac-m4", "folder": "~/.local/share/message-board/tuis"})
terminal_wait(<pane>)                            # for the prompt
terminal_send(<pane>, text="bin/<name>", keys=["Enter"])
terminal_read(<pane>)                            # check it drew
```

Then answer in the thread, pointing at what's on screen, and say they can take the keyboard. If they take the keys back, stop typing: the tools say when they have.
