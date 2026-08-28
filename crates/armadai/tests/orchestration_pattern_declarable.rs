//! Black-box regression for #415: `AgentMetadata.orchestration` is an
//! `OrchestrationPattern` with **five** variants, but `parse_metadata`
//! accepted only three and `bail!`ed on `hierarchical` and `auto`.
//!
//! This is a wiring test on purpose. The parser's own unit tests (in
//! `armadai-core::parser::metadata`) prove the five values map to the five
//! variants; they cannot prove the consequence, which is what actually hurt:
//! `parse_agent_file` propagates the error, so the **whole file** became
//! unloadable and the agent disappeared from every surface at once — silently
//! for `list`, which drops it and still exits 0.
//!
//! Measured on `master` before the fix, on a library holding one agent per
//! pattern:
//!
//! ```text
//! $ armadai inspect coord      # - orchestration: hierarchical
//! Error: Invalid orchestration: 'hierarchical'. Expected 'direct', 'blackboard', or 'ring'
//! EXIT=1
//! $ armadai inspect ringer     # - orchestration: ring
//! Agent: Ringer …
//! EXIT=0
//! $ armadai list
//!   3 agent(s) found.          # of the five declared
//! EXIT=0
//! ```

#[cfg(test)]
mod tests {
    use assert_cmd::Command;
    use std::path::{Path, PathBuf};

    /// The five variants of `OrchestrationPattern`, each paired with the agent
    /// name used for it. Keeping all five here is the negative control the
    /// count assertion needs: a fix that accepted *anything* would also pass
    /// `five_patterns_are_all_listed`, so `an_unknown_pattern_is_still_refused`
    /// below pins the other side.
    const PATTERNS: &[(&str, &str)] = &[
        ("directer", "direct"),
        ("blacker", "blackboard"),
        ("ringer", "ring"),
        ("coord", "hierarchical"),
        ("autoer", "auto"),
    ];

    fn agent_md(name: &str, pattern: &str) -> String {
        format!(
            "# {name}\n\n\
             ## Metadata\n\
             - provider: anthropic\n\
             - model: claude-sonnet-4-5-20250929\n\
             - orchestration: {pattern}\n\n\
             ## System Prompt\n\
             You are {name}.\n"
        )
    }

    /// Isolate `~/.config/armadai`: `list`/`inspect` scan the global agent
    /// library, which on a developer machine holds real agents.
    fn isolated_library(dir: &Path, agents: &[(&str, &str)]) -> PathBuf {
        let config = dir.join("config");
        std::fs::create_dir_all(config.join("agents")).unwrap();
        for (name, pattern) in agents {
            std::fs::write(
                config.join("agents").join(format!("{name}.md")),
                agent_md(name, pattern),
            )
            .unwrap();
        }
        config
    }

    fn armadai(config: &Path, root: &Path) -> Command {
        let mut cmd = Command::cargo_bin("armadai").unwrap();
        cmd.current_dir(root).env("ARMADAI_CONFIG_DIR", config);
        cmd
    }

    /// One agent per pattern in the library; every one of them must load.
    /// The count is asserted exactly, not with `contains`: before the fix
    /// `list` printed "3 agent(s) found." and exited 0, so an assertion that
    /// merely looked for the surviving names would have stayed green.
    #[test]
    fn five_patterns_are_all_listed() {
        let dir = tempfile::tempdir().unwrap();
        let config = isolated_library(dir.path(), PATTERNS);
        let root = dir.path().join("project");
        std::fs::create_dir_all(&root).unwrap();

        let output = armadai(&config, &root).arg("list").output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success(),
            "list failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            stdout.contains(&format!("{} agent(s) found.", PATTERNS.len())),
            "expected all {} declared agents to load, got:\n{stdout}",
            PATTERNS.len()
        );
        for (name, pattern) in PATTERNS {
            assert!(
                stdout.to_lowercase().contains(name),
                "the '{pattern}' agent is missing from `list`:\n{stdout}"
            );
        }
    }

    /// `inspect` is the surface #415 measured. Every pattern must exit 0 —
    /// `hierarchical` and `auto` exited 1 before the fix.
    #[test]
    fn every_pattern_can_be_inspected() {
        let dir = tempfile::tempdir().unwrap();
        let config = isolated_library(dir.path(), PATTERNS);
        let root = dir.path().join("project");
        std::fs::create_dir_all(&root).unwrap();

        for (name, pattern) in PATTERNS {
            let output = armadai(&config, &root)
                .args(["inspect", name])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "`inspect {name}` (- orchestration: {pattern}) exited {:?}:\n{}",
                output.status.code(),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    /// The other side of the control: an invented pattern must still be
    /// refused, and the message must enumerate the five real ones. The old
    /// message listed three as if the list were complete.
    #[test]
    fn an_unknown_pattern_is_still_refused_with_all_five_named() {
        let dir = tempfile::tempdir().unwrap();
        let config = isolated_library(dir.path(), &[("mesher", "mesh")]);
        let root = dir.path().join("project");
        std::fs::create_dir_all(&root).unwrap();

        let output = armadai(&config, &root)
            .args(["inspect", "mesher"])
            .output()
            .unwrap();
        assert!(
            !output.status.success(),
            "`- orchestration: mesh` is not a pattern and must be refused"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains(
                "Invalid orchestration: 'mesh'. Expected 'direct', 'blackboard', \
                 'ring', 'hierarchical' or 'auto'"
            ),
            "the refusal must name all five patterns, got:\n{stderr}"
        );
    }
}
