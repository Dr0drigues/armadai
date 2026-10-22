//! Strict types of the `.armadai/` directory format.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Opaque per-target block: `{ claude|codex|copilot|opencode: <free YAML> }`.
pub type Native = BTreeMap<String, NativeValue>;

/// Free YAML held as JSON data. YAML-only constructs do not survive: tags (`!foo`) are
/// refused on read, anchors and aliases are expanded.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(transparent)]
pub struct NativeValue(pub serde_json::Value);

impl std::ops::Deref for NativeValue {
    type Target = serde_json::Value;
    fn deref(&self) -> &serde_json::Value {
        &self.0
    }
}

impl std::ops::DerefMut for NativeValue {
    fn deref_mut(&mut self) -> &mut serde_json::Value {
        &mut self.0
    }
}

impl From<serde_json::Value> for NativeValue {
    fn from(v: serde_json::Value) -> Self {
        Self(v)
    }
}

impl Serialize for NativeValue {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        PlainNumbers(&self.0).serialize(s)
    }
}

/// Under `arbitrary_precision` (enabled by the binary) a JSON `Number` serializes as a
/// private map: this writes it as a plain YAML number instead.
struct PlainNumbers<'a>(&'a serde_json::Value);

impl Serialize for PlainNumbers<'_> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::{SerializeMap, SerializeSeq};
        use serde_json::Value;
        match self.0 {
            Value::Number(n) => {
                if let Some(u) = n.as_u64() {
                    s.serialize_u64(u)
                } else if let Some(i) = n.as_i64() {
                    s.serialize_i64(i)
                } else if let Ok(i) = n.to_string().parse::<i128>() {
                    s.serialize_i128(i)
                } else {
                    s.serialize_f64(n.as_f64().unwrap_or(f64::NAN))
                }
            }
            Value::Array(a) => {
                let mut q = s.serialize_seq(Some(a.len()))?;
                for v in a {
                    q.serialize_element(&PlainNumbers(v))?;
                }
                q.end()
            }
            Value::Object(m) => {
                let mut q = s.serialize_map(Some(m.len()))?;
                for (k, v) in m {
                    q.serialize_entry(k, &PlainNumbers(v))?;
                }
                q.end()
            }
            v => v.serialize(s),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(description = "Default link targets: claude, codex, copilot, opencode")]
    pub targets: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub outputs: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coordinator: Option<String>,
    #[serde(default, skip_serializing_if = "Policy::is_default")]
    pub policy: Policy,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[schemars(with = "BTreeMap<String, serde_json::Value>")]
    pub native: Native,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    #[serde(default, skip_serializing_if = "is_off")]
    pub delegation: Delegation,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub free_agents: Vec<String>,
}

fn is_off(d: &Delegation) -> bool {
    *d == Delegation::Off
}

impl Policy {
    fn is_default(&self) -> bool {
        *self == Policy::default()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Delegation {
    #[default]
    Off,
    Strict,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    Fast,
    Pro,
    Max,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Cap {
    Allow,
    Ask,
    Deny,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Capabilities {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read: Option<Cap>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub write: Option<Cap>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell: Option<Cap>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub web: Option<Cap>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delegate: Option<Cap>,
}

impl Capabilities {
    fn is_empty(&self) -> bool {
        *self == Capabilities::default()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(transform = description_unless_from)]
pub struct AgentFront {
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier: Option<Tier>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<String>,
    #[serde(default, skip_serializing_if = "Capabilities::is_empty")]
    pub capabilities: Capabilities,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[schemars(with = "BTreeMap<String, serde_json::Value>")]
    pub native: Native,
    /// The agent template this agent is an instance of (always `None` once rendered).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// Values of the template's `{{var}}` (always empty once rendered).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub vars: BTreeMap<String, String>,
}

/// Schema only: an instance (`from`) may take its `description` from the template
/// (the semantic layer still requires one once rendered).
fn description_unless_from(s: &mut schemars::Schema) {
    if let Some(serde_json::Value::Array(r)) = s.get_mut("required") {
        r.retain(|k| k != "description");
        if r.is_empty() {
            s.remove("required");
        }
    }
    s.insert(
        "anyOf".into(),
        serde_json::json!([
            { "required": ["description"] },
            // `from: null` must not count as an instance
            { "required": ["from"], "properties": { "from": { "type": "string" } } },
        ]),
    );
}

/// Frontmatter of `templates/<id>.md`: the keys of an agent, all optional, without
/// `from` nor `vars` (a template does not inherit).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TemplateFront {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier: Option<Tier>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<String>,
    #[serde(default, skip_serializing_if = "Capabilities::is_empty")]
    pub capabilities: Capabilities,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[schemars(with = "BTreeMap<String, serde_json::Value>")]
    pub native: Native,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub agent: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub states: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_failure: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Link {
    pub from: String,
    pub to: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<String>,
}

/// Element of the `sequence` shortcut.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum SeqItem {
    Agent(String),
    Explicit(SeqExplicit),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SeqExplicit {
    pub id: String,
    pub agent: String,
}

/// Raw flow as read from `flows/<id>.yaml`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FlowFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(
        description = "The step the flow begins with; needed when every step has an incoming link"
    )]
    pub start: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub steps: BTreeMap<String, Step>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<Link>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sequence: Vec<SeqItem>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Event {
    SessionStart,
    SessionEnd,
    BeforeTool,
    AfterTool,
    OnPrompt,
    OnStop,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Automation {
    pub event: Event,
    pub run: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[schemars(with = "BTreeMap<String, serde_json::Value>")]
    pub native: Native,
}

/// Skill frontmatter. No `deny_unknown_fields`: the format belongs to other tools,
/// unknown keys are warnings computed by the loader.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SkillFront {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compatibility: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<BTreeMap<String, String>>,
    /// Keys owned by other tools (`allowed-tools`, ...): kept so a rewrite loses nothing.
    #[serde(flatten, skip_serializing_if = "BTreeMap::is_empty")]
    #[schemars(with = "BTreeMap<String, serde_json::Value>")]
    pub extra: BTreeMap<String, NativeValue>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse<T: serde::de::DeserializeOwned>(y: &str) -> Result<T, crate::yaml::Error> {
        crate::yaml::from_str(y)
    }

    #[test]
    fn manifest_minimal_roundtrips() {
        let m: Manifest = parse("schema_version: 1\n").unwrap();
        assert_eq!(m.policy.delegation, Delegation::Off);
        let out = crate::yaml::to_string(&m).unwrap();
        assert_eq!(out, "schema_version: 1\n");
    }

    #[test]
    fn unknown_manifest_key_is_rejected() {
        let e = parse::<Manifest>("schema_version: 1\nbogus: 1\n").unwrap_err();
        assert!(e.to_string().contains("unknown field `bogus`"), "{e}");
    }

    #[test]
    fn native_accepts_free_yaml() {
        let m: Manifest =
            parse("schema_version: 1\nnative:\n  claude:\n    anything: [1, 2]\n").unwrap();
        assert!(m.native.contains_key("claude"));
    }

    #[test]
    fn duplicate_keys_are_rejected() {
        assert!(parse::<Manifest>("schema_version: 1\nschema_version: 1\n").is_err());
    }

    #[test]
    fn capability_value_outside_domain_is_rejected() {
        assert!(parse::<Capabilities>("read: maybe\n").is_err());
    }
}
