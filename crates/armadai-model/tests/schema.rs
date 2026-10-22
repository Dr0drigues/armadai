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
    assert_eq!(all.len(), 6);
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
    for n in ["project", "agent", "template", "flow", "automation"] {
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

#[test]
fn agent_description_is_required_unless_from_is_set() {
    let s = parsed("agent.schema.json");
    assert_eq!(s.get("required"), None);
    assert_eq!(
        s["anyOf"],
        serde_json::json!([
            { "required": ["description"] },
            { "required": ["from"], "properties": { "from": { "type": "string" } } },
        ])
    );
    for k in ["from", "vars"] {
        assert!(s["properties"].get(k).is_some(), "{k}");
    }
}

#[test]
fn template_schema_has_the_agent_keys_all_optional_without_from_nor_vars() {
    let s = parsed("template.schema.json");
    assert_eq!(s.get("required"), None);
    assert_eq!(s.get("anyOf"), None);
    let keys: Vec<&str> = s["properties"]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        [
            "capabilities",
            "description",
            "native",
            "skills",
            "tags",
            "temperature",
            "tier",
            "title"
        ]
    );
}
