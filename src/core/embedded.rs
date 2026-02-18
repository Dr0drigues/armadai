use std::path::Path;

/// Check whether an extracted embedded directory needs updating.
///
/// Returns `true` if the directory doesn't contain `.armadai-version`
/// or if the stored version differs from the current binary version.
pub(crate) fn needs_update(dest: &Path) -> bool {
    let version_file = dest.join(".armadai-version");
    match std::fs::read_to_string(&version_file) {
        Ok(v) => v.trim() != env!("CARGO_PKG_VERSION"),
        Err(_) => true,
    }
}

/// Write the current binary version into `.armadai-version` inside `dest`.
pub(crate) fn write_version_marker(dest: &Path) {
    let _ = std::fs::write(dest.join(".armadai-version"), env!("CARGO_PKG_VERSION"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_needs_update_no_dir() {
        assert!(needs_update(Path::new("/nonexistent/path")));
    }

    #[test]
    fn test_needs_update_no_version_file() {
        let dir = tempfile::tempdir().unwrap();
        assert!(needs_update(dir.path()));
    }

    #[test]
    fn test_needs_update_current_version() {
        let dir = tempfile::tempdir().unwrap();
        write_version_marker(dir.path());
        assert!(!needs_update(dir.path()));
    }

    #[test]
    fn test_needs_update_old_version() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".armadai-version"), "0.0.1").unwrap();
        assert!(needs_update(dir.path()));
    }
}
