pub mod embedded;
pub mod queries;
pub mod schema;

use surrealdb::Surreal;
use surrealdb::engine::local::{Db, Mem};

pub type Database = Surreal<Db>;

/// Initialize an embedded in-memory SurrealDB instance.
pub async fn init_embedded() -> anyhow::Result<Database> {
    let db = Surreal::new::<Mem>(()).await?;
    db.use_ns("swarm").use_db("main").await?;
    schema::apply(&db).await?;
    Ok(db)
}
