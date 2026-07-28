#[cfg(feature = "storage")]
pub mod es_log;
pub mod queries;
pub mod schema;

use rusqlite::Connection;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub type Database = Arc<Mutex<Connection>>;

/// Resolve a possibly-relative `storage.path` to an absolute path.
///
/// Absolute paths are used verbatim. A **relative** path (e.g. the legacy
/// default `data/armadai.sqlite`) used to be opened relative to the current
/// working directory, so the event log lived in a different DB per CWD and
/// `--resume`/`--replay` only worked from the directory the run started in
/// (#266). Anchor such paths under the user data dir instead, so the DB is
/// CWD-independent, and warn once so the user can migrate their config to an
/// absolute path.
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

    // Safety net (#267): no test may open the real user database. Tests must
    // redirect storage (e.g. point `ARMADAI_CONFIG_DIR` at a temp config, or
    // use `init_embedded()`); a test that reaches the real data-dir DB is a
    // silent-pollution bug, so fail loudly at the source instead of writing
    // `test-run`-style rows into the user's real event log.
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

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let conn = Connection::open(&path)?;
    schema::apply(&conn)?;
    Ok(Arc::new(Mutex::new(conn)))
}

/// Initialize an in-memory SQLite database (for tests).
#[cfg(test)]
pub fn init_embedded() -> anyhow::Result<Database> {
    let conn = Connection::open_in_memory()?;
    schema::apply(&conn)?;
    Ok(Arc::new(Mutex::new(conn)))
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
        // The legacy CWD-relative default must resolve to an absolute path
        // under the data dir, not against the current working directory.
        let resolved = resolve_storage_path("data/armadai.sqlite");
        assert!(resolved.is_absolute());
        assert!(resolved.starts_with(crate::core::config::data_dir()));
        assert!(resolved.ends_with("data/armadai.sqlite"));
    }
}
