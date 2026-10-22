# Flow status: where a flow stands

`armadai flow status` shows, for a Claude Code session, where each flow of the project stands:
which steps are done, which one is running, which are still to come, and how many times a loop
went round. It reads what `armadai link` made Claude Code record while the flows ran. It runs
nothing: ArmadAI has no execution engine, it only observes.

MVP: Claude Code only (the report says `flow recording: not-compiled (Claude Code only for now)`
for OpenCode, Codex and Copilot).

## What is recorded

When the project has a flow, `armadai link` installs three **asynchronous** hooks in
`.claude/settings.json`, each running `armadai __record-flow-event`:

| Hook | When |
|---|---|
| `SubagentStart` | a sub-agent starts |
| `SubagentStop` | a sub-agent ends |
| `PreToolUse`, matcher `SubagentHandback` | a sub-agent hands back, with its reply (this tool exists in auto mode only) |

Each event is one JSON line appended to `.armadai/state/claude/<session>.jsonl`: a timestamp,
start or stop, the sub-agent id, its agent name and the `STATE:` word it ended with. Nothing
else: no prompt, no reply text. The hooks never fail and never print: bad input, no project or an
unwritable log record nothing.

`link` also adds `state/` to `.armadai/.gitignore` (and says `patched .../.gitignore` when it
changed an existing file): the logs describe this machine only. To purge them:

```bash
rm -r .armadai/state/
```

## The `STATE:` line

Where a step declares `states:` (for example `approved`, `rejected`), the agent holding it gets
this at the end of its generated file:

```
End your reply with exactly one line `STATE: <state>`, where <state> is one of: approved, rejected.
```

The recorded state is the last line of the reply that reads exactly `STATE: <word>`. The state
picks the link taken next (`when:`), so a review loop is followed: `build`, `guard` rejected,
`build` again.

Under Claude Code the coordinator (the main thread, which lives in `CLAUDE.md`) does not get
this instruction and is not recorded: only sub-agents are. A flow is therefore seen from its
first delegated step on.

## Reading the output

```bash
armadai flow status                    # the latest session, every flow
armadai flow status deliver            # only the runs of the flow `deliver`
armadai flow status --session 3f2a9c   # another session
armadai flow status --watch            # redraw on each new event (checked each second), Ctrl-C to stop
armadai flow status --list             # the recorded sessions (no flow or --session with it)
```

```
session s1 (UTC)

deliver  in progress
  ✓ design    done
  ✓ build     done
  ● guard     running
  ○ validate  pending
  build ⇄ guard : 2
  09:15:20  design → build  (done)
  09:15:30  build → guard  (done)
  09:15:40  guard → build  (rejected)
  09:15:50  build → guard
```

| Mark | Meaning |
|---|---|
| `✓` | done, with its state |
| `●` | running |
| `○` | pending |
| `?` | unsure, with the reason in parentheses (for example no `STATE:` line) |
| `✗` | a state the step does not declare. A state the step declares but no link follows ends the flow normally: it is not a `✗` |

Other lines: `a ⇄ b : n` is a link taken more than once; the timed transitions follow. All times
are UTC. A finished run stays listed, finished runs first.

- Two flows whose entry step is shared are shown as `a | b` until their steps diverge.
- A run left blocked (`?`) is closed when the entry step starts again.
- The project root is resolved as the hooks resolve it, so the command works from a
  subdirectory and from a git worktree (the logs of the main checkout are read).

## Limits

- Claude Code only.
- When the log does not say, the answer is `?`, never a guess.
- A sub-agent running in the background is not guaranteed to be seen: its stop may come after
  the reply that started it.
- The coordinator is not recorded (see above) and `SubagentHandback` exists in auto mode only:
  outside it the stop event carries the `STATE:` word read from `SubagentStop` alone.
