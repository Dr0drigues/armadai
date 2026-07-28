pub mod queries;
pub mod schema;

use rusqlite::Connection;
use std::path::Path;
use std::sync::{Arc, Mutex};

pub type Database = Arc<Mutex<Connection>>;

/// Open a persistent SQLite database at `path` and apply the schema.
/// Pure storage primitive: no config/path-resolution logic (that lives
/// bin-side in `crate::db`).
pub fn open(path: &Path) -> anyhow::Result<Database> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let conn = Connection::open(path)?;
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
