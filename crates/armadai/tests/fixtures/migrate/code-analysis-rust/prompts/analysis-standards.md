---
name: analysis-standards
description: Shared output format and reporting conventions for code analysis agents
apply_to: [analysis]
---

# Analysis Standards

## Output Format

All analysis reports MUST follow this structure:

### Findings

Each finding uses this format:

```
### [SEVERITY] Title
- **File**: path/to/file.rs:42
- **Description**: Clear explanation of the issue
- **Suggestion**: Concrete fix or improvement
```

### Severity Levels

- **Critical**: Security vulnerability, data loss risk, or crash. Must be fixed immediately.
- **Major**: Bug, significant code smell, or missing error handling. Should be fixed before merge.
- **Minor**: Style issue, minor inefficiency, or non-idiomatic code. Fix when convenient.
- **Suggestion**: Enhancement opportunity, not a defect. Consider for future improvement.

## Reporting Conventions

- Always include file path and line number for each finding
- Provide a code example for the suggested fix when possible
- Do not report false positives — only flag real, actionable issues
- Be constructive: explain WHY something is an issue, not just WHAT is wrong
- Consider the project context — patterns used intentionally should not be flagged

## Summary Table

End every report with a summary table:

```markdown
| Severity   | Count |
|------------|-------|
| Critical   | 0     |
| Major      | 0     |
| Minor      | 0     |
| Suggestion | 0     |
| **Total**  | **0** |
```

## Principles

1. **No false positives** — Every finding must be real and actionable
2. **Prioritize risk** — Critical and security issues first
3. **Be constructive** — Suggest fixes, not just problems
4. **Respect context** — Understand the project's conventions before flagging
5. **Concise** — One finding per issue, no duplication
