//! Black-box regression for #415: `AgentMetadata.orchestration` is an
//! `OrchestrationPattern` with **five** variants, but `parse_metadata`
//! accepted only three and `bail!`ed on `hierarchical` and `auto`.
//!
//! Since step 2 the legacy parser is read by `migrate` only, so that is the
//! surface measured here.
//!
//! This is a wiring test on purpose. The parser's own unit tests (in
//! `armadai-core::parser::metadata`) prove the five values map to the five
//! variants; they cannot prove the consequence, which is what actually hurt:
//! `parse_agent_file` propagates the error, so the **whole file** became
//! unloadable and the agent disappeared from every surface at once. `list`
//! does warn on stderr, but drops the agent from stdout and still exits 0 —
//! so a script reading stdout sees a shorter fleet and a success code.
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

    /// A legacy project naming every agent of the isolated library: `migrate`
    /// reads them with the parser this file guards (`list` and `inspect` read
    /// the canonical model since step 2).
    fn project(dir: &Path, agents: &[(&str, &str)]) -> PathBuf {
        let root = dir.join("project");
        std::fs::create_dir_all(root.join(".armadai")).unwrap();
        let list: String = agents
            .iter()
            .map(|(n, _)| format!("  - name: {n}\n"))
            .collect();
        std::fs::write(
            root.join(".armadai/config.yaml"),
            format!("agents:\n{list}"),
        )
        .unwrap();
        root
    }

    fn migrate(config: &Path, root: &Path) -> (bool, String) {
        let out = armadai(config, root)
            .args(["migrate", "--dry-run"])
            .output()
            .unwrap();
        (
            out.status.success(),
            format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            ),
        )
    }

    #[test]
    fn five_patterns_are_all_migrated() {
        let dir = tempfile::tempdir().unwrap();
        let config = isolated_library(dir.path(), PATTERNS);
        let root = project(dir.path(), PATTERNS);
        let (ok, text) = migrate(&config, &root);
        assert!(ok, "{text}");
        for (name, pattern) in PATTERNS {
            assert!(
                text.contains(&format!(".armadai/agents/{name}.md")),
                "the '{pattern}' agent was not migrated:\n{text}"
            );
        }
        assert!(!text.contains("not migrated"), "{text}");
    }

    #[test]
    fn an_unknown_pattern_is_still_refused_with_all_five_named() {
        let dir = tempfile::tempdir().unwrap();
        let agents = &[("mesher", "mesh"), ("ringer", "ring")];
        let config = isolated_library(dir.path(), agents);
        let root = project(dir.path(), agents);
        let (_, text) = migrate(&config, &root);
        assert!(
            text.contains(
                "Invalid orchestration: 'mesh'. Expected 'direct', 'blackboard', \
                 'ring', 'hierarchical' or 'auto'"
            ),
            "the refusal must name all five patterns, got:\n{text}"
        );
    }
}
