# Flow status: where a flow stands

`armadai flow status` shows, for a Claude Code session, where each flow of the project stands:
which steps are done, which one is running, which are still to come, and how many times a loop
went round. It reads what `armadai link` made Claude Code record while the flows ran. It runs
nothing: ArmadAI has no execution engine, it only observes.

MVP: Claude Code only (the report says `flow recording: not-compiled (Claude Code only for now)`
for OpenCode, Codex and Copilot).

## What is recorded

When the project has a flow, `armadai link` installs four **asynchronous** hooks in
`.claude/settings.json`, each running `armadai __record-flow-event`:

| Hook | When |
|---|---|
| `SubagentStart` | a sub-agent starts |
| `SubagentStop` | a sub-agent ends |
| `PreToolUse`, matcher `SubagentHandback` | a sub-agent hands back, with its reply (this tool exists in auto mode only) |
| `UserPromptExpansion` | a command typed in Claude Code (`/name`, skill or custom command): its name only |

Each event is one JSON line appended to `.armadai/state/claude/<session>.jsonl`: a timestamp,
start or stop, the sub-agent id, its agent name and the `STATE:` word it ended with; for a typed
command, a timestamp and its name (`{"ts":...,"event":"command","command":"bugfix"}`). Nothing
else: no prompt, no reply text, no command arguments. The hooks never fail and never print: bad
input, no project or an unwritable log record nothing.

`link` also adds `state/` to `.armadai/.gitignore` (and says `patched .../.gitignore` when it
changed an existing file): the logs describe this machine only. To purge them:

```bash
rm -r .armadai/state/
```

## The command you type

Without more, the flow of a run is inferred from the sub-agents seen. When you start a flow with
a command whose name is exactly the id of a flow (`/bugfix` for the flow `bugfix`, case
included), the run is attributed to that flow instead:

- the command opens a run of that flow alone (never `a | b`), attributed until that run is
  `finished` or the next flow command comes; after that the sub-agents are inferred again. It
  closes the runs still open: a run the next flow command cut short is shown `interrupted`
  instead of `in progress`;
- each sub-agent goes to the one step of the flow its agent holds, even before the step that
  leads to it has finished (the step is `running`, and the transition is noted when its source
  stops), so sub-agents running in the background that overlap lose no step;
- while the run is open, an agent that no step of the flow holds is ignored: it opens no run of
  its own (once the run has `finished`, it is inferred, and may open the run of another flow);
- a step that never started stays `○ pending`, and the run stays `in progress` while a link to
  that step is still to take: flow status cannot tell whether the step will come.

A command that does not name a flow (another name, a plugin command such as `/plugin:bugfix`:
not verified how Claude Code names it), a flow started in natural language, a Claude Code that
does not fire `UserPromptExpansion`, or the sub-agents after an attributed run has finished,
record no boundary: the inference above applies unchanged. A project linked before this hook
existed gets it by running `armadai link --target claude` again.

## The `STATE:` line

Where a step declares `states:` (for example `approved`, `rejected`), or has a failure path
(`on_failure`, or a link `when: failed`), the agent holding it gets this at the end of its
generated file, for Claude Code and for OpenCode alike:

```
End your reply with exactly one line `STATE: <state>`, where <state> is one of: approved, rejected.
```

A failure path adds `failed` to the list, and `done` too when the step declares no other state.

The recorded state is the last line of the reply that reads exactly `STATE: <word>`. The state
picks the link taken next (`when:`), so a review loop is followed: `build`, `guard` rejected,
`build` again. A link without `when:` is taken on any state but `failed`; `STATE: failed`
follows the failure path, even on a step without `states:`.

Under Claude Code the coordinator (the main thread, which lives in `CLAUDE.md`) does not get
this instruction and is not recorded: only sub-agents are. A step it holds is shown `~` and
never asserted done or pending. The run goes through it: a link into it leads on, in the same
run, to the steps after it (all of them, since it records no state), so `dev → lead → qa` or a
review loop `qa → lead → dev` is one run, with its loop counted. The transitions list each hop
(`qa → lead`, then `lead → dev`). A flow that starts with the coordinator opens on the steps
after it (also through several coordinator's steps in a row); a step after a coordinator's step
in the middle of a flow never opens a run by itself. A loop that comes back to the same step
through the coordinator does not hold the run open. A fan-out through the coordinator lists the
hop into it once (`f → hub`), then one hop per step reached (`hub → x`, `hub → y`).

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
  build → guard : 2
  09:15:20  design → build  (done)
  09:15:30  build → guard  (done)
  09:15:40  guard → build  (rejected)
  09:15:50  build → guard  (done)
```

| Mark | Meaning |
|---|---|
| `✓` | done, with its state |
| `●` | running |
| `○` | pending |
| `?` | unsure, with the reason in parentheses (for example no `STATE:` line). It leads nowhere: started again, the step is retried in place and its new state replaces the `?` |
| `✗` | a state the step does not declare. It leads nowhere: started again, the step is retried in place and its new state replaces the `✗`. A state the step declares but no link follows ends the flow normally: it is not a `✗` |
| `~` | held by the coordinator, which is not recorded (`coordinator, not recorded`) |
| `-` | not run in this session (`not run in this session`): a step before the one a run opened at, when the session started the flow mid-way (inferred runs only: in a run a command opened, a step that never started stays `○ pending`). Neither done nor pending, it never holds the run open |

A run heading reads `in progress`, `finished`, or `interrupted` (a later flow command closed the
run before it finished).

Other lines: `a → b : n` is a link taken `n` times (shown from twice on); the timed transitions
follow. All times
are UTC. A finished run stays listed, finished and interrupted runs first, each group in the order
they started.

- Without a recorded command, two flows whose entry step is shared are shown as `a | b` until
  their steps diverge. With one, the run is the flow the command names.
- A session can start a flow mid-way (for example at `frame`, its `refine` done in an earlier
  session): when no flow holding the agent has a run open (or its last one finished), a step
  of the flow opens a run where it starts, the steps before it shown `-`. Flows sharing that
  step are shown as `a | b` as above; an agent holding two steps of one flow opens one run with
  both steps `?` (ambiguous step) and only the steps before both shown `-`. Its entry step started later opens a new run. A step right after a
  coordinator's step in mid-flow is the exception (see the limits).
- An entry step that nothing leads back to, started again, closes the open run of its flow
  (kept, unfinished) and opens a new one. An entry step a loop leads back to does so only when
  the open run is stuck (nothing running, its last step `?` or `✗`).
- A `?` or `✗` step never ends a run.
- A step that links of several open runs lead to (an agent shared by two flows) goes to the run
  whose link is the most recent: the one whose source step stopped last. For example, `guide`
  left at `build` after `guard` rejected, then `implementer` (in `deliver` only) builds: the
  next `guardian` follows `deliver`. A link older than one another run already took to that
  step leads nowhere: a later `guardian` is not shown in `guide`. A link exactly as recent
  (same second) as the one already taken elsewhere is still taken. Flows still shown together
  (`deliver | guide`) are never split by this rule: their candidates keep moving together until
  a step only one of them holds.
- The project root is resolved as the hooks resolve it, so the command works from a
  subdirectory and from a git worktree. A worktree without its own `.armadai/` writes and reads
  the logs of the main checkout; a worktree with its own `.armadai/` writes and reads its own
  `state/`.

## In a status bar

`armadai flow status --line` prints one line for a status bar, and never fails: with no project, an invalid project, an unknown flow or session, or no run, it prints an empty line, with exit code 0 and nothing on stderr.

```text
deliver ● guard 2/4 (loops 1)
deliver 3/4
hotfix finished 2/2
bugfix interrupted 1/2
```

- The run shown is the last one still open, else the last one. `●` guard is the current step (the running one, else the first `?` or `✗`, else the first pending one) with its mark; `2/4` counts done steps over the steps recorded in this session (`~` and `-` steps are left out); `(loops 1)` counts the extra times links were taken, and shows only when there is one.
- The session is `--session`, else the `session_id` of a JSON read on stdin (the one Claude Code pipes to a status line), else the latest log of the project. The project is the `cwd` of that JSON, else the current directory. stdin is read only when it is piped, for 250 ms at most, so a terminal or a pipe that stays open never blocks the command.
- Marks are always printed; colour comes on top. `--color auto|always|never` (default `auto`) applies to every `flow status` output: `auto` follows the terminal and honours `NO_COLOR` and `CLICOLOR_FORCE`. A status bar that runs the command through a pipe needs `--color always`: `armadai flow status --line --color always`.
- The latest log is a guess when two sessions run at once in the same project: pass the session explicitly when you can.

## Machine-readable output

`armadai flow status --json` prints the session as one JSON document on stdout, and nothing else (it does not combine with `--watch` or `--list`). The `flow` argument and `--session` apply as for the text.

```json
{
  "schema": "armadai.flow-status/1",
  "session": { "id": "s1", "started_at": "2026-10-22T09:15:00Z", "last_event_at": "2026-10-22T09:15:50Z" },
  "runs": [
    {
      "flow": "deliver",
      "status": "in-progress",
      "steps": [ { "step": "guard", "mark": "running", "sign": "●", "state": null, "note": null } ],
      "loops": [ { "from": "build", "to": "guard", "times": 2 } ],
      "transitions": [ { "at": "2026-10-22T09:15:20Z", "from": "design", "to": "build", "state": "done" } ]
    }
  ]
}
```

- The schema only grows: a key is never renamed, retyped or removed, a new key is added. A change that breaks a reader gets a new `schema` value (`armadai.flow-status/2`). Read the keys you need and ignore the others.
- `runs` come in the order the runs started (the text shows finished and interrupted runs first).
- `started_at` and `last_event_at` are the first and last event of the log, UTC, to the second; `null` when the log is empty.
- No log at all: `"session": null, "runs": []`, exit code 0. `--session S` with no log for `S`: the session with its dates `null` and no run, exit code 0.

| `status` | Meaning |
|---|---|
| `in-progress` | The run has not finished and no later flow command closed it |
| `finished` | The run reached the end of its flow |
| `interrupted` | A later flow command closed it before it finished |

| `mark` | `sign` | Meaning |
|---|---|---|
| `done` | `✓` | The step finished, `state` says how |
| `running` | `●` | The step is running |
| `pending` | `○` | The step has not started |
| `unsure` | `?` | The log does not say; `note` gives the reason |
| `unexpected` | `✗` | The step ended in a state it does not declare |
| `coordinator` | `~` | The coordinator holds the step; it is not recorded |
| `not-run` | `-` | Before the step the run opened at in this session |

## Limits

- Claude Code only.
- When the log does not say, the answer is `?`, never a guess.
- Two links to a shared agent's step whose sources stopped in the same second cannot be told
  apart: the step is `?` (ambiguous step) in both runs, and neither moves. The most recent link
  is a choice, not a certainty: the user may have meant the older run.
- A sub-agent running in the background is not guaranteed to be seen: its stop may come after
  the reply that started it. When the command was recorded, overlapping sub-agents are placed
  in the attributed run and no step is lost; without it, a step started before its source has
  finished can still be left out.
- The version of Claude Code that introduced `UserPromptExpansion` is not verified. Where it
  does not fire, or for a command it does not cover, nothing says so: `flow status` falls back
  to the inference, with no message asking to link again.
- `/clear` is not a boundary. Whether it starts a new session, and so a new log, is not verified.
- A sub-agent called on its own that is also a step of the flow you launched (for example a
  manual `test` during `/bugfix`) is placed in that run: nothing tells it apart.
- A late transition is stamped when its source step stops. When a step ran and ended before its
  source did (overlapping sub-agents), the transition can carry a time after the end of the step
  it leads to: the list of transitions shows the order the log gave, not a strict timeline.
- A `start` or `stop` line with an empty `agent_id` loses the attribution, as two lines sharing
  the same id do. The hook never writes such a line in normal use; a log written by hand can.
- The coordinator is not recorded (see above) and `SubagentHandback` exists in auto mode only:
  outside it the stop event carries the `STATE:` word read from `SubagentStop` alone.
- A step that ends with `STATE: failed` and has no failure path (`on_failure`, `when: failed`)
  leads nowhere: the run shows `finished`, with the steps after it still pending.
- A coordinator's step records no state, so it is crossed to every step after it, whatever the
  `when:` of its links.
- A flow that is only a loop through the coordinator (`build → lead → build`) cannot tell a new
  run from one more round: both show in the same run.
- A step started while its run cannot be told is not shown: a step after a coordinator's step
  in mid-flow with no run open, or a step that no link allows while a run of its flow is open
  (stuck included: a `?`/`✗` step leads nowhere, only that step or the entry step starts
  again).
