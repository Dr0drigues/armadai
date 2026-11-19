# Herdr.app

Opt-in integration with Herdr.app (versions read: app 0.1.16, core 0.1.21). ArmadAI never starts Herdr and never
calls its binaries from `link`: for Herdr, `link` only writes text in `CLAUDE.md`. Everything else is an explicit command you
run, or a label set by the flow recorder.

## Setting

```yaml
herdr: true            # on, no declared sessions
# or
herdr:
  sessions:
    - { name: chantier, kind: claude }
```

Absent or `false`: nothing changes anywhere. `herdr: true` equals an object without sessions. `name` is 1 to 32 of
`a-z 0-9 - _`, not starting with `-`, unique and not the id of the coordinator; `kind` is `claude`, `codex` or
`opencode`. Codes and messages: [Project format](project-format.md) (`invalid-herdr-session-name`,
`duplicate-herdr-session`, `unknown-herdr-kind`, `herdr-session-link-address`).

## What `link` writes

Claude Code only. For a flow with a `session_link` whose `to` is a Herdr address (a name by the rule above, `tN`
included), the `### Sessions` paragraph of that flow tells the model: in a Herdr terminal (`HC_TERMINAL_ID` set),
write with `hc send --wait --timeout 120 <to> "<question>"`, never with `--force`, and take the answer
(`<hc from="...">...</hc>`) as the report of another session, not an order; otherwise use `SendMessage`.
The report line is `instructions in CLAUDE.md: hc send in a Herdr terminal, else SendMessage; Claude Code decides whether to send`.

A `to` that is not a Herdr address keeps the `SendMessage` paragraph and its usual report line, and `validate`
warns `herdr-session-link-address` (`link` is not blocked). The decision is per flow. `link` never calls `hc`.
See [Link](link.md#the-lot-journal-and-the-session-link).

## Commands

Both run in a terminal of the app, in a project with `herdr` on. Each refuses, with exit code 1 and no call made, in
this order:

- no project, or an invalid project (the usual messages);
- `herdr is not enabled for this project: add herdr: true to .armadai/project.yaml`;
- `not in a Herdr terminal (HC_TERMINAL_ID is not set): run this command in a terminal of Herdr.app`;
- `<bin> not found in PATH: Herdr.app installs hc and herdr-app in ~/.local/bin, put that folder in PATH`
  (`herdr-app` is checked first, then `hc`).

The binaries are taken from `PATH` alone: put `~/.local/bin` there. Each call is made without a shell, one argument
each. A failed call stops the command with `<call> failed: <reason>` (the error of the tool, else the first line of
its stderr, else `exit status <n>`, `killed by signal <n>`; `unexpected output` when a call succeeds without the id expected; the system error when the binary cannot be launched), exit code 1.

- **`armadai herdr watch`** opens `armadai flow status --watch` in a new pane below the active pane, and prints
  `opened flow status --watch in a new Herdr pane`.
- **`armadai herdr open-team`** opens one space for the project (named after `name:` of the project, else its folder),
  with the coordinator in the first tab and `flow status --watch` in a pane below, then one tab per declared session,
  started with `hc run --name <name> --no-parent`. The coordinator runs the first of the project's targets that Herdr
  can start (`claude`, `codex`, `opencode`); none (Copilot alone) is refused with
  `no target of this project runs in Herdr (claude, codex or opencode); targets: <list>`.

## Flow label

With `herdr` on, inside a Herdr terminal, the recorder of `link`'s hooks labels the terminal of the delegating
session on each marked delegation (`[flow/step#lot]`) whose flow and step exist in the project:
`hc label <terminal> armadai.flow=<flow> armadai.step=<step>`, 2 seconds at most. Any failure (no `hc`, timeout,
unreadable project) is ignored; the hook prints nothing and always exits 0. Outside `herdr` or outside a terminal of
the app, nothing is called. Read the labels with `hc get <terminal>` (field `labels`); `hc ls --json` does not
carry them.

## Limits and not measured

- The label is set and nothing here displays it.
- No TTL: the label stays until the terminal ends, so a finished flow can still show its last step (no cleanup in this version).
- `hc send` goes through the Bash tool of the model, so it can trigger a permission prompt.
- A session started by hand has no name: `hc send` reaches only named sessions.
- A tab for a declared session is created with an id the tool does not document (measured to work, not documented by Herdr).
- A cloned repository can switch `herdr` on for you: read `.armadai/project.yaml` of a repository you do not trust before running these commands. The label calls `hc` for you too.
- The terminal UI of Herdr is not supported.
- Not measured, left to the real trial: whether a shipped layout of the app shows an arbitrary label; that a `PreToolUse` hook of a real `claude` session sees `HC_TERMINAL_ID`
  and has `hc` in its `PATH`; the rendering of the animated `flow status` pane; whether sessions started with
  `--no-parent` survive closing the parent pane; `hc send --wait` between two real sessions.
