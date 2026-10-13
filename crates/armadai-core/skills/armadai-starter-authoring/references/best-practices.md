# Starter pack best practices

- **One domain per pack**: a Rust team, a web analysis crew. 3 to 6 agents.
- **Descriptions say when to use each agent**; prompts state what each does not do.
- **Topology in flows**: a lead handing work out is a `fan-out` flow and the lead as
  `coordinator`; a chain is a `pipeline`; a review with a way back is a `review-loop`. Add no
  coordinator by default.
- **Shared knowledge in skills**, listed in each agent's `skills:` (conventions, a report
  format), not copied into every prompt.
- **Automations sparingly**: each one runs a command in the user's repository; the install
  lists them, keep them obvious (a formatter, a linter).
- **No warning**: `armadai validate <pack>` shows 0 error(s), 0 warning(s).
- **Ship no state**: no `link-manifest.yaml`, no generated `.claude/` files; they are not copied
  anyway.
