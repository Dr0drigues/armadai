---
name: armadai-protocol
description: Response protocol for ArmadAI shell — standardized markers for end-of-response detection and metadata
version: "1.0.0"
tools: []
---

# ArmadAI Response Protocol

You MUST follow this protocol for ALL your responses when running in ArmadAI shell mode.

## Markers

### End of Response

When you have completely finished your response, end with this marker on its own line:

```
<!--ARMADAI_END-->
```

This marker signals to the shell that you have finished generating your response and that it is safe to proceed.

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

- **Deterministic parsing**: The shell can reliably detect when you've finished responding
- **Streaming support**: No need to wait for timeout or guess when response is complete
- **Metadata extraction**: Status, delegation info, and other data can be extracted without LLM parsing
- **Clean output**: Markers are stripped for display, users see only your actual response
- **Composability**: Multiple agents can use the same protocol for hierarchical delegation
