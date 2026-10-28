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

use super::json_keys::{JsonKeys, OwnedJson, conflict, merge_into, plain_values, remove_from};

/// One hook handler and the group it sits in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HookEntry {
    pub event: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matcher: Option<String>,
    pub command: String,
    /// `native.claude` of the automation, merged into the handler object.
    #[serde(
        default,
        skip_serializing_if = "BTreeMap::is_empty",
        serialize_with = "plain_values"
    )]
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
    /// Top-level keys from the manifest's `native.claude`, `permissions` excepted
    /// (see [`Owned::permissions`]).
    #[serde(
        default,
        skip_serializing_if = "BTreeMap::is_empty",
        serialize_with = "plain_values"
    )]
    pub keys: BTreeMap<String, Value>,
    /// Containers link created (`hooks`, `hooks.<Event>`): removed once empty again;
    /// a container the user had, even empty, is kept.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub containers: Vec<String>,
    /// Rules and values of `native.claude.permissions`, owned entry by entry. Absent
    /// from manifests written before 1.0.0, hence the default.
    #[serde(default, skip_serializing_if = "OwnedJson::is_empty")]
    pub permissions: OwnedJson,
}

/// What a merge did besides its text and what it owns: for the report.
#[derive(Debug, Default, PartialEq)]
pub struct MergeNotes {
    /// Identical hooks found and taken over (`--adopt-hooks`).
    pub adopted: Vec<HookEntry>,
    /// Identical hooks found and left to the user.
    pub kept: Vec<HookEntry>,
    /// `deny` rules link added that the `allow` of the user also lists.
    pub deny_over_allow: Vec<String>,
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
    if !project.flows.is_empty() {
        for e in flow_status_entries() {
            if !out.contains(&e) {
                out.push(e);
            }
        }
    }
    Ok(out)
}

/// The flow recording hooks (`armadai __record-flow-event`), asynchronous so they never
/// slow a subagent down. No matcher on `SubagentStart`/`SubagentStop`: an omitted
/// matcher fires on every agent type (hooks doc, code.claude.com/docs/en/hooks,
/// 2026-10-22); `SubagentHandback` is the tool a subagent hands its report back with
/// (tools reference, code.claude.com/docs/en/tools-reference, 2026-10-22).
/// `UserPromptExpansion` records the command a user types: it matches on `command_name`
/// and "leave the matcher empty to fire on every prompt-type command"
/// (code.claude.com/docs/en/hooks.md, `UserPromptExpansion`, 2026-10-26). The stdout of
/// a `UserPromptExpansion` hook enters the model's context (same page), and `async`
/// does not guarantee otherwise: the recorder itself writes no byte.
pub fn flow_status_entries() -> Vec<HookEntry> {
    [
        ("SubagentStart", None),
        ("SubagentStop", None),
        ("PreToolUse", Some("SubagentHandback")),
        ("UserPromptExpansion", None),
    ]
    .into_iter()
    .map(|(event, matcher)| HookEntry {
        event: event.into(),
        matcher: matcher.map(Into::into),
        command: "armadai __record-flow-event".into(),
        native: BTreeMap::from([("async".to_string(), Value::Bool(true))]),
    })
    .collect()
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
        doc.shift_remove("hooks");
    }
}

fn unmerge(doc: &mut Map<String, Value>, entries: &[&HookEntry], keys: &[(&String, &Value)]) {
    for e in entries {
        remove_one(doc, e);
    }
    for (k, v) in keys {
        if doc.get(*k) == Some(*v) {
            doc.shift_remove(*k);
        }
    }
}

/// `previous` as this version reads it: a `permissions` key owned whole (what 1.0.0-rc.8
/// recorded when it created the key) becomes owned rules, entry by entry.
fn read_previous(previous: &Owned) -> Owned {
    let mut p = previous.clone();
    if p.keys.get("permissions").is_some_and(Value::is_object)
        && let Some(Value::Object(m)) = p.keys.remove("permissions")
    {
        let o = &mut p.permissions;
        o.containers.push("permissions".into());
        for (k, v) in m {
            let path = format!("permissions.{k}");
            match v {
                Value::Array(a) => {
                    o.containers.push(path.clone());
                    o.array_entries.insert(path, a);
                }
                v => {
                    o.values.insert(path, v);
                }
            }
        }
        o.containers.sort();
        o.containers.dedup();
    }
    p
}

/// `native.claude.permissions`, when an object, as keys to merge one by one (a list
/// into the entries of `permissions.<key>`, anything else as a value); taken out of
/// `keys`. shortcut: a key of `permissions` containing `.` cannot be owned, none exists.
fn permission_keys(keys: &mut BTreeMap<String, Value>) -> JsonKeys {
    let mut out = JsonKeys::default();
    // not an object: stays in `keys`, a whole value like any other key
    if keys.get("permissions").is_some_and(Value::is_object)
        && let Some(Value::Object(m)) = keys.remove("permissions")
    {
        for (k, v) in m {
            let path = format!("permissions.{k}");
            match v {
                Value::Array(a) => {
                    out.array_entries.insert(path, a);
                }
                v => {
                    out.values.insert(path, v);
                }
            }
        }
    }
    out
}

/// Edit the document as the `Value` the shared merge works on.
fn on_value<T>(doc: &mut Map<String, Value>, f: impl FnOnce(&mut Value) -> T) -> T {
    let mut v = Value::Object(std::mem::take(doc));
    let out = f(&mut v);
    if let Value::Object(m) = v {
        *doc = m;
    }
    out
}

/// Merge `entries` and the native `keys` into `existing` (absent = new file). What
/// `previous` owned and is no longer wanted is removed by value; an identical
/// entry already present is neither duplicated nor claimed, unless `adopt` takes it
/// over (a clone without manifest). `permissions` is merged rule by rule. Returns
/// the new text, what this link owns and what the report must say.
pub fn merge_settings(
    existing: Option<&str>,
    entries: &[HookEntry],
    keys: &BTreeMap<String, Value>,
    previous: &Owned,
    adopt: bool,
) -> anyhow::Result<(String, Owned, MergeNotes)> {
    let mut doc = parse(existing)?;
    let previous = &read_previous(previous);
    let mut keys = keys.clone();
    let perms = permission_keys(&mut keys);
    let keys = &keys;
    let gone: Vec<&HookEntry> = (previous.entries.iter())
        .filter(|e| !entries.contains(e))
        .collect();
    let gone_keys: Vec<(&String, &Value)> = (previous.keys.iter())
        .filter(|(k, v)| keys.get(*k) != Some(*v))
        .collect();
    unmerge(&mut doc, &gone, &gone_keys);

    let mut notes = MergeNotes::default();
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
            Some(cur) => return Err(conflict(k, cur, v)),
            None => {
                doc.insert(k.clone(), v.clone());
                owned.keys.insert(k.clone(), v.clone());
            }
        }
    }
    owned.permissions = on_value(&mut doc, |d| merge_into(d, &perms, &previous.permissions))?;
    for e in entries {
        if present(&mut doc, e) {
            if previous.entries.contains(e) {
                owned.entries.push(e.clone());
            } else if adopt {
                owned.entries.push(e.clone());
                notes.adopted.push(e.clone());
            } else {
                notes.kept.push(e.clone());
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
    if !notes.adopted.is_empty() {
        // the earlier link that wrote these hooks left no record of the event arrays and
        // of `hooks` it created: those left empty once the adopted hooks are out were its
        let mut without = doc.clone();
        for e in &notes.adopted {
            remove_one(&mut without, e);
        }
        for e in &notes.adopted {
            if groups(&mut without, &e.event).is_some_and(|g| g.is_empty()) {
                owned.containers.push(format!("hooks.{}", e.event));
            }
        }
        let mut as_if = owned.containers.clone();
        as_if.push("hooks".into());
        prune(&mut without, &as_if);
        if !without.contains_key("hooks") {
            owned.containers.push("hooks".into());
        }
    }
    prune(&mut doc, &owned.containers);
    owned.containers.sort();
    owned.containers.dedup();
    if let Some(deny) = owned.permissions.array_entries.get("permissions.deny") {
        let allow = doc.get("permissions").and_then(|p| p.get("allow"));
        let mine = owned.permissions.array_entries.get("permissions.allow");
        notes.deny_over_allow = (deny.iter())
            .filter(|r| {
                allow
                    .and_then(Value::as_array)
                    .is_some_and(|a| a.contains(r))
                    && !mine.is_some_and(|m| m.contains(r))
            })
            .filter_map(|r| r.as_str().map(str::to_string))
            .collect();
    }
    Ok((render(doc), owned, notes))
}

/// Remove exactly what `previous` owns from `existing`.
pub fn remove_entries(existing: &str, previous: &Owned) -> anyhow::Result<String> {
    let mut doc = parse(Some(existing))?;
    let previous = &read_previous(previous);
    let entries: Vec<&HookEntry> = previous.entries.iter().collect();
    let keys: Vec<(&String, &Value)> = previous.keys.iter().collect();
    unmerge(&mut doc, &entries, &keys);
    on_value(&mut doc, |d| remove_from(d, &previous.permissions));
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

    /// `merge_settings` without adoption, as most tests use it.
    fn merge(
        existing: Option<&str>,
        entries: &[HookEntry],
        keys: &BTreeMap<String, Value>,
        previous: &Owned,
    ) -> anyhow::Result<(String, Owned)> {
        merge_settings(existing, entries, keys, previous, false).map(|(t, o, _)| (t, o))
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
            "off: no gate; no flow: no recording hook"
        );
    }

    fn recording(event: &str, matcher: Option<&str>) -> HookEntry {
        HookEntry {
            event: event.into(),
            matcher: matcher.map(Into::into),
            command: "armadai __record-flow-event".into(),
            native: BTreeMap::from([("async".to_string(), json!(true))]),
        }
    }

    #[test]
    fn a_project_linked_with_three_recording_hooks_gets_the_fourth_on_relink() {
        let old = flow_status_entries()[..3].to_vec();
        let (first, owned1) = merge(Some(USER), &old, &none(), &Owned::default()).unwrap();
        let (second, owned2) =
            merge(Some(&first), &flow_status_entries(), &none(), &owned1).unwrap();
        assert_eq!(
            value(&second)["hooks"]["UserPromptExpansion"],
            json!([{"hooks": [{"type": "command", "command": "armadai __record-flow-event", "async": true}]}])
        );
        assert_eq!(owned2.entries, flow_status_entries());
        assert_eq!(
            value(&remove_entries(&second, &owned2).unwrap()),
            value(USER)
        );
    }

    #[test]
    fn a_project_with_a_flow_records_subagents_after_the_gate() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".armadai/agents")).unwrap();
        std::fs::create_dir_all(dir.path().join(".armadai/flows")).unwrap();
        std::fs::write(
            dir.path().join(".armadai/project.yaml"),
            "schema_version: 1\npolicy:\n  delegation: strict\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join(".armadai/agents/dev.md"),
            "---\ndescription: d\n---\nB\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join(".armadai/flows/f.yaml"),
            "steps:\n  build: { agent: dev }\n",
        )
        .unwrap();
        let (p, _) = armadai_model::load_checked(dir.path()).unwrap();
        assert_eq!(
            hook_entries(&p).unwrap(),
            vec![
                gate_entry(),
                recording("SubagentStart", None),
                recording("SubagentStop", None),
                recording("PreToolUse", Some("SubagentHandback")),
                recording("UserPromptExpansion", None),
            ]
        );
        assert_eq!(
            recording("SubagentStop", None).handler(),
            json!({"type": "command", "command": "armadai __record-flow-event", "async": true})
        );
    }

    #[test]
    fn creates_a_settings_file_from_nothing() {
        let (text, owned) = merge(None, &wanted(), &none(), &Owned::default()).unwrap();
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
        let (text, _) = merge(Some(USER), &wanted(), &none(), &Owned::default()).unwrap();
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
        let (once, owned) = merge(Some(USER), &wanted(), &none(), &Owned::default()).unwrap();
        let (twice, owned2) = merge(Some(&once), &wanted(), &none(), &owned).unwrap();
        assert_eq!(once, twice);
        assert_eq!(owned, owned2);
    }

    #[test]
    fn removing_after_merging_gives_the_user_file_back() {
        let (merged, owned) = merge(Some(USER), &wanted(), &none(), &Owned::default()).unwrap();
        assert_eq!(
            value(&remove_entries(&merged, &owned).unwrap()),
            value(USER)
        );

        let (merged, owned) = merge(None, &wanted(), &none(), &Owned::default()).unwrap();
        assert_eq!(value(&remove_entries(&merged, &owned).unwrap()), json!({}));

        let empty_hooks = r#"{"hooks": {}}"#;
        let (merged, owned) =
            merge(Some(empty_hooks), &wanted(), &none(), &Owned::default()).unwrap();
        assert_eq!(
            value(&remove_entries(&merged, &owned).unwrap()),
            json!({"hooks": {}}),
            "a pre-existing empty `hooks` is the user's"
        );

        let empty_event = r#"{"hooks": {"PreToolUse": []}}"#;
        let (merged, owned) =
            merge(Some(empty_event), &wanted(), &none(), &Owned::default()).unwrap();
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
        let (text, owned, notes) = merge_settings(
            Some(user),
            std::slice::from_ref(&mine),
            &none(),
            &Owned::default(),
            false,
        )
        .unwrap();
        assert_eq!(value(&text), value(user));
        assert!(owned.entries.is_empty());
        assert_eq!(notes.kept, [mine]);
        assert_eq!(notes.adopted, []);
        assert_eq!(value(&remove_entries(&text, &owned).unwrap()), value(user));
    }

    #[test]
    fn an_identical_entry_is_adopted_on_request() {
        let mine = entry("PreToolUse", "armadai check");
        let theirs = r#"{"type": "command", "command": "armadai check --mine"}"#;
        let user = format!(
            r#"{{"hooks": {{"PreToolUse": [{{"hooks": [{{"type": "command", "command": "armadai check"}}, {theirs}]}}]}}}}"#
        );
        let (text, owned, notes) = merge_settings(
            Some(&user),
            std::slice::from_ref(&mine),
            &none(),
            &Owned::default(),
            true,
        )
        .unwrap();
        assert_eq!(text, pretty(&user));
        assert_eq!(owned.entries, std::slice::from_ref(&mine));
        assert_eq!(notes.adopted, [mine]);
        assert_eq!(notes.kept, []);
        assert_eq!(
            remove_entries(&text, &owned).unwrap(),
            pretty(&format!(
                r#"{{"hooks": {{"PreToolUse": [{{"hooks": [{theirs}]}}]}}}}"#
            )),
            "the hook of the user whose text differs stays"
        );
    }

    #[test]
    fn adopting_every_hook_of_a_file_lets_unlink_empty_it() {
        let wanted = [entry("PreToolUse", "a"), entry("Stop", "b")];
        let user = r#"{"hooks": {"PreToolUse": [{"hooks": [{"type": "command", "command": "a"}]}], "Stop": [{"hooks": [{"type": "command", "command": "b"}]}]}}"#;
        let (text, owned, _) =
            merge_settings(Some(user), &wanted, &none(), &Owned::default(), true).unwrap();
        assert_eq!(remove_entries(&text, &owned).unwrap(), pretty("{}"));
        // not adopted: the arrays and `hooks` are the user's as far as anyone knows
        let (text, owned, _) =
            merge_settings(Some(user), &wanted, &none(), &Owned::default(), false).unwrap();
        assert_eq!(remove_entries(&text, &owned).unwrap(), pretty(user));
    }

    #[test]
    fn a_hook_with_another_matcher_is_not_adopted() {
        let mine = entry("PreToolUse", "armadai check");
        let user = r#"{"hooks": {"PreToolUse": [{"matcher": "Bash", "hooks": [{"type": "command", "command": "armadai check"}]}]}}"#;
        let (text, owned, notes) = merge_settings(
            Some(user),
            std::slice::from_ref(&mine),
            &none(),
            &Owned::default(),
            true,
        )
        .unwrap();
        assert_eq!(
            text,
            pretty(
                r#"{"hooks": {"PreToolUse": [{"matcher": "Bash", "hooks": [{"type": "command", "command": "armadai check"}]}, {"hooks": [{"type": "command", "command": "armadai check"}]}]}}"#
            ),
            "added beside the user's, not taken for it"
        );
        assert_eq!((notes.adopted, notes.kept), (vec![], vec![]));
        assert_eq!(remove_entries(&text, &owned).unwrap(), pretty(user));
    }

    #[test]
    fn an_entry_no_longer_wanted_is_removed_on_relink() {
        let (first, owned) = merge(Some(USER), &wanted(), &none(), &Owned::default()).unwrap();
        let (second, owned2) = merge(Some(&first), &wanted()[..1], &none(), &owned).unwrap();
        assert!(
            value(&second)["hooks"].get("SessionStart").is_none(),
            "{second}"
        );
        assert_eq!(owned2.entries, wanted()[..1]);
    }

    #[test]
    fn native_keys_are_added_owned_and_removed() {
        let keys: BTreeMap<String, Value> = [("model".to_string(), json!("opus"))].into();
        let (text, owned) = merge(Some(USER), &[], &keys, &Owned::default()).unwrap();
        assert_eq!(value(&text)["model"], json!("opus"));
        assert_eq!(value(&remove_entries(&text, &owned).unwrap()), value(USER));
        let clash = r#"{"model": "haiku"}"#;
        let err = merge(Some(clash), &[], &keys, &Owned::default()).unwrap_err();
        assert_eq!(
            err.to_string(),
            r#"key `model` is already set to another value: "haiku" (link writes "opus")"#
        );
    }

    const ORDERED: &str = r#"{"zeta":1,"permissions":{"allow":["Bash(ls)"]},"alpha":{"x":true}}"#;

    fn ordered_with(extra: &[(&str, i32)]) -> String {
        let mut v = value(ORDERED);
        for (k, n) in extra {
            v[*k] = json!(n);
        }
        render(v.as_object().unwrap().clone())
    }

    #[test]
    fn unlink_keeps_the_key_order_of_the_user() {
        let keys: BTreeMap<String, Value> = [("model".to_string(), json!("opus"))].into();
        let (merged, owned) = merge(Some(ORDERED), &wanted(), &keys, &Owned::default()).unwrap();
        // the user adds two keys after the link
        let mut v = value(&merged);
        v["beta"] = json!(2);
        v["gamma"] = json!(3);
        let edited = render(v.as_object().unwrap().clone());
        assert_eq!(
            remove_entries(&edited, &owned).unwrap(),
            ordered_with(&[("beta", 2), ("gamma", 3)])
        );
    }

    #[test]
    fn a_key_no_longer_wanted_leaves_the_order_on_relink() {
        let keys: BTreeMap<String, Value> = [
            ("model".to_string(), json!("opus")),
            ("env".to_string(), json!({"A": "1"})),
        ]
        .into();
        let (merged, owned) = merge(Some(ORDERED), &[], &keys, &Owned::default()).unwrap();
        let mut v = value(&merged);
        v["beta"] = json!(2);
        v["gamma"] = json!(3);
        let edited = render(v.as_object().unwrap().clone());
        let only_model: BTreeMap<String, Value> = [("model".to_string(), json!("opus"))].into();
        let (relinked, _) = merge(Some(&edited), &[], &only_model, &owned).unwrap();
        let mut expected = value(ORDERED);
        expected["model"] = json!("opus");
        expected["beta"] = json!(2);
        expected["gamma"] = json!(3);
        assert_eq!(relinked, render(expected.as_object().unwrap().clone()));
    }

    fn pretty(t: &str) -> String {
        render(value(t).as_object().unwrap().clone())
    }

    fn permissions(v: Value) -> BTreeMap<String, Value> {
        [("permissions".to_string(), v)].into()
    }

    #[test]
    fn permission_lists_merge_rule_by_rule() {
        let user = r#"{"permissions":{"allow":["Bash(ls)"]}}"#;
        let keys = permissions(json!({"ask": ["Bash(git push:*)"]}));
        let (text, owned) = merge(Some(user), &[], &keys, &Owned::default()).unwrap();
        assert_eq!(
            text,
            pretty(r#"{"permissions":{"allow":["Bash(ls)"],"ask":["Bash(git push:*)"]}}"#)
        );
        let (again, owned2) = merge(Some(&text), &[], &keys, &owned).unwrap();
        assert_eq!((again, &owned2), (text.clone(), &owned), "second link");
        assert_eq!(remove_entries(&text, &owned).unwrap(), pretty(user));
    }

    #[test]
    fn a_project_allow_goes_after_the_user_allow() {
        let user = r#"{"permissions":{"allow":["Bash(ls)"]}}"#;
        let keys = permissions(json!({"allow": ["Bash(git *)"]}));
        let (text, owned) = merge(Some(user), &[], &keys, &Owned::default()).unwrap();
        assert_eq!(
            text,
            pretty(r#"{"permissions":{"allow":["Bash(ls)","Bash(git *)"]}}"#)
        );
        assert_eq!(remove_entries(&text, &owned).unwrap(), pretty(user));
    }

    #[test]
    fn additional_directories_merge_like_the_rules() {
        let user = r#"{"permissions":{"additionalDirectories":["../u"]}}"#;
        let keys = permissions(json!({"additionalDirectories": ["../p"]}));
        let (text, owned) = merge(Some(user), &[], &keys, &Owned::default()).unwrap();
        assert_eq!(
            text,
            pretty(r#"{"permissions":{"additionalDirectories":["../u","../p"]}}"#)
        );
        assert_eq!(remove_entries(&text, &owned).unwrap(), pretty(user));
    }

    #[test]
    fn a_rule_the_user_already_has_is_neither_duplicated_nor_claimed() {
        let user = r#"{"permissions":{"ask":["Bash(git push:*)"]}}"#;
        let keys = permissions(json!({"ask": ["Bash(git push:*)"]}));
        let (text, owned) = merge(Some(user), &[], &keys, &Owned::default()).unwrap();
        assert_eq!(text, pretty(user));
        assert_eq!(owned.permissions, OwnedJson::default());
        assert_eq!(remove_entries(&text, &owned).unwrap(), pretty(user));
    }

    #[test]
    fn a_deny_the_user_allows_is_written_and_noted() {
        let keys = permissions(json!({"deny": ["Bash(curl:*)", "Bash(rm:*)"]}));
        let user = r#"{"permissions":{"allow":["Bash(curl:*)","Bash(ls)"]}}"#;
        let (text, _, notes) =
            merge_settings(Some(user), &[], &keys, &Owned::default(), false).unwrap();
        assert_eq!(
            text,
            pretty(
                r#"{"permissions":{"allow":["Bash(curl:*)","Bash(ls)"],"deny":["Bash(curl:*)","Bash(rm:*)"]}}"#
            )
        );
        assert_eq!(notes.deny_over_allow, ["Bash(curl:*)"]);
        let user = r#"{"permissions":{"allow":["Bash(ls)"]}}"#;
        let (_, _, notes) =
            merge_settings(Some(user), &[], &keys, &Owned::default(), false).unwrap();
        assert_eq!(notes.deny_over_allow, Vec::<String>::new());
    }

    #[test]
    fn a_deny_against_an_allow_link_added_itself_is_not_noted() {
        let keys = permissions(json!({"allow": ["Bash(curl:*)"], "deny": ["Bash(curl:*)"]}));
        let (_, _, notes) = merge_settings(None, &[], &keys, &Owned::default(), false).unwrap();
        assert_eq!(notes.deny_over_allow, Vec::<String>::new());
    }

    #[test]
    fn a_non_list_permission_with_another_value_names_both_values() {
        let user = r#"{"permissions":{"defaultMode":"plan"}}"#;
        let keys = permissions(json!({"defaultMode": "acceptEdits"}));
        let err = merge(Some(user), &[], &keys, &Owned::default()).unwrap_err();
        assert_eq!(
            err.to_string(),
            r#"key `permissions.defaultMode` is already set to another value: "plan" (link writes "acceptEdits")"#
        );
    }

    /// rc.8 owned the whole `permissions` key when it created it: that reads as owned
    /// rules, a relink changes nothing and the old manifest still unlinks.
    #[test]
    fn a_manifest_owning_the_whole_permissions_key_reads_as_rules() {
        let previous = Owned {
            keys: permissions(json!({"ask": ["Bash(git push:*)"]})),
            ..Owned::default()
        };
        let file = r#"{"permissions":{"ask":["Bash(git push:*)"]},"zeta":1}"#;
        let (text, owned) = merge(
            Some(file),
            &[],
            &permissions(json!({"ask": ["Bash(git push:*)"]})),
            &previous,
        )
        .unwrap();
        assert_eq!(text, pretty(file));
        assert_eq!(
            owned,
            Owned {
                permissions: OwnedJson {
                    array_entries: [(
                        "permissions.ask".to_string(),
                        vec![json!("Bash(git push:*)")]
                    )]
                    .into(),
                    containers: vec!["permissions".into(), "permissions.ask".into()],
                    ..OwnedJson::default()
                },
                ..Owned::default()
            }
        );
        assert_eq!(
            remove_entries(file, &previous).unwrap(),
            pretty(r#"{"zeta":1}"#)
        );
    }

    /// The manifest stores a number as a plain YAML number (not serde_json's private
    /// `arbitrary_precision` map, which is what it wrote until 1.0.0-rc.8), and reads
    /// both forms.
    #[test]
    fn owned_numbers_are_plain_yaml_numbers_in_the_manifest() {
        let mut e = entry("PreToolUse", "./x.sh");
        e.native.insert("timeout".into(), json!(5));
        let owned = Owned {
            entries: vec![e],
            keys: [("cleanupPeriodDays".to_string(), json!(30))].into(),
            ..Owned::default()
        };
        let plain = "entries:\n- event: PreToolUse\n  command: ./x.sh\n  native:\n    timeout: 5\nkeys:\n  cleanupPeriodDays: 30\n";
        assert_eq!(armadai_model::yaml::to_string(&owned).unwrap(), plain);
        assert_eq!(
            armadai_model::yaml::from_str::<Owned>(plain).unwrap(),
            owned
        );
        let private = "entries:\n- event: PreToolUse\n  command: ./x.sh\n  native:\n    timeout:\n      \"$serde_json::private::Number\": \"5\"\nkeys:\n  cleanupPeriodDays:\n    \"$serde_json::private::Number\": \"30\"\n";
        assert_eq!(
            armadai_model::yaml::from_str::<Owned>(private).unwrap(),
            owned
        );
    }

    #[test]
    fn invalid_json_is_an_error() {
        assert!(merge(Some("{ not json"), &wanted(), &none(), &Owned::default()).is_err());
        assert!(merge(Some("[1]"), &wanted(), &none(), &Owned::default()).is_err());
        assert!(remove_entries("{ not json", &Owned::default()).is_err());
    }
}
