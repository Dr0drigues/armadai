use std::path::Path;

use armadai_model::*;

fn parsed(name: &str) -> serde_json::Value {
    let (_, json) = schemas().into_iter().find(|(n, _)| *n == name).unwrap();
    serde_json::from_str(&json).unwrap()
}

#[test]
fn committed_schemas_are_up_to_date() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schemas/v1");
    let all = schemas();
    assert_eq!(all.len(), 5);
    for (name, json) in all {
        let committed = std::fs::read_to_string(dir.join(name)).unwrap_or_default();
        assert!(
            committed == json,
            "schemas/v1/{name} is stale: run `cargo run -p armadai-model --example gen_schemas`"
        );
    }
}

#[test]
fn targets_description_is_generated_from_the_constant() {
    let s = parsed("project.schema.json");
    let desc = s["properties"]["targets"]["description"].as_str().unwrap();
    assert!(desc.ends_with(&TARGETS.join(", ")), "{desc}");
    assert!(s["properties"]["targets"].get("enum").is_none());
    assert!(s["properties"]["targets"]["items"].get("enum").is_none());
}

#[test]
fn schemas_reject_unknown_properties() {
    for n in ["project", "agent", "flow", "automation"] {
        let s = parsed(&format!("{n}.schema.json"));
        assert_eq!(s["additionalProperties"], false, "{n}");
    }
    // skill frontmatter belongs to other tools: unknown keys stay allowed
    assert_ne!(parsed("skill.schema.json")["additionalProperties"], false);
}

#[test]
fn files_end_with_a_newline() {
    for (name, json) in schemas() {
        assert!(json.ends_with("}\n"), "{name}");
    }
}
