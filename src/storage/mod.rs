pub mod queries;
pub mod schema;

use rusqlite::Connection;
use std::sync::Mutex;

pub type Database = Mutex<Connection>;

/// Initialize a persistent SQLite database at the configured path.
pub fn init_db() -> anyhow::Result<Database> {
    let config = crate::core::config::load_user_config();
    let path = &config.storage.path;
    if let Some(parent) = std::path::Path::new(path).parent() {
        std::fs::create_dir_all(parent)?;
    }
    let conn = Connection::open(path)?;
    schema::apply(&conn)?;
    Ok(Mutex::new(conn))
}

/// Initialize an in-memory SQLite database (for tests).
#[cfg(test)]
pub fn init_embedded() -> anyhow::Result<Database> {
    let conn = Connection::open_in_memory()?;
    schema::apply(&conn)?;
    Ok(Mutex::new(conn))
}
