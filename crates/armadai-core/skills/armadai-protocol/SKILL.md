---
name: armadai-protocol
description: ArmadAI response markers (END, DELEGATE, META) that armadai link writes into generated coordinator instructions — no longer consumed by ArmadAI
version: "1.0.0"
tools: []
---

# ArmadAI Response Protocol

> **Status: no consumer.** `armadai link` still writes this protocol into the coordinator's root
> instructions file it generates (`.claude/CLAUDE.md` and its equivalents for Codex, OpenCode and
> Copilot). The ArmadAI shell relay that parsed these markers is no longer part of ArmadAI, so the
> markers have **no automatic effect** today: nothing reads them, and emitting them changes
> nothing in Claude Code, Codex, OpenCode or Copilot. The adapters that write them are to be
> rewritten (planned); until then, treat this page as a description of what the generated files
> contain, not of a working mechanism.

The protocol, as written into the generated files, asks the agent to follow it for all its
responses.

## Markers

### End of Response

When you have completely finished your response, end with this marker on its own line:

```
<!--ARMADAI_END-->
```

This marker was meant to signal to the (removed) shell relay that the response was complete.

### Delegation

When delegating a task to a sub-agent, prefix the delegation with:

```
<!--ARMADAI_DELEGATE:agent-name-->
```

Where `agent-name` is the exact name (or slug) of the agent you're delegating to. This marker should appear inline where the delegation occurs.

### Metadata

At the very end of your response (just before the END marker), include metadata:

```
<!--ARMADAI_META:key1=value1,key2=value2-->
```

Common metadata keys:
- `status` — `complete`, `partial`, `error`, `delegated`
- `tokens` — Estimated token count (optional)
- `delegated_to` — Agent name if you delegated the task

## Complete Example

Here's a complete response following the protocol:

```markdown
I analyzed the code and found two issues:

1. Missing error handling on line 42
2. Unused variable on line 15

I recommend adding a Result return type and removing the unused variable.

<!--ARMADAI_META:status=complete-->
<!--ARMADAI_END-->
```

## Example with Delegation

```markdown
I'll delegate the code review to the QA specialist.

<!--ARMADAI_DELEGATE:qa-specialist-->

Please review the changes in src/core/agent.rs and verify test coverage.

<!--ARMADAI_META:status=delegated,delegated_to=qa-specialist-->
<!--ARMADAI_END-->
```

## Rules

1. The `END` marker MUST be the very last line of your response (no text after it)
2. The `META` marker MUST appear just before the `END` marker
3. `DELEGATE` markers appear inline where delegation occurs
4. Never omit the `END` marker, even for short responses
5. Markers must be on their own line
6. Markers are HTML comments and will not be visible in rendered Markdown

## Why This Protocol?

These were the reasons for the protocol when the shell relay existed; none of them applies while
nothing consumes the markers.

- **Deterministic parsing**: The shell relay could reliably detect the end of a response
- **Streaming support**: No need to wait for timeout or guess when response is complete
- **Metadata extraction**: Status, delegation info, and other data can be extracted without LLM parsing
- **Clean output**: Markers are stripped for display, users see only your actual response
- **Composability**: Multiple agents relayed through the shell could use the same marker protocol to signal delegation. This was a **shell-relay** mechanism only. It is unrelated to the `orchestration:` declaration of the project config, which the Claude Code policy gate reads (see the `armadai-orchestration-patterns` skill). Empirically, only Claude Code reliably emitted these markers for sub-agent delegation.
