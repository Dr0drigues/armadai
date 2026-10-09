//! A project linked to Gemini before the refocus must still unlink cleanly or
//! fail with a readable message — never panic, never silently half-undo.

#[cfg(test)]
mod tests {
    use assert_cmd::Command;
    use std::path::Path;

    fn copy_dir(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let target = to.join(entry.file_name());
            if entry.path().is_dir() {
                copy_dir(&entry.path(), &target);
            } else {
                std::fs::copy(entry.path(), target).unwrap();
            }
        }
    }

    #[test]
    fn unlinking_a_project_linked_to_gemini_never_panics_or_half_undoes() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        copy_dir(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/gemini-linked"),
            &root,
        );
        let generated = root.join(".gemini/agents/nested.md");
        assert!(
            generated.is_file(),
            "fixture must contain the Gemini output"
        );

        let output = Command::cargo_bin("armadai")
            .unwrap()
            .current_dir(&root)
            .env("ARMADAI_CONFIG_DIR", dir.path().join("config"))
            .args(["unlink", "--target", "gemini"])
            .output()
            .unwrap();
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!text.contains("panicked"), "unlink panicked:\n{text}");
        if output.status.success() {
            assert!(
                !generated.exists(),
                "unlink reported success but left {} behind:\n{text}",
                generated.display()
            );
        } else {
            assert!(
                !text.trim().is_empty(),
                "unlink failed with no message at all"
            );
            assert!(
                generated.exists(),
                "unlink failed yet deleted the generated file: half-undone state"
            );
        }
    }
}
