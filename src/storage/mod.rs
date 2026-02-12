pub mod embedded;
pub mod queries;
pub mod schema;

use surrealdb::Surreal;
use surrealdb::engine::local::{Db, Mem};

pub type Database = Surreal<Db>;

/// Initialize an embedded in-memory SurrealDB instance.
pub async fn init_embedded() -> anyhow::Result<Database> {
    let db = Surreal::new::<Mem>(()).await?;
    db.use_ns("armadai").use_db("main").await?;
    schema::apply(&db).await?;
    Ok(db)
}

/// Initialize the best available database backend.
/// Prefers RocksDB (persistent) when available, falls back to in-memory.
pub async fn init_db() -> anyhow::Result<Database> {
    #[cfg(feature = "storage-rocksdb")]
    {
        let path = "data/armadai.db";
        if let Some(parent) = std::path::Path::new(path).parent() {
            std::fs::create_dir_all(parent)?;
        }
        return embedded::init_persistent(path).await;
    }
    #[cfg(not(feature = "storage-rocksdb"))]
    {
        init_embedded().await
    }
}
