//! JSON Schemas of the v1 file formats, generated from the types.

use schemars::{JsonSchema, schema_for};

use crate::types::{AgentFront, Automation, FlowFile, Manifest, SkillFront};

/// Keys sorted at every level: the bytes must not depend on whether another crate of
/// the build enables serde_json's `preserve_order` (feature unification).
fn sort_keys(v: &mut serde_json::Value) {
    match v {
        serde_json::Value::Object(m) => {
            m.sort_keys();
            m.values_mut().for_each(sort_keys);
        }
        serde_json::Value::Array(a) => a.iter_mut().for_each(sort_keys),
        _ => {}
    }
}

fn render<T: JsonSchema>() -> String {
    let mut v = serde_json::to_value(schema_for!(T)).expect("schema serializes");
    sort_keys(&mut v);
    let mut s = serde_json::to_string_pretty(&v).expect("schema serializes");
    s.push('\n');
    s
}

/// (file name, JSON) of every schema, as committed under `schemas/v1/`.
pub fn schemas() -> Vec<(&'static str, String)> {
    vec![
        ("project.schema.json", render::<Manifest>()),
        ("agent.schema.json", render::<AgentFront>()),
        ("skill.schema.json", render::<SkillFront>()),
        ("flow.schema.json", render::<FlowFile>()),
        ("automation.schema.json", render::<Automation>()),
    ]
}
