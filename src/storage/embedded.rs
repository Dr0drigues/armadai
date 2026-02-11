use surrealdb::Surreal;
use surrealdb::engine::local::{Db, RocksDb};

/// Initialize a persistent embedded SurrealDB instance backed by RocksDB.
pub async fn init_persistent(path: &str) -> anyhow::Result<Surreal<Db>> {
    let db = Surreal::new::<RocksDb>(path).await?;
    db.use_ns("swarm").use_db("main").await?;
    super::schema::apply(&db).await?;
    Ok(db)
}
