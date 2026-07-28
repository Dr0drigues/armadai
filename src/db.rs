//! Bin-side database bootstrap (OH7 Lot 2b).
//!
//! Owns the config-driven path resolution that used to live in the storage
//! module's `init_db`. Depends on `crate::core::config`; delegates the actual
//! open+schema to the storage wrapper (`crate::storage::open`), which is
//! core-free. Kept bin-side so `armadai-storage` stays a pure rusqlite leaf.

use std::path::{Path, PathBuf};

use crate::storage::Database;

/// Resolve a possibly-relative `storage.path` to an absolute path.
///
/// Absolute paths are used verbatim. A relative path (e.g. the legacy default
/// `data/armadai.sqlite`) is anchored under the user data dir so the DB is
/// CWD-independent and `--resume`/`--replay` work from any directory (#266),
/// warning once so the user can migrate their config to an absolute path.
fn resolve_storage_path(configured: &str) -> PathBuf {
    let p = Path::new(configured);
    if p.is_absolute() {
        return p.to_path_buf();
    }
    static WARNED: std::sync::Once = std::sync::Once::new();
    WARNED.call_once(|| {
        tracing::warn!(
            "storage.path '{configured}' is relative and was resolved under the data dir \
             (it used to be CWD-relative, which fragments the event log per directory and \
             breaks --resume/--replay from another directory). Set an absolute storage.path \
             in config.yaml to silence this."
        );
    });
    crate::core::config::data_dir().join(p)
}

/// Initialize a persistent SQLite database at the configured path.
pub fn init_db() -> anyhow::Result<Database> {
    let config = crate::core::config::load_user_config();
    let path = resolve_storage_path(&config.storage.path);

    // Safety net (#267): no test may open the real user database.
    #[cfg(test)]
    {
        let real = crate::core::config::data_dir();
        assert!(
            !path.starts_with(&real),
            "init_db() would open the real user database at {} during a test — \
             redirect storage (ARMADAI_CONFIG_DIR -> temp config) or use init_embedded()",
            path.display()
        );
    }

    crate::storage::open(&path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absolute_storage_path_used_verbatim() {
        let abs = if cfg!(windows) {
            r"C:\tmp\db.sqlite"
        } else {
            "/tmp/db.sqlite"
        };
        assert_eq!(resolve_storage_path(abs), PathBuf::from(abs));
    }

    #[test]
    fn relative_storage_path_anchored_under_data_dir() {
        let resolved = resolve_storage_path("data/armadai.sqlite");
        assert!(resolved.is_absolute());
        assert!(resolved.starts_with(crate::core::config::data_dir()));
        assert!(resolved.ends_with("data/armadai.sqlite"));
    }
}
