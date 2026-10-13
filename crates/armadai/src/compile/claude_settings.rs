//! Automations → Claude Code hooks, merged into `.claude/settings.json` without
//! clobbering what the user wrote there.
//!
//! No marker is written into the hook objects (an unknown field is not documented by
//! Claude Code): ownership is the exact values recorded in the link manifest
//! ([`Owned`]), and removal is by value equality.

use std::collections::BTreeMap;

use anyhow::{Context, bail};
use armadai_model::{Delegation, Event, NativeValue, Project};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// One hook handler and the group it sits in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HookEntry {
    pub event: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matcher: Option<String>,
    pub command: String,
    /// `native.claude` of the automation, merged into the handler object.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub native: BTreeMap<String, Value>,
}

impl HookEntry {
    fn handler(&self) -> Value {
        let mut h = Map::new();
        h.insert("type".into(), "command".into());
        h.insert("command".into(), self.command.clone().into());
        h.extend(self.native.clone());
        Value::Object(h)
    }
}

/// What a link added to a settings file, as exact values.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Owned {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<HookEntry>,
    /// Top-level keys from the manifest's `native.claude`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub keys: BTreeMap<String, Value>,
    /// Containers link created (`hooks`, `hooks.<Event>`): removed once empty again;
    /// a container the user had, even empty, is kept.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub containers: Vec<String>,
}

pub fn event_name(e: Event) -> &'static str {
    match e {
        Event::SessionStart => "SessionStart",
        Event::SessionEnd => "SessionEnd",
        Event::BeforeTool => "PreToolUse",
        Event::AfterTool => "PostToolUse",
        Event::OnPrompt => "UserPromptSubmit",
        Event::OnStop => "Stop",
    }
}

fn native_map(v: Option<&NativeValue>, what: &str) -> anyhow::Result<Map<String, Value>> {
    match v.map(|v| &v.0) {
        None => Ok(Map::new()),
        Some(Value::Object(m)) => Ok(m.clone()),
        Some(_) => bail!("{what}: native.claude must be a mapping"),
    }
}

/// One entry per automation, sorted by automation id, then the delegation gate
/// when `policy.delegation` is `strict`. `native.claude.matcher` becomes the
/// group matcher; any other native key goes into the handler (`type`/`command` = error).
pub fn hook_entries(project: &Project) -> anyhow::Result<Vec<HookEntry>> {
    let mut out = Vec::new();
    for (id, a) in &project.automations {
        let what = format!("automation `{id}`");
        let mut native = native_map(a.native.get("claude"), &what)?;
        let matcher = match native.remove("matcher") {
            None => None,
            Some(Value::String(s)) => Some(s),
            Some(_) => bail!("{what}: native.claude.matcher must be a string"),
        };
        if let Some(k) = ["type", "command"]
            .into_iter()
            .find(|k| native.contains_key(*k))
        {
            bail!("{what}: native.claude key `{k}` collides with a generated key");
        }
        let entry = HookEntry {
            event: event_name(a.event).into(),
            matcher,
            command: a.run.clone(),
            native: native.into_iter().collect(),
        };
        // two identical automations are one hook: owning it twice would let unlink
        // remove a user's identical handler as the second copy
        if !out.contains(&entry) {
            out.push(entry);
        }
    }
    if project.manifest.policy.delegation == Delegation::Strict {
        out.push(gate_entry());
    }
    Ok(out)
}

/// The delegation gate: `Agent` and its former name `Task` (an exact-name list
/// for Claude Code's matcher, not a regex).
pub fn gate_entry() -> HookEntry {
    HookEntry {
        event: "PreToolUse".into(),
        matcher: Some("Agent|Task".into()),
        command: "armadai __claude-policy-gate".into(),
        native: BTreeMap::new(),
    }
}

/// Top-level settings keys from the manifest's `native.claude` (`hooks` is generated).
pub fn native_settings(project: &Project) -> anyhow::Result<BTreeMap<String, Value>> {
    let m = native_map(project.manifest.native.get("claude"), "project.yaml")?;
    if m.contains_key("hooks") {
        bail!("project.yaml: native.claude key `hooks` collides with the generated hooks");
    }
    Ok(m.into_iter().collect())
}

fn parse(existing: Option<&str>) -> anyhow::Result<Map<String, Value>> {
    match existing {
        None => Ok(Map::new()),
        Some(t) => match serde_json::from_str(t).context("settings.json is not valid JSON")? {
            Value::Object(m) => Ok(m),
            _ => bail!("settings.json is not a JSON object"),
        },
    }
}

fn render(doc: Map<String, Value>) -> String {
    let mut s = serde_json::to_string_pretty(&Value::Object(doc)).expect("JSON value serializes");
    s.push('\n');
    s
}

fn groups<'a>(doc: &'a mut Map<String, Value>, event: &str) -> Option<&'a mut Vec<Value>> {
    doc.get_mut("hooks")?
        .as_object_mut()?
        .get_mut(event)?
        .as_array_mut()
}

fn same_matcher(group: &Value, e: &HookEntry) -> bool {
    group.get("matcher").and_then(Value::as_str) == e.matcher.as_deref()
}

fn present(doc: &mut Map<String, Value>, e: &HookEntry) -> bool {
    let h = e.handler();
    groups(doc, &e.event).is_some_and(|gs| {
        gs.iter().any(|g| {
            same_matcher(g, e)
                && g.get("hooks")
                    .and_then(Value::as_array)
                    .is_some_and(|hs| hs.contains(&h))
        })
    })
}

/// Remove the first handler equal to `e`; drop its group when that empties it.
fn remove_one(doc: &mut Map<String, Value>, e: &HookEntry) {
    let h = e.handler();
    let Some(gs) = groups(doc, &e.event) else {
        return;
    };
    for i in 0..gs.len() {
        if !same_matcher(&gs[i], e) {
            continue;
        }
        let Some(hs) = gs[i].get_mut("hooks").and_then(Value::as_array_mut) else {
            continue;
        };
        if let Some(j) = hs.iter().position(|x| *x == h) {
            hs.remove(j);
            if hs.is_empty() {
                gs.remove(i);
            }
            return;
        }
    }
}

/// Drop the containers link created once they are empty.
fn prune(doc: &mut Map<String, Value>, containers: &[String]) {
    if let Some(hooks) = doc.get_mut("hooks").and_then(Value::as_object_mut) {
        hooks.retain(|ev, v| {
            !(containers.contains(&format!("hooks.{ev}"))
                && v.as_array().is_some_and(Vec::is_empty))
        });
    }
    let empty = doc
        .get("hooks")
        .and_then(Value::as_object)
        .is_some_and(Map::is_empty);
    if empty && containers.iter().any(|c| c == "hooks") {
        doc.remove("hooks");
    }
}

fn unmerge(doc: &mut Map<String, Value>, entries: &[&HookEntry], keys: &[(&String, &Value)]) {
    for e in entries {
        remove_one(doc, e);
    }
    for (k, v) in keys {
        if doc.get(*k) == Some(*v) {
            doc.remove(*k);
        }
    }
}

/// Merge `entries` and the native `keys` into `existing` (absent = new file). What
/// `previous` owned and is no longer wanted is removed by value; an identical
/// entry already present is neither duplicated nor claimed. Returns the new text
/// and what this link owns.
pub fn merge_settings(
    existing: Option<&str>,
    entries: &[HookEntry],
    keys: &BTreeMap<String, Value>,
    previous: &Owned,
) -> anyhow::Result<(String, Owned)> {
    let mut doc = parse(existing)?;
    let gone: Vec<&HookEntry> = (previous.entries.iter())
        .filter(|e| !entries.contains(e))
        .collect();
    let gone_keys: Vec<(&String, &Value)> = (previous.keys.iter())
        .filter(|(k, v)| keys.get(*k) != Some(*v))
        .collect();
    unmerge(&mut doc, &gone, &gone_keys);

    let mut owned = Owned {
        containers: previous.containers.clone(),
        ..Owned::default()
    };
    for (k, v) in keys {
        match doc.get(k) {
            Some(cur) if cur == v => {
                if previous.keys.get(k) == Some(v) {
                    owned.keys.insert(k.clone(), v.clone());
                }
            }
            Some(_) => bail!("settings.json key `{k}` is already set to another value"),
            None => {
                doc.insert(k.clone(), v.clone());
                owned.keys.insert(k.clone(), v.clone());
            }
        }
    }
    for e in entries {
        if present(&mut doc, e) {
            if previous.entries.contains(e) {
                owned.entries.push(e.clone());
            }
            continue;
        }
        if !doc.contains_key("hooks") {
            doc.insert("hooks".into(), Value::Object(Map::new()));
            owned.containers.push("hooks".into());
        }
        let hooks = doc["hooks"]
            .as_object_mut()
            .context("settings.json `hooks` is not an object")?;
        if !hooks.contains_key(&e.event) {
            hooks.insert(e.event.clone(), Value::Array(Vec::new()));
            owned.containers.push(format!("hooks.{}", e.event));
        }
        let mut group = Map::new();
        if let Some(m) = &e.matcher {
            group.insert("matcher".into(), m.clone().into());
        }
        group.insert("hooks".into(), Value::Array(vec![e.handler()]));
        hooks[&e.event]
            .as_array_mut()
            .with_context(|| format!("settings.json `hooks.{}` is not an array", e.event))?
            .push(Value::Object(group));
        owned.entries.push(e.clone());
    }
    prune(&mut doc, &owned.containers);
    owned.containers.sort();
    owned.containers.dedup();
    Ok((render(doc), owned))
}

/// Remove exactly what `previous` owns from `existing`.
pub fn remove_entries(existing: &str, previous: &Owned) -> anyhow::Result<String> {
    let mut doc = parse(Some(existing))?;
    let entries: Vec<&HookEntry> = previous.entries.iter().collect();
    let keys: Vec<(&String, &Value)> = previous.keys.iter().collect();
    unmerge(&mut doc, &entries, &keys);
    prune(&mut doc, &previous.containers);
    Ok(render(doc))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn entry(event: &str, command: &str) -> HookEntry {
        HookEntry {
            event: event.into(),
            matcher: None,
            command: command.into(),
            native: BTreeMap::new(),
        }
    }

    fn value(t: &str) -> Value {
        serde_json::from_str(t).unwrap()
    }

    const USER: &str = r#"{
  "zeta": 1,
  "permissions": { "allow": ["Bash(git *)"] },
  "hooks": {
    "PreToolUse": [
      { "matcher": "Bash", "hooks": [ { "type": "command", "command": "./mine.sh" } ] }
    ]
  },
  "alpha": { "unknown": true }
}"#;

    fn wanted() -> Vec<HookEntry> {
        vec![
            entry("PreToolUse", "armadai check"),
            entry("SessionStart", "echo hi"),
        ]
    }

    fn none() -> BTreeMap<String, Value> {
        BTreeMap::new()
    }

    #[test]
    fn automations_map_to_the_six_documented_events() {
        let mut files = vec![(
            ".armadai/project.yaml".to_string(),
            "schema_version: 1\n".to_string(),
        )];
        for ev in [
            "session-start",
            "session-end",
            "before-tool",
            "after-tool",
            "on-prompt",
            "on-stop",
        ] {
            files.push((
                format!(".armadai/automations/{ev}.yaml"),
                format!("event: {ev}\nrun: ./{ev}.sh\n"),
            ));
        }
        let dir = tempfile::tempdir().unwrap();
        for (p, c) in &files {
            let p = dir.path().join(p);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, c).unwrap();
        }
        let (p, _) = armadai_model::load_checked(dir.path()).unwrap();
        let got: Vec<(String, String)> = hook_entries(&p)
            .unwrap()
            .into_iter()
            .map(|e| {
                assert_eq!(e.matcher, None);
                (e.event, e.command)
            })
            .collect();
        let pairs = |a: &str, b: &str| (a.to_string(), b.to_string());
        assert_eq!(
            got,
            vec![
                pairs("PostToolUse", "./after-tool.sh"),
                pairs("PreToolUse", "./before-tool.sh"),
                pairs("UserPromptSubmit", "./on-prompt.sh"),
                pairs("Stop", "./on-stop.sh"),
                pairs("SessionEnd", "./session-end.sh"),
                pairs("SessionStart", "./session-start.sh"),
            ]
        );
    }

    fn one_automation(yaml: &str) -> anyhow::Result<Vec<HookEntry>> {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".armadai/automations")).unwrap();
        std::fs::write(
            dir.path().join(".armadai/project.yaml"),
            "schema_version: 1\n",
        )
        .unwrap();
        std::fs::write(dir.path().join(".armadai/automations/a.yaml"), yaml).unwrap();
        let (p, _) = armadai_model::load_checked(dir.path()).unwrap();
        hook_entries(&p)
    }

    #[test]
    fn identical_automations_give_one_hook() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".armadai/automations")).unwrap();
        std::fs::write(
            dir.path().join(".armadai/project.yaml"),
            "schema_version: 1\n",
        )
        .unwrap();
        for id in ["a", "b"] {
            std::fs::write(
                dir.path().join(format!(".armadai/automations/{id}.yaml")),
                "event: on-stop\nrun: cargo fmt\n",
            )
            .unwrap();
        }
        let (p, _) = armadai_model::load_checked(dir.path()).unwrap();
        assert_eq!(hook_entries(&p).unwrap().len(), 1);
    }

    #[test]
    fn native_claude_goes_into_the_handler_and_matcher_into_the_group() {
        let es = one_automation(
            "event: before-tool\nrun: ./x.sh\nnative:\n  claude:\n    matcher: Bash\n    timeout: 30\n",
        )
        .unwrap();
        assert_eq!(es[0].matcher.as_deref(), Some("Bash"));
        assert_eq!(
            es[0].handler(),
            json!({"type": "command", "command": "./x.sh", "timeout": 30})
        );
        let err = one_automation(
            "event: before-tool\nrun: ./x.sh\nnative:\n  claude:\n    command: other\n",
        )
        .unwrap_err();
        assert!(err.to_string().contains("`command`"), "{err}");
    }

    fn manifest_only(manifest: &str) -> Vec<HookEntry> {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".armadai")).unwrap();
        std::fs::write(dir.path().join(".armadai/project.yaml"), manifest).unwrap();
        let (p, _) = armadai_model::load_checked(dir.path()).unwrap();
        hook_entries(&p).unwrap()
    }

    #[test]
    fn strict_delegation_installs_the_gate_on_agent_and_task() {
        let es = manifest_only("schema_version: 1\npolicy:\n  delegation: strict\n");
        assert_eq!(
            es,
            vec![HookEntry {
                event: "PreToolUse".into(),
                matcher: Some("Agent|Task".into()),
                command: "armadai __claude-policy-gate".into(),
                native: BTreeMap::new(),
            }]
        );
        assert!(
            manifest_only("schema_version: 1\n").is_empty(),
            "off: no gate"
        );
    }

    #[test]
    fn creates_a_settings_file_from_nothing() {
        let (text, owned) = merge_settings(None, &wanted(), &none(), &Owned::default()).unwrap();
        assert_eq!(
            value(&text),
            json!({"hooks": {
                "PreToolUse": [{"hooks": [{"type": "command", "command": "armadai check"}]}],
                "SessionStart": [{"hooks": [{"type": "command", "command": "echo hi"}]}]
            }})
        );
        assert_eq!(owned.entries, wanted());
        assert!(text.ends_with("}\n"));
    }

    #[test]
    fn user_hooks_permissions_unknown_keys_and_key_order_are_preserved() {
        let (text, _) = merge_settings(Some(USER), &wanted(), &none(), &Owned::default()).unwrap();
        let v = value(&text);
        assert_eq!(v["permissions"], value(USER)["permissions"]);
        assert_eq!(v["alpha"], json!({"unknown": true}));
        assert_eq!(
            v["hooks"]["PreToolUse"][0],
            value(USER)["hooks"]["PreToolUse"][0]
        );
        let keys: Vec<&String> = v.as_object().unwrap().keys().collect();
        assert_eq!(
            keys,
            ["zeta", "permissions", "hooks", "alpha"],
            "user key order kept"
        );
    }

    #[test]
    fn merging_twice_changes_nothing() {
        let (once, owned) =
            merge_settings(Some(USER), &wanted(), &none(), &Owned::default()).unwrap();
        let (twice, owned2) = merge_settings(Some(&once), &wanted(), &none(), &owned).unwrap();
        assert_eq!(once, twice);
        assert_eq!(owned, owned2);
    }

    #[test]
    fn removing_after_merging_gives_the_user_file_back() {
        let (merged, owned) =
            merge_settings(Some(USER), &wanted(), &none(), &Owned::default()).unwrap();
        assert_eq!(
            value(&remove_entries(&merged, &owned).unwrap()),
            value(USER)
        );

        let (merged, owned) = merge_settings(None, &wanted(), &none(), &Owned::default()).unwrap();
        assert_eq!(value(&remove_entries(&merged, &owned).unwrap()), json!({}));

        let empty_hooks = r#"{"hooks": {}}"#;
        let (merged, owned) =
            merge_settings(Some(empty_hooks), &wanted(), &none(), &Owned::default()).unwrap();
        assert_eq!(
            value(&remove_entries(&merged, &owned).unwrap()),
            json!({"hooks": {}}),
            "a pre-existing empty `hooks` is the user's"
        );

        let empty_event = r#"{"hooks": {"PreToolUse": []}}"#;
        let (merged, owned) =
            merge_settings(Some(empty_event), &wanted(), &none(), &Owned::default()).unwrap();
        assert_eq!(
            value(&remove_entries(&merged, &owned).unwrap()),
            value(empty_event),
            "a pre-existing empty event array is the user's"
        );
    }

    #[test]
    fn an_identical_entry_already_present_is_neither_duplicated_nor_claimed() {
        let mine = entry("PreToolUse", "armadai check");
        let user = r#"{"hooks": {"PreToolUse": [{"hooks": [{"type": "command", "command": "armadai check"}]}]}}"#;
        let (text, owned) = merge_settings(
            Some(user),
            std::slice::from_ref(&mine),
            &none(),
            &Owned::default(),
        )
        .unwrap();
        assert_eq!(value(&text), value(user));
        assert!(owned.entries.is_empty());
        assert_eq!(value(&remove_entries(&text, &owned).unwrap()), value(user));
    }

    #[test]
    fn an_entry_no_longer_wanted_is_removed_on_relink() {
        let (first, owned) =
            merge_settings(Some(USER), &wanted(), &none(), &Owned::default()).unwrap();
        let (second, owned2) =
            merge_settings(Some(&first), &wanted()[..1], &none(), &owned).unwrap();
        assert!(
            value(&second)["hooks"].get("SessionStart").is_none(),
            "{second}"
        );
        assert_eq!(owned2.entries, wanted()[..1]);
    }

    #[test]
    fn native_keys_are_added_owned_and_removed() {
        let keys: BTreeMap<String, Value> = [("model".to_string(), json!("opus"))].into();
        let (text, owned) = merge_settings(Some(USER), &[], &keys, &Owned::default()).unwrap();
        assert_eq!(value(&text)["model"], json!("opus"));
        assert_eq!(value(&remove_entries(&text, &owned).unwrap()), value(USER));
        let clash = r#"{"model": "haiku"}"#;
        let err = merge_settings(Some(clash), &[], &keys, &Owned::default()).unwrap_err();
        assert!(err.to_string().contains("`model`"), "{err}");
    }

    #[test]
    fn invalid_json_is_an_error() {
        assert!(merge_settings(Some("{ not json"), &wanted(), &none(), &Owned::default()).is_err());
        assert!(merge_settings(Some("[1]"), &wanted(), &none(), &Owned::default()).is_err());
        assert!(remove_entries("{ not json", &Owned::default()).is_err());
    }
}
