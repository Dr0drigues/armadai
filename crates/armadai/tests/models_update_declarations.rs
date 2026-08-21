//! Black-box regression: `armadai models update` (both the single-project
//! and `--all` branches, `crates/armadai/src/cli/models.rs`) must route an
//! `agents.yaml` finding through the declarative rewriter
//! (`model_updater::update_declarations`, via `model_updater::
//! apply_findings`), not the `.md` one (`update_agent_file`). The latter's
//! single `replacen(.., 1)` and unbounded `: <model>` pattern can rewrite a
//! comment that happens to contain the deprecated model string, while
//! leaving the real `model:` field untouched — and still report success.

#[cfg(test)]
mod tests {
    use assert_cmd::Command;

    /// Every project directory in this file gets its own isolated
    /// `ARMADAI_CONFIG_DIR` — without it, `models update --all`'s registry
    /// read/write would land in the developer's real `~/.config/armadai/`.
    fn isolated_config(dir: &std::path::Path) -> std::path::PathBuf {
        let config = dir.join("config");
        std::fs::create_dir_all(&config).unwrap();
        config
    }

    /// A project whose `agents.yaml` carries a deprecated model both as the
    /// real `defaults.model` value AND, deliberately positioned earlier in
    /// the file, inside a comment containing the exact same string right
    /// after a colon (`": gpt-4-turbo"`) — the substring
    /// `update_agent_file`'s textual rewrite (wrongly applied to a
    /// declaration file before this fix) matches first, since it scans the
    /// whole file top to bottom with no notion of which key a line
    /// belongs to.
    fn project_with_a_deprecated_model_and_a_lookalike_comment() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        std::fs::create_dir_all(root.join(".armadai")).unwrap();
        std::fs::write(root.join(".armadai/config.yaml"), "agents: []\n").unwrap();
        std::fs::write(
            root.join(".armadai/agents.yaml"),
            "# fallback note: gpt-4-turbo was our old default\n\
             defaults:\n  provider: claude\n  model: gpt-4-turbo\n\
             agents:\n  - name: demo-agent\n",
        )
        .unwrap();
        dir
    }

    fn assert_fixed_correctly(root: &std::path::Path, stdout: &str, stderr: &str) {
        assert!(
            stdout.contains("1 replacement(s)") && stdout.contains("1 model(s) updated"),
            "the reported count must be truthful: stdout={stdout} stderr={stderr}"
        );
        let after = std::fs::read_to_string(root.join(".armadai/agents.yaml")).unwrap();
        assert!(
            after.contains("model: gpt-4o"),
            "the real deprecated field must actually be fixed:\n{after}"
        );
        assert!(
            after.contains("# fallback note: gpt-4-turbo was our old default"),
            "the comment must survive untouched — this is the corruption this fix \
             prevents:\n{after}"
        );
    }

    #[test]
    fn models_update_fixes_the_real_field_and_reports_a_truthful_count() {
        let dir = project_with_a_deprecated_model_and_a_lookalike_comment();
        let root = dir.path().join("project");

        let mut cmd = Command::cargo_bin("armadai").unwrap();
        cmd.current_dir(&root)
            .env("ARMADAI_CONFIG_DIR", isolated_config(dir.path()))
            .arg("models")
            .arg("update");
        let output = cmd.output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        assert!(
            output.status.success(),
            "a clean single-occurrence fix must not error: stdout={stdout} stderr={stderr}"
        );
        assert_fixed_correctly(&root, &stdout, &stderr);
    }

    /// Same fix, the other cited call site: `models update --all`, driven
    /// off the project registry rather than the current directory.
    #[test]
    fn models_update_all_fixes_the_real_field_and_reports_a_truthful_count() {
        let dir = project_with_a_deprecated_model_and_a_lookalike_comment();
        let root = dir.path().join("project");
        let config = isolated_config(dir.path());
        std::fs::write(
            config.join("projects.json"),
            format!(
                r#"{{"projects":[{{"path":"{}","last_seen":"2026-01-01T00:00:00Z"}}]}}"#,
                root.display()
            ),
        )
        .unwrap();

        let mut cmd = Command::cargo_bin("armadai").unwrap();
        cmd.env("ARMADAI_CONFIG_DIR", &config)
            .arg("models")
            .arg("update")
            .arg("--all");
        let output = cmd.output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        assert!(
            output.status.success(),
            "a clean single-occurrence fix must not error: stdout={stdout} stderr={stderr}"
        );
        assert!(
            stdout.contains("1 model(s) updated across all projects"),
            "the --all summary must be truthful too: {stdout}"
        );
        assert_fixed_correctly(&root, &stdout, &stderr);
    }

    /// A project whose `agents.yaml` carries TWO findings for the same
    /// file: a plain `defaults.model` (which the textual rewrite can fix)
    /// and a quoted `"model":` key on an agent (which it cannot — a
    /// structured-parse-vs-textual-scan disagreement). Regression for the
    /// bug a prior fix round reintroduced one layer up: looping
    /// `apply_findings` once per finding let the plain fix land on disk
    /// before the quoted one's failure was discovered, reporting "0
    /// model(s) updated." with exit 0 while the file sat half-rewritten.
    fn project_with_one_fixable_and_one_unfixable_finding() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        std::fs::create_dir_all(root.join(".armadai")).unwrap();
        std::fs::write(root.join(".armadai/config.yaml"), "agents: []\n").unwrap();
        std::fs::write(
            root.join(".armadai/agents.yaml"),
            "defaults:\n  provider: claude\n  model: gpt-4-turbo\nagents:\n  - name: a\n    \"model\": gpt-4-turbo\n",
        )
        .unwrap();
        dir
    }

    #[test]
    fn models_update_leaves_the_file_untouched_and_exits_non_zero_on_a_partial_failure() {
        let dir = project_with_one_fixable_and_one_unfixable_finding();
        let root = dir.path().join("project");
        let agents_yaml = root.join(".armadai/agents.yaml");
        let before = std::fs::read_to_string(&agents_yaml).unwrap();

        let mut cmd = Command::cargo_bin("armadai").unwrap();
        cmd.current_dir(&root)
            .env("ARMADAI_CONFIG_DIR", isolated_config(dir.path()))
            .arg("models")
            .arg("update");
        let output = cmd.output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        assert!(
            !output.status.success(),
            "a file that could not be fully fixed must not exit 0: stdout={stdout}              stderr={stderr}"
        );
        assert!(
            stdout.contains("0 model(s) updated"),
            "the reported count must not claim a fix that did not happen: {stdout}"
        );
        assert!(!stderr.trim().is_empty(), "must report why: {stderr}");
        assert_eq!(
            std::fs::read_to_string(&agents_yaml).unwrap(),
            before,
            "the plain defaults.model fix must NOT land on disk just because the              quoted-key finding in the SAME file failed"
        );
    }
}
