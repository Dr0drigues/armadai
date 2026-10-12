//! `armadai` with no subcommand used to start the conversational shell. The
//! shell is gone: it must print the help and exit, never hang or panic.

#[cfg(test)]
mod tests {
    use assert_cmd::Command;

    #[test]
    fn no_subcommand_prints_the_help_instead_of_starting_a_shell() {
        let dir = tempfile::tempdir().unwrap();
        let output = Command::cargo_bin("armadai")
            .unwrap()
            .current_dir(dir.path())
            .env("ARMADAI_CONFIG_DIR", dir.path().join("config"))
            .timeout(std::time::Duration::from_secs(10))
            .output()
            .unwrap();
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            text.contains("Usage:"),
            "expected the help on no subcommand, got:\n{text}"
        );
        // The full help, not a bare usage error: only the help lists the
        // subcommands under `Commands:`.
        assert!(
            text.contains("Commands:"),
            "expected the full help with its command list, got:\n{text}"
        );
        assert!(
            text.contains("link"),
            "the help must list the kept commands, got:\n{text}"
        );
        // clap's `arg_required_else_help` exits with 2 after printing the help.
        assert_eq!(
            output.status.code(),
            Some(2),
            "unexpected exit status, output:\n{text}"
        );
        assert!(
            !text.contains("panicked"),
            "no subcommand must not panic:\n{text}"
        );
    }
}
