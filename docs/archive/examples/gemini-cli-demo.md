# Demo: Fleet d'analyse Rust avec Gemini CLI

Deploy a code analysis crew in one command, let Gemini CLI handle the orchestration.

## Prerequisites

```bash
# Gemini CLI installed
gemini --version

# ArmadAI built
cargo build --release
alias armadai=./target/release/armadai
```

## 1. Create the fleet

```bash
cd ~/my-rust-project

# Install the 5 agents + generate a pre-configured armadai.yaml
armadai init --pack code-analysis-rust --project
```

Expected output:

```
Installing starter pack: code-analysis-rust — Code analysis crew for Rust projects
  installed  ~/.config/armadai/agents/lead-analyst.md
  installed  ~/.config/armadai/agents/rust-reviewer.md
  installed  ~/.config/armadai/agents/rust-test-analyzer.md
  installed  ~/.config/armadai/agents/rust-doc-writer.md
  installed  ~/.config/armadai/agents/rust-security.md
  installed  ~/.config/armadai/prompts/analysis-standards.md
Pack 'code-analysis-rust' installed: 5 agent(s), 1 prompt(s)

Created armadai.yaml with pack 'code-analysis-rust' agents
  Run `armadai link` to generate target config files.
```

The generated `armadai.yaml`:

```yaml
agents:
  - lead-analyst
  - rust-reviewer
  - rust-test-analyzer
  - rust-doc-writer
  - rust-security
prompts:
  - analysis-standards
link:
  target: gemini
  coordinator: lead-analyst
```

## 2. Validate the agents

```bash
# Quick validation of all agents
armadai validate

# Inspect the coordinator — check that Pipeline is parsed
armadai inspect lead-analyst
```

You should see:

```
## Pipeline
  -> rust-reviewer
  -> rust-test-analyzer
  -> rust-doc-writer
  -> rust-security
```

## 3. Generate Gemini CLI config

```bash
# Preview without writing
armadai link --dry-run

# Generate into .gemini/
armadai link
```

Generated files:

```
.gemini/
├── GEMINI.md                        ← Coordinator (lead-analyst prompt + team table)
└── agents/
    ├── rust-reviewer.md             ← Code quality specialist
    ├── rust-test-analyzer.md        ← Test coverage specialist
    ├── rust-doc-writer.md           ← Documentation specialist
    └── rust-security.md             ← Security audit specialist
```

`GEMINI.md` contains the Lead Analyst system prompt with a team table linking to each specialist. Gemini CLI discovers it automatically on launch.

## 4. Use Gemini CLI — it handles the rest

```bash
# Gemini auto-discovers .gemini/GEMINI.md
gemini
```

Example prompts to try:

```
> Review the error handling in src/parser/
# → Lead Analyst delegates to Rust Reviewer

> Are there enough tests for the CLI module?
# → Delegates to Rust Test Analyzer

> Check for unsafe code and dependency vulnerabilities
# → Delegates to Rust Security

> Full analysis of the project
# → All 4 specialists combined, unified report
```

## 5. Variant: target a single specialist

```bash
# Link only the reviewer, no coordinator
armadai link --agents rust-reviewer

# Result: GEMINI.md points directly to the reviewer
gemini
> Review src/core/agent.rs
```

## Fleet composition

| Agent | Role | Scope | Temperature |
|-------|------|-------|-------------|
| **Lead Analyst** | Coordinator — dispatches to specialists | — | 0.4 |
| Rust Reviewer | Code quality (bugs, idioms, patterns) | `src/**/*.rs` | 0.3 |
| Rust Test Analyzer | Test coverage and quality | `src/`, `tests/` | 0.3 |
| Rust Doc Writer | Documentation review and generation | `docs/`, `*.md`, `src/**/*.rs` | 0.5 |
| Rust Security | Security audit and vulnerability scanning | `src/`, `Cargo.toml`, `Cargo.lock` | 0.2 |

All agents use `provider: google` / `model: gemini-2.5-pro`.
