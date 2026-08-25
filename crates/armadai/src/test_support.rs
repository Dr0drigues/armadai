//! Test-only helpers shared across this binary's modules.
//!
//! The generic environment isolation lives in
//! [`armadai_core::test_support`]; this module holds only what is specific to
//! the `armadai` binary — currently the storage redirect, which existed as two
//! byte-for-byte identical copies (`cli::run` and `web::api`) before #365.

/// Redirect `storage` at a throwaway temp DB for the scope of a test, so a
/// handler's persistence (`SqliteLog` via `crate::db::init_db`) never writes
/// into the user's real event log (#267).
///
/// Wraps [`armadai_core::test_support::IsolatedConfigDir`] — which supplies
/// the temp `ARMADAI_CONFIG_DIR`, the shared env lock and the restore-on-drop
/// — and only adds the `config.yaml` whose `storage.path` points at a scratch
/// sqlite file.
pub struct TempStorageGuard {
    _config: armadai_core::test_support::IsolatedConfigDir,
}

impl TempStorageGuard {
    pub fn new() -> Self {
        let config = armadai_core::test_support::IsolatedConfigDir::enter();
        let db_path = config.config_dir().join("test.sqlite");
        let config_yaml = format!(
            "storage:\n  mode: embedded\n  path: \"{}\"\n",
            db_path.display()
        );
        std::fs::write(config.config_dir().join("config.yaml"), config_yaml).unwrap();
        Self { _config: config }
    }
}
