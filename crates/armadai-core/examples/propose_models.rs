//! Prints the tier table a release would propose, resolved from the local
//! models cache: `propose_models <AAAA-MM-JJ>`. Used by `scripts/refresh-models.sh`.

fn main() {
    let as_of = std::env::args()
        .nth(1)
        .expect("usage: propose_models <AAAA-MM-JJ>");
    // A vendor the cache does not know keeps its row unchanged: say so.
    let cache = std::fs::read_to_string(
        armadai_core::config::config_dir().join(armadai_core::config::MODELS_CACHE_FILE),
    )
    .ok()
    .and_then(|c| serde_json::from_str::<serde_json::Value>(&c).ok());
    for vendor in armadai_core::model_table::model_table().tiers.keys() {
        if cache
            .as_ref()
            .and_then(|c| c["providers"].get(vendor))
            .is_none()
        {
            eprintln!("{vendor}: not in the catalog, row kept as is");
        }
    }
    let table = armadai_core::model_table::propose_tier_table(&as_of);
    println!("{}", serde_json::to_string_pretty(&table).unwrap());
}
