# Flow status: where a flow stands

`armadai flow status` shows, for a Claude Code session, where each flow of the project stands:
which steps are done, which one is running, which are still to come, and how many times a loop
went round. It reads what `armadai link` made Claude Code record while the flows ran. It runs
nothing: ArmadAI has no execution engine, it only observes.

MVP: Claude Code only (the report says `flow recording: not-compiled (Claude Code only for now)`
for OpenCode, Codex and Copilot).

## What is recorded

When the project has a flow, `armadai link` installs **asynchronous** hooks on four events in
`.claude/settings.json`, each running `armadai __record-flow-event`:

| Hook | When |
|---|---|
| `SubagentStart` | a sub-agent starts |
| `SubagentStop` | a sub-agent ends |
| `PreToolUse`, matcher `SubagentHandback` | a sub-agent hands back, with its reply (this tool exists in auto mode only) |
| `PreToolUse`, matcher `Agent\|Task` | a delegation whose description starts with a [marker](#the-delegation-marker): the marker alone |
| `UserPromptExpansion` | a command typed in Claude Code (`/name`, skill or custom command): its name only |

Each event is one JSON line appended to `.armadai/state/claude/<session>.jsonl`: a timestamp,
start or stop, the sub-agent id, its agent name, the `STATE:` word it ended with and its `REF:`
word (see below); for a typed command, a timestamp and its name
(`{"ts":...,"event":"command","command":"bugfix"}`); for a marked delegation, a `mark` line
(`{"ts":...,"event":"mark","agent_type":"reviewer","flow":"evolution","step":"review","lot":"menu"}`,
without `lot` when the marker has none). Nothing else: no prompt, no reply text, no description, no
command arguments. The `mark` and the `REF:` word are the only things recorded beyond what the
first releases wrote. The hooks never fail and never print: bad
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

## The delegation marker

On Claude Code, the coordinator starts the `description` of an `Agent` call to an agent that holds a
step with `[<flow>/<step>]` or `[<flow>/<step>#<lot>]` (for example `[lot/plan#12] write the plan`).
`#<lot>` is optional: it names one batch of work, so two batches of one flow stay apart. Under
`policy.delegation: strict` the [gate](policy-gate.md#the-delegation-marker) refuses a delegation
without it; without `strict` it is optional, and recorded when present. When the project has a flow,
`link` writes the form into the `### Marking delegations` section of the root `CLAUDE.md` block.

The hook writes a `mark` line before the sub-agent starts. `SubagentStart` carries no call id, so a
`mark` and the `start` it belongs to are matched by the order of the log, with these rules:

- a `start` of a new sub-agent takes the oldest unused `mark` for its agent name and goes to the run
  of that `(flow, lot)` pair, opened if there is none. The run is attributed as by a typed command:
  no inference moves it, and only its natural end, not another lot or a restart of its entry step,
  closes it. Two runs of one flow with different lots stay apart;
- when several unused marks for one agent name differ, the match is **ambiguous**: each such `start`
  is `?` (`ambiguous marker`) on the step of each of them, in a run of each `(flow, lot)`, and no
  link is taken. Identical marks are not ambiguous: each `start` takes one;
- a `start` of a sub-agent already seen (a resumption by `SendMessage`) takes no `mark` and
  **inherits** the marker of its first start; when that marker was ambiguous it stays unplaced, and
  when there was none it is inferred as before;
- an **orphan** mark (its call never started: refused, failed) is dropped at the next `stop` of a
  sub-agent already seen, so it cannot stick to a later start. A `stop` of the main thread does not
  drop it;
- a mark that names a flow or step the project lacks, or a step the agent does not hold, is
  **ignored**: the `start` is inferred as if it had none;
- an unmarked `start` follows the rules above and never extends a run a marker opened. Without any
  `mark` line in a session, nothing in this section applies and the output is what it was before.

A sub-agent may end its reply, after its `STATE:` line, with one line reading exactly
`REF: <ref>`: up to 64 letters, digits or `._/#-` (a branch, a merge request). The hook records
the last valid such line of the reply; the first one recorded for a sub-agent shows on its step. Any
other line (`REF: two words`, `REF:x`, `see REF: x`) records nothing.

## Reading the output

```bash
armadai flow status                    # the latest session, every flow
armadai flow status deliver            # only the runs of the flow `deliver`
armadai flow status --session 3f2a9c   # another session
armadai flow status --watch            # follow the session until Ctrl-C (in a terminal: the running flow drawn in boxes)
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
| `⤼` | an optional step that never started and is skipped (`skipped`): see [Optional steps](#optional-steps) |
| `-` | not run in this session (`not run in this session`): a step before the one a run opened at, when the session started the flow mid-way (inferred runs only: in a run a command opened, a step that never started stays `○ pending`). Neither done nor pending, it never holds the run open |

A run whose steps were placed by a marker with a lot heads `deliver #menu  finished` (Markdown: `## deliver #menu: finished`); a run without a lot is headed as before. A step with a `REF:` shows it after its state, in square brackets: `  ✓ build     done [feature/menu]` (Markdown: the Status cell reads `done [feature/menu]`).

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

In a terminal, `--watch` draws the run in progress in boxes (the drawing of `armadai graph --format ascii`), each step with its mark, and a spinner (`| / - \`) after the running step. The last run still open is drawn (the one `--line` names), and the other runs are counted on one line under it (`+ 2 other runs: armadai flow status`). The view is redrawn within 250 ms of a new event, without clearing the screen, and stays still while no step runs and the terminal keeps its size. When the drawing does not fit the terminal (`COLUMNS` and `LINES`, else the terminal's size, else 80×24), that run is listed instead of drawn, with the same count line. A flow shown as `a | b` is listed too. The cursor is hidden while watching and shown again on exit, Ctrl-C included. Outside a terminal (a pipe, a file), `--watch` prints the list again on each new event, with no cursor or erase sequence, even with `--color always`.

Under the drawing, after a blank line, comes the block of the run as listed above (its steps, the links taken more than once, its transitions):

```
session s1 (UTC)

deliver  in progress
      [✓ ▶ design (architect)  ]
                  ┌┘
                  ↓
          [✓ build (dev)  ]
                  └┐
                   ↓
           [● guard (qa) |]
                   │
              "approved"
                   ↓
        [○ validate (ops) ■  ]

links not drawn:
  guard → build (rejected)

  ✓ design
  ✓ build
  ● guard     running
  ○ validate  pending
```

## In a status bar

`armadai flow status --line` prints one line for a status bar, and never fails: with no project, an invalid project, an unknown flow or session, or no run, it prints an empty line, with exit code 0 and nothing on stderr.

```text
deliver ● guard 2/4 (loops 1)
deliver 3/4
hotfix finished 2/2
bugfix interrupted 1/2
```

- The run shown is the last one still open, else the last one. `●` guard is the current step (the running one, else the first `?` or `✗`, else the first pending one) with its mark, then the agent that holds it in parentheses, and after a colon the reason of a `?` or `✗` (`deliver ? guard (qa: no STATE line) 2/4`, `deliver ○ validate (ops) 3/4`); a finished or interrupted run names no step and no agent, and the row of a sub-agent names its step but not the agent. `2/4` counts done steps over the steps recorded in this session (`~`, `-` and `⤼` steps are left out); `(loops 1)` counts the extra times links were taken, and shows only when there is one.
- The session is `--session`, else the `session_id` of a JSON read on stdin (the one Claude Code pipes to a status line), else the latest log of the project. The project is the `cwd` of that JSON, else the current directory. stdin is read only when it is piped, for 250 ms at most, so a terminal or a pipe that stays open never blocks the command.
- Marks are always printed; colour comes on top. `--color auto|always|never` (default `auto`) applies to every `flow status` output: `auto` follows the terminal and honours `NO_COLOR` and `CLICOLOR_FORCE`. A status bar that runs the command through a pipe needs `--color always`: `armadai flow status --line --color always`.
- In [ccstatusline](https://github.com/sirmalloc/ccstatusline), add a **Custom Command** widget, give it `armadai flow status --line --color always` and turn on its **preserve colors** option (without it the widget drops the colour). Read in the code of ccstatusline (the `render` method of the Custom Command widget; its page does not say so): the widget passes the whole JSON of Claude Code on stdin (`session_id`, `cwd`, ...) plus `terminal_width`, so the line follows the session and the project of Claude Code. Measured with ccstatusline run in an isolated `HOME`: the bar shows the same line as `flow status --line --session <id>`, an unknown session leaves the widget empty, and a JSON without `session_id` shows the latest log. The widget waits 1000 ms by default (`timeoutMs`, set per widget); its page also announces an optional cache of up to 60 s, so the line can lag behind a flow by that much.
- The status line that `armadai link` writes for Claude Code calls the hidden command `armadai __claude-statusline`: it reads `session_id` and `cwd` from the JSON on stdin, always colours, takes no argument, and prints nothing (0 bytes, exit code 0) when the JSON has no `session_id`, the session is unknown, the JSON is invalid or the log holds no run. It never falls back to the latest log.
- To get that bar without a script, set `ui: { status_line: true }` in `project.yaml` and run `armadai link --target claude`: it writes `statusLine` (`armadai __claude-statusline`, refreshed every 5 s) in `.claude/settings.json`, which runs the `statusLine` of your user settings first (same stdin, at most 1 s, `sh -c`; Unix only) and prints its output, then the flow line on the next row (details in [Link](link.md#the-chained-status-line)). A failing (non-zero), slow or absent user bar leaves the flow line alone, and the exit code is always 0. If `.claude/settings.json` already has a `statusLine`, `link` leaves it, writes the rest and reports `ui.status_line` as `not-compiled`: add `armadai flow status --line` to your own bar instead. If your own `statusLine` is identical to the one `link` would write, `link` reports `compiled` but does not own it: `unlink` leaves it in place. Other targets report `not-compiled` (Claude Code only). See [Project format](project-format.md).
- To get a row per subagent, set `ui: { subagent_status_line: true }` and run `armadai link --target claude`: it writes the plugin `armadai-ui` (see [Link](link.md#claude-code)), whose `subagentStatusLine` runs the hidden command `armadai __claude-subagent-statusline`. Claude Code pipes it one JSON object with the session `session_id`, its `cwd` and a `tasks` array; for each task whose `agentType` holds a running step of a run in progress, the command prints one JSON line `{"id": "<task id>", "content": "<the line of that run naming that step>"}` (coloured; the other tasks keep Claude Code's row). It prints nothing (0 bytes, exit code 0) on an empty or invalid input and never blocks. Format read on [code.claude.com/docs/en/statusline](https://code.claude.com/docs/en/statusline) (2026-11-05).
- The latest log is a guess when two sessions run at once in the same project: pass the session explicitly when you can.

## Machine-readable output

`armadai flow status --json` prints the session as one JSON document on stdout, and nothing else (it does not combine with `--watch` or `--list`). The `flow` argument and `--session` apply as for the text. The schema only grows: read the keys you need. `steps[].agent` (the agent that holds the step, always present) and the value `skipped` of `mark` were added without a new schema version; a reader that matches `mark` strictly must accept it.

```json
{
  "schema": "armadai.flow-status/1",
  "session": { "id": "s1", "started_at": "2026-10-22T09:15:00Z", "last_event_at": "2026-10-22T09:15:50Z" },
  "runs": [
    {
      "flow": "deliver",
      "lot": "menu",
      "status": "in-progress",
      "steps": [ { "step": "guard", "agent": "qa", "mark": "running", "sign": "●", "state": null, "note": null, "ref": "feature/menu" } ],
      "loops": [ { "from": "build", "to": "guard", "times": 2 } ],
      "transitions": [ { "at": "2026-10-22T09:15:20Z", "from": "design", "to": "build", "state": "done" } ]
    }
  ]
}
```

- The schema only grows: a key is never renamed, retyped or removed, a new key is added. A change that breaks a reader gets a new `schema` value (`armadai.flow-status/2`). Read the keys you need and ignore the others.
- `runs[].lot` and `steps[].ref` are always present: `null` when the run has no lot or the step no `REF:`. The text and Markdown outputs of a session with no marker and no `REF:` are unchanged byte for byte.
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
| `skipped` | `⤼` | An optional step that did not run; `note` is `skipped` |

## Optional steps

A step marked `optional: true` in its flow (see [Project format](project-format.md)) is one a run may
never reach. While it has not started, `flow status` treats it as it treats a coordinator's step:
what follows it can start without it. It shows `⤼ skipped` (JSON `mark: "skipped"`, `sign: "⤼"`,
`note: "skipped"`) as soon as a step it reaches has started, or the run is over; before that it stays
`○ pending`. A pending optional step does not hold the run open: a flow whose last step is optional
finishes when the step before it does, with that step skipped. The link taken goes straight from the
step before to the one after (`build → release` in the example below), with no line into the skipped step. An optional
step that ran in a turn of a loop keeps its `✓` and is crossed again in the next turn. `--line` does
not count skipped steps, and the animated `--watch` draws `⤼` in the box of the step.

```
opt  in progress
  ✓ design   done
  ✓ build    done
  ⤼ check    (skipped)
  ● release  running
```

## Markdown snapshot

`armadai flow status --format md` prints the same runs as Markdown, for a pull request, an issue or a file VS Code previews (it does not combine with `--watch`, `--list`, `--json` or `--line`, and is never coloured). `flow` and `--session` apply as for the text.

```markdown
# Flow status: session s1

Snapshot at the last recorded event, 2026-10-22 09:15:50 UTC.

## deliver: in progress

| Step | Mark | Status |
|---|---|---|
| design | ✓ | done |
| guard | ● | running |

Links taken more than once: build → guard (2).

| Time (UTC) | From | To | State |
|---|---|---|---|
| 2026-10-22 09:15:20 | design | build | done |
```

The Time column shows the time only, with the date added on the first row of a table and whenever the UTC day changes. The Status column is the text of the plain output (the state, or `running`/`pending`, then the note in parentheses). A session with no run says `No run of a declared flow in this session yet.`; with no log at all, the usual `no flow activity recorded yet` line.

## Panel (mod)

`ui: { panel: true }` in `project.yaml`, then `armadai link --target claude`, adds a Claude Code mod to the plugin `armadai-ui` (the same plugin as `ui.subagent_status_line`; either key writes it). The mod is three files in `.armadai/plugins/armadai-ui/`: `hooks/hooks.json` (`{"modules": ["./register.js"]}`), `hooks/register.js` (the same bytes for every project, embedded in the `armadai` binary) and `panel.json` (`{"documents": [...]}`, the data: `ui.documents`, or `["docs"]` when the key is absent). Format read 2026-11-09 on [code.claude.com/docs/en/plugins/mods/overview](https://code.claude.com/docs/en/plugins/mods/overview.md) and [mods/reference](https://code.claude.com/docs/en/plugins/mods/reference.md).

The panel opens when a session starts (Claude Code seats a pane nobody asked for from 144 terminal columns; the command `/armadai-panel` opens it at any width). Every 2 seconds it runs `armadai flow status --json` (the `armadai` of your `PATH`, in the project folder) and draws each run with its steps and their signs, then lists the `.md` files under the `ui.documents` folders (up to 4 levels deep) modified since the Claude Code session began, newest first, 20 at most. Nothing new is recorded: the list comes from the disk. If the JSON has a `schema` other than `armadai.flow-status/1`, the panel says `armadai flow status --json gave schema <value>; this panel reads armadai.flow-status/1. Run armadai link again.` and draws no run.

`link` reports `ui.panel` as `approximated`, never `compiled`: it cannot know whether Claude Code loads the mod. The mod runs JavaScript and the `armadai` command with your permissions; it needs Claude Code 2.1.318 or later and does not run under `--safe-mode`, with `disableAllHooks`, or in `claude -p`. Other targets report `not-compiled (Claude Code only)`. Without the key, `link` writes no mod file.

The JavaScript is tested with `claude plugin test` (`crates/armadai/src/compile/ui/armadai-ui-panel.test.ts`, copied into a linked plugin folder); CI has no Claude Code, so it does not run there.

## Statistics: `flow stats`

`armadai flow stats` reads the same log and says how a session went: calls, durations, verdicts, loops.

```bash
armadai flow stats                      # the latest session, grouped by agent
armadai flow stats deliver --by step    # only the calls and loops of the flow `deliver`, per step
armadai flow stats --all --by lot       # every recorded session together, per lot
armadai flow stats --session 3f2a9c     # another session
armadai flow stats --json
```

| Option | Meaning |
|---|---|
| `[FLOW]` | Only the calls and loops of this flow |
| `--session <ID>` | A session (see `flow status --list`); the latest one by default |
| `--all` | Every recorded session together |
| `--by agent\|step\|lot` | What to group by (default `agent`) |
| `--json` | One JSON document on stdout, and nothing else |

```
flow stats  session s1  by agent
AGENT      CALLS  RESUMED  MEDIAN  MAX  TOTAL  STATES                  REJECTED  LOOPS
architect      2        0  5s      5s   10s    -                       -         0
qa             2        0  5s      5s   10s    approved 1, rejected 1  50%       0
durations run from a start to the first stop of the same agent: a resumed agent counts once, so they are lower bounds
states count each verdict rendered, a resumed agent can render several
```

- A **call** is a sub-agent that started. Its duration runs from its start to its first `stop`; an agent resumed by `SendMessage` counts once (`RESUMED` counts the restarts) and its later segments are not timed, hence the note under the table. Durations read `45s`, `4m12s`, `1h03m`; the median of an even number of calls is the mean of the two in the middle, rounded down to the second.
- `STATES` counts each verdict a call rendered; `REJECTED` is the share of `rejected` or `rejected-*` among the verdicts of calls whose step declares such a state (`-` when there is none); `LOOPS` counts the links taken more than once, as `flow status` does (the sum of `times - 1`, by the step they leave).
- `--by step` groups by `<flow>/<step>` and `--by lot` by lot (`(none)` for a marker without one): both count the calls a [marker](#the-delegation-marker) places, and a line says how many were left out (`4 calls are not marked and are left out of --by step`, or `1 call is not marked and is left out …`). Without any marked call: `no call is marked: --by step and --by lot need [flow/step#lot] markers (see armadai flow status)`, exit code 0. With no log: `no flow activity recorded yet (link installs the hooks; start a flow in Claude Code)`, exit code 0. With a `<flow>` none of whose calls is in the session: `no call of flow <flow> in this session`, exit code 0 (`in these sessions` with `--all`).
- **`--by agent` needs no marker, and cannot tell which step a call served**: an agent that holds a step that rejects and one that does not counts the verdicts of both in the denominator of `REJECTED`. Group by step to read a rejection rate per step.

`--json` prints `armadai.flow-stats/1`; keys are always present, the schema only grows (a change that breaks a reader gets `armadai.flow-stats/2`):

```json
{"schema":"armadai.flow-stats/1","sessions":["3f2a9c"],"by":"agent","groups":[{"key":"qa","calls":2,"resumed":0,"duration_s":{"median":5,"max":5,"total":10},"states":{"approved":1,"rejected":1},"rejection_rate":0.5,"loops":0}],"unmarked_calls":0}
```

`rejection_rate` is `null` when no verdict can reject; `unmarked_calls` is `0` for `--by agent`; `groups` is `[]` when no call is marked under `--by step|lot`.

## Limits

- Claude Code only.
- The marker is written by the model. Without `strict` nothing makes it write one; under `strict` the gate refuses a delegation without it, but not a wrong one that still names a step the target holds.
- A `mark` and its `start` are matched by the order of the log, not by a call id: two parallel calls to one agent name with different markers are shown `?` (`ambiguous marker`), never guessed.
- A resumption (`SendMessage`) writes no `mark` and is not judged by the gate: it inherits the marker of the first start, and the durations of `flow stats` stop at the first `stop`.
- An optional step that starts late (after the step that follows it started, from the same stop) is
  shown done and does not reopen the run. If the step that follows it then starts again in that same
  turn, that start has no incoming line in the list of transitions (the run itself stays right; not
  measured beyond that sequence).
- A marked run is closed only by its natural end: a step that never comes (a skipped review) leaves it `in progress`.
- `REF:` is read from the reply of a sub-agent only, and the first one recorded wins.
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
- `--watch` on Windows is not tested: it draws without hiding the cursor. A terminal that shows `●` or `✓` two columns wide shifts the boxes. A run longer than the terminal is cut, its last line saying how many lines are left out.
