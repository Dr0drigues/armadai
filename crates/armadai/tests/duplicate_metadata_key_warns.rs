//! Black-box regression for #396: `## Metadata`, `## Triggers` and
//! `## Ring Config` are read line by line and the **last** value of a repeated
//! key wins — silently.
//!
//! This is a wiring test on purpose, and it is the load-bearing one. The
//! parser's unit tests prove which duplicates are *detected*; a warning that is
//! computed but never printed leaves every one of them green. What matters here
//! is that the real binary puts the sentence on stderr, so these assertions run
//! `armadai` itself — `migrate`, the command that reads legacy agents since step 2.
//!
//! Measured on `master` before the fix, on an agent whose `## Metadata` carries
//! an `### Alternative setup (not in use)` block:
//!
//! ```text
//! $ armadai inspect dup
//!   Provider:       openai       # declared anthropic above
//!   Model:          gpt-4o-mini  # declared claude-sonnet-4-5-20250929 above
//!   Temperature:    1            # declared 0.2 above
//! EXIT=0            # and not one word on stderr
//! ```
//!
//! The values themselves are deliberately left alone: last-wins stays. Only the
//! silence is fixed — see the commit message for why this warns instead of
//! failing.

#[cfg(test)]
mod tests {
    use assert_cmd::Command;
    use std::path::{Path, PathBuf};

    /// Duplicates in all three sections at once, each pair with **different**
    /// values so an assertion can name which one won: a fixture repeating the
    /// same value would make the warning unfalsifiable.
    const DUP_AGENT: &str = "\
# dup

## Metadata
- provider: anthropic
- model: claude-sonnet-4-5-20250929
- temperature: 0.2

### Alternative setup (not in use)
- provider: openai
- model: gpt-4o-mini
- temperature: 1.0

## System Prompt
You are dup.

## Triggers
- requires: [alpha]
- priority: 10

### Alternative triggers (not in use)
- requires: [beta]
- priority: 99

## Ring Config
- role: specialist
- vote_weight: 1.0

### Alternative ring (not in use)
- role: coordinator
- vote_weight: 9.0
";

    /// Negative control. Every recognised key appears once; two `###` blocks
    /// carry lines that are *not* recognised keys, plus prose with a colon in
    /// it. None of that may produce a duplicate warning.
    const CLEAN_AGENT: &str = "\
# clean

## Metadata
- provider: anthropic
- model: claude-sonnet-4-5-20250929
- temperature: 0.2

### Notes
- reviewer: someone
- reviewer: someone else
- Rationale: kept at 0.2 for reproducibility

## System Prompt
You are clean.

## Triggers
- requires: [alpha]
- priority: 10

## Ring Config
- role: specialist
- vote_weight: 1.0
";

    fn isolated_library(dir: &Path, agents: &[(&str, &str)]) -> PathBuf {
        let config = dir.join("config");
        std::fs::create_dir_all(config.join("agents")).unwrap();
        for (name, body) in agents {
            std::fs::write(config.join("agents").join(format!("{name}.md")), body).unwrap();
        }
        config
    }

    fn armadai(config: &Path, root: &Path) -> Command {
        let mut cmd = Command::cargo_bin("armadai").unwrap();
        cmd.current_dir(root).env("ARMADAI_CONFIG_DIR", config);
        cmd
    }

    fn project(dir: &Path, config_yaml: &str) -> PathBuf {
        let root = dir.join("project");
        std::fs::create_dir_all(root.join(".armadai")).unwrap();
        std::fs::write(root.join(".armadai/config.yaml"), config_yaml).unwrap();
        root
    }

    /// The exact sentence, for one override. Asserting on the whole line — not
    /// on `contains("duplicate")` — is what pins *which* value lost and which
    /// won; a warning naming them the wrong way round must fail here.
    fn expected(source: &Path, section: &str, key: &str, loser: &str, winner: &str) -> String {
        format!(
            "{}: ## {section} sets '{key}' again: '{loser}' is overridden by \
             '{winner}' (the last value wins)",
            source.display()
        )
    }

    #[test]
    fn every_duplicated_metadata_key_is_named_on_stderr() {
        let dir = tempfile::tempdir().unwrap();
        let config = isolated_library(dir.path(), &[("dup", DUP_AGENT)]);
        let root = project(dir.path(), "agents:\n  - name: dup\n");
        let source = config.join("agents/dup.md");

        let output = armadai(&config, &root)
            .args(["migrate", "--dry-run"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "inspect must still succeed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stderr = String::from_utf8_lossy(&output.stderr);

        for (section, key, loser, winner) in [
            ("Metadata", "provider", "anthropic", "openai"),
            (
                "Metadata",
                "model",
                "claude-sonnet-4-5-20250929",
                "gpt-4o-mini",
            ),
            ("Metadata", "temperature", "0.2", "1.0"),
            ("Triggers", "requires", "[alpha]", "[beta]"),
            ("Triggers", "priority", "10", "99"),
            ("Ring Config", "role", "specialist", "coordinator"),
            ("Ring Config", "vote_weight", "1.0", "9.0"),
        ] {
            let line = expected(&source, section, key, loser, winner);
            assert!(
                stderr.contains(&line),
                "missing warning:\n  {line}\ngot stderr:\n{stderr}"
            );
            // One line per override, not two. Nothing else pins the count:
            // measured, emitting every warning twice left the whole suite
            // green. `migrate --dry-run` is the surface to assert it on — it parses
            // each file once. Pinning the count here says
            // "one warning per override" without rusting the moment #419 is
            // fixed.
            assert_eq!(
                stderr.matches(&line).count(),
                1,
                "warning repeated:\n  {line}\ngot stderr:\n{stderr}"
            );
        }

        // Which value wins is deliberately unchanged: this issue is about the
        // silence, not the precedence. If that ever becomes a real decision,
        // these three lines are what says it was one.
        // (`migrate` reads legacy agents with this parser since `inspect` moved to
        // the canonical model.)
        let done = armadai(&config, &root).arg("migrate").output().unwrap();
        assert!(
            done.status.success(),
            "{}",
            String::from_utf8_lossy(&done.stderr)
        );
        let migrated = std::fs::read_to_string(root.join(".armadai/agents/dup.md")).unwrap();
        assert!(
            migrated.contains("temperature: 1.0"),
            "last-wins changed:\n{migrated}"
        );
    }

    /// A clean agent must produce no duplicate warning at all — including for
    /// the repeated `reviewer:` key, which the parser ignores anyway, and for a
    /// prose line that merely contains a colon.
    #[test]
    fn an_agent_without_duplicates_warns_about_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let config = isolated_library(dir.path(), &[("clean", CLEAN_AGENT)]);
        let root = project(dir.path(), "agents:\n  - name: clean\n");

        let output = armadai(&config, &root)
            .args(["migrate", "--dry-run"])
            .output()
            .unwrap();
        assert!(output.status.success());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            !stderr.contains("is overridden by"),
            "no key is set twice in this agent, yet: {stderr}"
        );
    }
}
