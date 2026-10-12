//! Regenerate `schemas/v1/*.schema.json`:
//! `cargo run -p armadai-model --example gen_schemas`

use std::path::Path;

fn main() -> std::io::Result<()> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schemas/v1");
    std::fs::create_dir_all(&dir)?;
    for (name, json) in armadai_model::schemas() {
        std::fs::write(dir.join(name), json)?;
    }
    Ok(())
}
