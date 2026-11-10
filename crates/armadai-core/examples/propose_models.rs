//! Prints the tier table a release would propose, resolved from the local
//! models cache: `propose_models <AAAA-MM-JJ>`. Used by `scripts/refresh-models.sh`.

fn main() {
    let as_of = std::env::args()
        .nth(1)
        .expect("usage: propose_models <AAAA-MM-JJ>");
    let table = armadai_core::model_table::propose_tier_table(&as_of);
    println!("{}", serde_json::to_string_pretty(&table).unwrap());
}
