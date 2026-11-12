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
    /// The nested keys of `ui.subagent_status_line` (`enabledPlugins.<id>`,
    /// `extraKnownMarketplaces.<id>`), owned entry by entry. Absent from manifests
    /// written before it existed, hence the default.
    #[serde(default, skip_serializing_if = "OwnedJson::is_empty")]
    pub plugin: OwnedJson,
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
    for e in grant_entries(project) {
        if !out.contains(&e) {
            out.push(e);
        }
    }
    Ok(out)
}

/// The two `deny` rules that keep an agent from switching a level on itself (D15, D16:
/// a deny rule covers the call Claude usually makes, it is not a security boundary).
pub const GRANT_GUARD_RULES: [&str; 2] =
    ["Bash(armadai grant *)", "Edit(/.armadai/state/grants.yaml)"];

/// One `PermissionRequest` entry per (level, `Bash` rule), levels and rules in order: the
/// matcher is the rule's tool name, `if` the rule itself (best effort, hooks doc D2-D4),
/// the command `armadai __claude-grant <level>`. None without `policy.grants`.
pub fn grant_entries(project: &Project) -> Vec<HookEntry> {
    let mut out = Vec::new();
    for (level, grant) in &project.manifest.policy.grants {
        // a rule that is not a `Bash` rule is never answered (`grant-unsupported-rule`)
        for rule in grant
            .allow
            .iter()
            .filter(|r| crate::claude_adapter::grant_hook::is_bash_rule(r))
        {
            out.push(HookEntry {
                event: "PermissionRequest".into(),
                matcher: Some(crate::claude_adapter::grant_hook::rule_tool(rule).to_string()),
                command: format!("armadai __claude-grant {level}"),
                native: BTreeMap::from([("if".to_string(), Value::String(rule.clone()))]),
            });
        }
    }
    out
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
        // the marker of a delegation, written only when the gate lets it through
        ("PreToolUse", Some("Agent|Task")),
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
pub fn native_keys(project: &Project) -> anyhow::Result<BTreeMap<String, Value>> {
    let m = native_map(project.manifest.native.get("claude"), "project.yaml")?;
    if m.contains_key("hooks") {
        bail!("project.yaml: native.claude key `hooks` collides with the generated hooks");
    }
    Ok(m.into_iter().collect())
}

/// What Claude Code runs for `ui.status_line`. Format read on
/// code.claude.com/docs/en/statusline (2026-11-05): `type: command`, `command`, optional
/// `padding` (not written) and `refreshInterval` in seconds, minimum 1.
pub const STATUS_LINE_COMMAND: &str = "armadai __claude-statusline";

/// `{"type":"command","command":STATUS_LINE_COMMAND,"refreshInterval":5}`
pub fn status_line_value() -> Value {
    serde_json::json!({"type": "command", "command": STATUS_LINE_COMMAND, "refreshInterval": 5})
}

/// The plugin link writes for `ui.subagent_status_line`: its name, which is also the
/// name of its marketplace (`enabledPlugins` keys are `<plugin>@<marketplace>`).
pub const PLUGIN_NAME: &str = "armadai-ui";

/// The settings key (dotted path) holding the plugin in `enabledPlugins`.
pub const PLUGIN_ENABLED_KEY: &str = "enabledPlugins.armadai-ui@armadai-ui";
/// The settings key (dotted path) registering the plugin's marketplace.
pub const PLUGIN_MARKETPLACE_KEY: &str = "extraKnownMarketplaces.armadai-ui";

/// The files of the plugin, relative to the project root, with their content.
///
/// Formats read 2026-11-05 on code.claude.com/docs/en/plugins/manifest-reference
/// (`plugin.json`: `name` is the only required key; `settings.json` at the plugin root:
/// only `agent` and `subagentStatusLine` take effect), plugins/marketplace-reference
/// (`marketplace.json`: `name`, `owner.name`, `plugins[]` with `name` and `source`; a
/// relative `source` is a path from the marketplace root, `"."` being the root itself)
/// and statusline (`subagentStatusLine`: `type: command`, `command`).
///
/// `settings.json` comes with `ui.subagent_status_line`, the mod (`hooks/hooks.json`,
/// `hooks/register.js`, `panel.json`) with `ui.panel` or `ui.flow_band`; the two manifests
/// are always there.
/// The mod's layout was read 2026-11-09 on code.claude.com/docs/en/plugins/mods/overview
/// and mods/reference ("Files": `hooks/hooks.json` with `modules: ["./register.js"]`,
/// an ES module, `.js` accepted).
pub fn plugin_files(ui: &armadai_model::types::Ui) -> Vec<(std::path::PathBuf, String)> {
    let desc = "Flow progress from ArmadAI, in Claude Code";
    let mut files = vec![
        (
            ".claude-plugin/marketplace.json",
            serde_json::json!({
                "name": PLUGIN_NAME,
                "owner": {"name": "ArmadAI"},
                "plugins": [{"name": PLUGIN_NAME, "source": ".", "description": desc}],
            }),
        ),
        (
            ".claude-plugin/plugin.json",
            serde_json::json!({"name": PLUGIN_NAME, "description": desc}),
        ),
    ];
    if ui.subagent_status_line {
        files.push((
            "settings.json",
            serde_json::json!({"subagentStatusLine": {
                "type": "command",
                "command": SUBAGENT_STATUS_LINE_COMMAND,
            }}),
        ));
    }
    let mut out: Vec<_> = files
        .into_iter()
        .map(|(name, v)| {
            let text = serde_json::to_string_pretty(&v).expect("JSON value serializes") + "\n";
            (std::path::Path::new(super::PLUGIN_DIR).join(name), text)
        })
        .collect();
    if ui.panel || ui.flow_band {
        // `ui.panel` alone: the file as it always was; `ui.flow_band` adds `band`, and
        // without the panel says the pane is not to open (and has no documents to list)
        let mut panel = serde_json::Map::new();
        if ui.panel {
            let documents = ui.documents.clone().unwrap_or_else(|| vec!["docs".into()]);
            panel.insert("documents".into(), serde_json::json!(documents));
        }
        if ui.flow_band {
            panel.insert("band".into(), true.into());
        }
        if !ui.panel {
            panel.insert("pane".into(), false.into());
        }
        let panel = serde_json::Value::Object(panel);
        let modules = serde_json::json!({ "modules": ["./register.js"] });
        for (name, text) in [
            (
                "hooks/hooks.json",
                serde_json::to_string_pretty(&modules).expect("JSON value serializes") + "\n",
            ),
            ("hooks/register.js", PANEL_MOD.to_string()),
            (
                "panel.json",
                serde_json::to_string_pretty(&panel).expect("JSON value serializes") + "\n",
            ),
        ] {
            out.push((std::path::Path::new(super::PLUGIN_DIR).join(name), text));
        }
    }
    out
}

/// The paths (relative to the project root) of the files only `ui.panel` and `ui.flow_band`
/// write.
pub fn is_mod_file(rel: &str) -> bool {
    ["hooks/hooks.json", "hooks/register.js", "panel.json"]
        .iter()
        .any(|f| std::path::Path::new(super::PLUGIN_DIR).join(f) == std::path::Path::new(rel))
}

/// The JavaScript of the panel mod, written as is.
pub const PANEL_MOD: &str = include_str!("ui/armadai-ui-panel.js");

/// What Claude Code runs for the plugin's `subagentStatusLine`.
pub const SUBAGENT_STATUS_LINE_COMMAND: &str = "armadai __claude-subagent-statusline";

/// The nested keys the plugin needs in `.claude/settings.json`, by dotted path.
/// UNVERIFIED: the docs only show absolute paths for a `directory` source (relative
/// ones are not described); a path relative to the project is assumed.
pub fn plugin_settings_keys() -> BTreeMap<String, Value> {
    BTreeMap::from([
        (PLUGIN_ENABLED_KEY.to_string(), Value::Bool(true)),
        (
            PLUGIN_MARKETPLACE_KEY.to_string(),
            serde_json::json!({"source": {
                "source": "directory",
                "path": format!("./{}", super::PLUGIN_DIR),
            }}),
        ),
    ])
}

/// The keys link wants: [`native_keys`] plus the `statusLine` of `ui.status_line` and
/// the dotted plugin keys of `ui.subagent_status_line`; each together with the same
/// key of `native.claude` is an error.
pub fn native_settings(project: &Project) -> anyhow::Result<BTreeMap<String, Value>> {
    let mut keys = native_keys(project)?;
    let ui = &project.manifest.ui;
    if ui.subagent_status_line || ui.panel || ui.flow_band {
        let key = if ui.subagent_status_line {
            "ui.subagent_status_line"
        } else if ui.panel {
            "ui.panel"
        } else {
            "ui.flow_band"
        };
        for k in ["enabledPlugins", "extraKnownMarketplaces"] {
            if keys.contains_key(k) {
                bail!("project.yaml: native.claude key `{k}` collides with {key}");
            }
        }
        keys.extend(plugin_settings_keys());
    }
    if project.manifest.ui.status_line {
        if keys.contains_key("statusLine") {
            bail!("project.yaml: native.claude key `statusLine` collides with ui.status_line");
        }
        keys.insert("statusLine".into(), status_line_value());
    }
    if !project.manifest.policy.grants.is_empty() {
        guard_grant_state(&mut keys)?;
    }
    Ok(keys)
}

/// Add [`GRANT_GUARD_RULES`] to `permissions.deny`, after the project's own rules.
fn guard_grant_state(keys: &mut BTreeMap<String, Value>) -> anyhow::Result<()> {
    let perms = keys
        .entry("permissions".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    let Value::Object(perms) = perms else {
        bail!("native.claude key `permissions` must be a mapping when policy.grants is set");
    };
    let deny = perms
        .entry("deny")
        .or_insert_with(|| Value::Array(Vec::new()));
    let Value::Array(deny) = deny else {
        bail!("native.claude key `permissions.deny` must be a list when policy.grants is set");
    };
    for rule in GRANT_GUARD_RULES {
        if !deny.iter().any(|d| d.as_str() == Some(rule)) {
            deny.push(Value::String(rule.into()));
        }
    }
    Ok(())
}

/// The path of the first plugin key the user already set to another value (neither
/// the wanted one nor the one `previous` wrote): then every plugin key leaves `keys`
/// and the plugin is not written at all.
pub fn keep_users_plugin(
    existing: Option<&str>,
    keys: &mut BTreeMap<String, Value>,
    previous: &Owned,
) -> anyhow::Result<Option<String>> {
    let Some(text) = existing else {
        return Ok(None);
    };
    let doc = Value::Object(parse(Some(text))?);
    let clash = plugin_settings_keys()
        .into_iter()
        .find_map(|(path, wanted)| {
            let cur = path.split('.').try_fold(&doc, |v, k| v.get(k))?;
            (*cur != wanted && previous.plugin.values.get(&path) != Some(cur)).then_some(path)
        });
    if clash.is_some() {
        keys.retain(|k, _| !is_plugin_key(k));
    }
    Ok(clash)
}

/// What of the plugin is already in the project when no earlier link recorded it.
#[derive(Debug, PartialEq)]
pub enum FoundPlugin {
    /// None of its files and 2 settings entries.
    Absent,
    /// All of them, byte for byte (files) and value for value (entries), as link writes them.
    Identical,
    /// Anything else: the first element, in a fixed order, present with other content,
    /// else the first one missing (a file path relative to the root, or a dotted key).
    Differs(String),
}

/// Files first ([`plugin_files`] order), then entries ([`plugin_settings_keys`] order).
/// `settings` is the text of `.claude/settings.json`, when there is one; an invalid one
/// is an error already raised by [`keep_users_plugin`].
pub fn found_plugin(
    root: &std::path::Path,
    settings: Option<&str>,
    ui: &armadai_model::types::Ui,
) -> anyhow::Result<FoundPlugin> {
    let doc = Value::Object(parse(settings)?);
    // (element, found content equals what link writes: None when missing)
    let mut seen: Vec<(String, Option<bool>)> = Vec::new();
    for (rel, text) in plugin_files(ui) {
        let found = std::fs::read(root.join(&rel)).ok();
        seen.push((
            rel.display().to_string(),
            found.map(|b| b == text.as_bytes()),
        ));
    }
    for (path, wanted) in plugin_settings_keys() {
        let cur = path.split('.').try_fold(&doc, |v, k| v.get(k));
        seen.push((path, cur.map(|c| *c == wanted)));
    }
    if seen.iter().all(|(_, s)| s.is_none()) {
        return Ok(FoundPlugin::Absent);
    }
    if seen.iter().all(|(_, s)| *s == Some(true)) {
        return Ok(FoundPlugin::Identical);
    }
    let first = |f: fn(&Option<bool>) -> bool| seen.iter().find(|(_, s)| f(s));
    let (element, _) = first(|s| *s == Some(false))
        .or_else(|| first(Option::is_none))
        .expect("a differing or a missing element exists here");
    Ok(FoundPlugin::Differs(element.clone()))
}

/// A dotted key of `keys`: a nested plugin key, merged by [`merge_into`] instead of
/// being a top-level key. shortcut: a top-level `native.claude` key containing `.`
/// would be read the same way, no Claude Code setting has one.
pub(crate) fn is_plugin_key(k: &str) -> bool {
    k.contains('.')
}

/// The dotted keys of `keys`, taken out of it.
fn plugin_keys(keys: &mut BTreeMap<String, Value>) -> JsonKeys {
    let mut out = JsonKeys::default();
    let dotted: Vec<String> = keys.keys().filter(|k| is_plugin_key(k)).cloned().collect();
    for k in dotted {
        if let Some(v) = keys.remove(&k) {
            out.values.insert(k, v);
        }
    }
    out
}

/// When `existing` already holds a `statusLine` that is neither the wanted one nor the
/// one `previous` wrote: the key leaves `keys` (the user's bar stays) and this says so.
pub fn keep_users_status_line(
    existing: Option<&str>,
    keys: &mut BTreeMap<String, Value>,
    previous: &Owned,
) -> anyhow::Result<bool> {
    let (Some(wanted), Some(text)) = (keys.get("statusLine"), existing) else {
        return Ok(false);
    };
    let doc = parse(Some(text))?;
    let kept = doc
        .get("statusLine")
        .is_some_and(|cur| cur != wanted && previous.keys.get("statusLine") != Some(cur));
    if kept {
        keys.remove("statusLine");
    }
    Ok(kept)
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
    let plugin = plugin_keys(&mut keys);
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
    owned.plugin = on_value(&mut doc, |d| merge_into(d, &plugin, &previous.plugin))?;
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
    on_value(&mut doc, |d| remove_from(d, &previous.plugin));
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

    fn bar_keys() -> BTreeMap<String, Value> {
        BTreeMap::from([("statusLine".to_string(), status_line_value())])
    }

    #[test]
    fn a_status_line_is_kept_unless_it_is_ours_or_the_wanted_one() {
        let theirs = r#"{"statusLine":{"type":"command","command":"mine"}}"#;
        let old = json!({"type":"command","command":"armadai old"});
        let owns_old = Owned {
            keys: BTreeMap::from([("statusLine".to_string(), old.clone())]),
            ..Owned::default()
        };
        let old_text = json!({ "statusLine": old }).to_string();
        let wanted_text = json!({ "statusLine": status_line_value() }).to_string();
        let cases: [(Option<&str>, &Owned, bool); 5] = [
            (None, &Owned::default(), false),
            (Some(r#"{"a":1}"#), &Owned::default(), false),
            (Some(theirs), &Owned::default(), true),
            // written by an earlier link: ours to update
            (Some(&old_text), &owns_old, false),
            (Some(&wanted_text), &Owned::default(), false),
        ];
        for (i, (existing, prev, want)) in cases.into_iter().enumerate() {
            let mut keys = bar_keys();
            assert_eq!(
                keep_users_status_line(existing, &mut keys, prev).unwrap(),
                want,
                "case {i}"
            );
            assert_eq!(keys.contains_key("statusLine"), !want, "case {i}");
        }
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
    fn a_project_linked_before_the_marker_hook_gets_it_on_relink() {
        let old = flow_status_entries()[..4].to_vec();
        let (first, owned1) = merge(Some(USER), &old, &none(), &Owned::default()).unwrap();
        let (second, owned2) =
            merge(Some(&first), &flow_status_entries(), &none(), &owned1).unwrap();
        assert!(!first.contains("Agent|Task"));
        assert_eq!(
            value(&second)["hooks"]["PreToolUse"],
            json!([
                {"matcher": "Bash", "hooks": [{"type": "command", "command": "./mine.sh"}]},
                {"matcher": "SubagentHandback", "hooks": [{"type": "command", "command": "armadai __record-flow-event", "async": true}]},
                {"matcher": "Agent|Task", "hooks": [{"type": "command", "command": "armadai __record-flow-event", "async": true}]}
            ])
        );
        assert_eq!(owned2.entries, flow_status_entries());
    }

    fn grants_project(extra_manifest: &str) -> (tempfile::TempDir, Project) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".armadai/agents")).unwrap();
        std::fs::write(
            dir.path().join(".armadai/project.yaml"),
            format!(
                "schema_version: 1\npolicy:\n  grants:\n    zeta:\n      allow: [\"Read(/logs/*)\"]\n    auto-until-mr:\n      allow: [\"Bash(git push origin feature/*)\", \"Bash(git push:*)\"]\n      until: publish\n{extra_manifest}"
            ),
        )
        .unwrap();
        std::fs::write(
            dir.path().join(".armadai/agents/dev.md"),
            "---\ndescription: d\n---\nB\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join(".armadai/flows")).unwrap();
        std::fs::write(
            dir.path().join(".armadai/flows/ev.yaml"),
            "steps:\n  publish: { agent: dev }\n",
        )
        .unwrap();
        let (p, _) = armadai_model::load_checked(dir.path()).unwrap();
        (dir, p)
    }

    fn grant(tool: &str, level: &str, rule: &str) -> HookEntry {
        HookEntry {
            event: "PermissionRequest".into(),
            matcher: Some(tool.into()),
            command: format!("armadai __claude-grant {level}"),
            native: BTreeMap::from([("if".to_string(), json!(rule))]),
        }
    }

    #[test]
    fn a_level_becomes_one_permission_request_hook_per_rule() {
        let (_d, p) = grants_project("");
        // levels in id order, rules in order
        assert_eq!(
            grant_entries(&p),
            vec![
                grant("Bash", "auto-until-mr", "Bash(git push origin feature/*)"),
                grant("Bash", "auto-until-mr", "Bash(git push:*)"),
            ]
        );
        // `zeta` lists a `Read` rule only: no hook, a level answers `Bash` rules alone
        assert!(
            grant_entries(&p)
                .iter()
                .all(|e| e.command != "armadai __claude-grant zeta")
        );
        let hooks = hook_entries(&p).unwrap();
        assert_eq!(&hooks[hooks.len() - 2..], grant_entries(&p).as_slice());
    }

    #[test]
    fn only_a_bash_pattern_rule_gets_a_hook() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".armadai")).unwrap();
        std::fs::write(
            dir.path().join(".armadai/project.yaml"),
            "schema_version: 1\npolicy:\n  grants:\n    g:\n      allow: [\"Bash\", \" Bash(x)\", \"Bashful(y)\", \"Bash(ok *)\"]\n",
        )
        .unwrap();
        let (p, _) = armadai_model::load_checked(dir.path()).unwrap();
        assert_eq!(grant_entries(&p), vec![grant("Bash", "g", "Bash(ok *)")]);
    }

    #[test]
    fn without_levels_there_is_no_grant_hook_and_no_guard_rule() {
        let (_d, p) = {
            let dir = tempfile::tempdir().unwrap();
            std::fs::create_dir_all(dir.path().join(".armadai")).unwrap();
            std::fs::write(
                dir.path().join(".armadai/project.yaml"),
                "schema_version: 1\n",
            )
            .unwrap();
            let (p, _) = armadai_model::load_checked(dir.path()).unwrap();
            (dir, p)
        };
        assert_eq!(grant_entries(&p), vec![]);
        assert_eq!(hook_entries(&p).unwrap(), vec![]);
        assert_eq!(native_settings(&p).unwrap(), BTreeMap::new());
    }

    #[test]
    fn levels_add_the_two_guard_rules_after_the_projects_own_denies() {
        let (_d, p) = grants_project(
            "native:\n  claude:\n    permissions:\n      deny: [\"Bash(curl *)\"]\n",
        );
        assert_eq!(
            native_settings(&p).unwrap()["permissions"],
            json!({"deny": ["Bash(curl *)", "Bash(armadai grant *)", "Edit(/.armadai/state/grants.yaml)"]})
        );
        // a guard rule the project already denies is not added twice
        let (_d, p) = grants_project(
            "native:\n  claude:\n    permissions:\n      deny: [\"Bash(armadai grant *)\"]\n",
        );
        assert_eq!(
            native_settings(&p).unwrap()["permissions"],
            json!({"deny": ["Bash(armadai grant *)", "Edit(/.armadai/state/grants.yaml)"]})
        );
        // no native permissions: the guard alone
        let (_d, p) = grants_project("");
        assert_eq!(
            native_settings(&p).unwrap()["permissions"],
            json!({"deny": ["Bash(armadai grant *)", "Edit(/.armadai/state/grants.yaml)"]})
        );
    }

    #[test]
    fn grant_hooks_and_guard_rules_are_owned_and_unlink_gives_the_user_file_back() {
        let (_d, p) = grants_project("");
        let hooks = hook_entries(&p).unwrap();
        let keys = native_settings(&p).unwrap();
        let (merged, owned, _) =
            merge_settings(Some(USER), &hooks, &keys, &Owned::default(), false).unwrap();
        let v = value(&merged);
        assert_eq!(
            v["hooks"]["PermissionRequest"],
            json!([
                {"matcher": "Bash", "hooks": [{"type": "command", "command": "armadai __claude-grant auto-until-mr", "if": "Bash(git push origin feature/*)"}]},
                {"matcher": "Bash", "hooks": [{"type": "command", "command": "armadai __claude-grant auto-until-mr", "if": "Bash(git push:*)"}]},
            ])
        );
        assert_eq!(
            v["permissions"]["deny"],
            json!(["Bash(armadai grant *)", "Edit(/.armadai/state/grants.yaml)"])
        );
        assert_eq!(
            value(&remove_entries(&merged, &owned).unwrap()),
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
                recording("PreToolUse", Some("Agent|Task")),
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

    #[test]
    fn an_owned_record_of_an_earlier_version_has_no_plugin() {
        let o: Owned = armadai_model::yaml::from_str("entries: []\n").unwrap();
        assert!(o.plugin.is_empty());
    }

    #[test]
    fn a_plugin_key_of_the_user_is_kept_unless_it_is_ours_or_the_wanted_one() {
        let wanted = plugin_settings_keys();
        let off = r#"{"enabledPlugins":{"armadai-ui@armadai-ui":false}}"#;
        let other_market = r#"{"extraKnownMarketplaces":{"armadai-ui":{"source":{"source":"github","repo":"a/b"}}}}"#;
        let same = json!({"enabledPlugins":{"armadai-ui@armadai-ui":true}}).to_string();
        let mut old = Owned::default();
        old.plugin
            .values
            .insert(PLUGIN_ENABLED_KEY.into(), json!(false));
        let cases: [(Option<&str>, &Owned, Option<&str>); 6] = [
            (None, &Owned::default(), None),
            (Some(r#"{"a":1}"#), &Owned::default(), None),
            (Some(off), &Owned::default(), Some(PLUGIN_ENABLED_KEY)),
            (
                Some(other_market),
                &Owned::default(),
                Some(PLUGIN_MARKETPLACE_KEY),
            ),
            // written by an earlier link: ours to update
            (Some(off), &old, None),
            (Some(&same), &Owned::default(), None),
        ];
        for (i, (existing, prev, want)) in cases.into_iter().enumerate() {
            let mut keys = wanted.clone();
            let got = keep_users_plugin(existing, &mut keys, prev).unwrap();
            assert_eq!(got.as_deref(), want, "case {i}");
            assert_eq!(keys.is_empty(), want.is_some(), "case {i}");
        }
    }

    /// The files of the plugin for each combination of the keys that ask for the mod.
    #[test]
    fn the_mod_is_written_for_the_panel_or_the_band_with_its_own_panel_json() {
        let names = |ui: &armadai_model::types::Ui| -> Vec<String> {
            plugin_files(ui)
                .iter()
                .map(|(p, _)| {
                    p.strip_prefix(super::super::PLUGIN_DIR)
                        .unwrap()
                        .display()
                        .to_string()
                })
                .collect()
        };
        let panel_json = |ui: &armadai_model::types::Ui| {
            plugin_files(ui)
                .into_iter()
                .find(|(p, _)| p.ends_with("panel.json"))
                .map(|(_, t)| t)
        };
        let ui = |panel, flow_band| armadai_model::types::Ui {
            panel,
            flow_band,
            ..Default::default()
        };
        let manifests = [
            ".claude-plugin/marketplace.json",
            ".claude-plugin/plugin.json",
        ];
        let with_mod = [
            ".claude-plugin/marketplace.json",
            ".claude-plugin/plugin.json",
            "hooks/hooks.json",
            "hooks/register.js",
            "panel.json",
        ];
        assert_eq!(names(&ui(false, false)), manifests);
        assert_eq!(names(&ui(true, false)), with_mod);
        assert_eq!(names(&ui(false, true)), with_mod);
        assert_eq!(names(&ui(true, true)), with_mod);
        assert_eq!(panel_json(&ui(false, false)), None);
        // the panel alone: the file as before this key existed (no `band` at all)
        assert_eq!(
            panel_json(&ui(true, false)).unwrap(),
            "{\n  \"documents\": [\n    \"docs\"\n  ]\n}\n"
        );
        assert_eq!(
            panel_json(&ui(true, true)).unwrap(),
            "{\n  \"documents\": [\n    \"docs\"\n  ],\n  \"band\": true\n}\n"
        );
        assert_eq!(
            panel_json(&ui(false, true)).unwrap(),
            "{\n  \"band\": true,\n  \"pane\": false\n}\n"
        );
    }

    #[test]
    fn only_the_three_exact_paths_are_mod_files() {
        let p = super::super::PLUGIN_DIR;
        for f in ["hooks/hooks.json", "hooks/register.js", "panel.json"] {
            assert!(is_mod_file(&format!("{p}/{f}")), "{f}");
        }
        // same endings, other places or other names
        for rel in [
            format!("{p}/.claude-plugin/panel.json"),
            format!("{p}/settings.json"),
            format!("{p}/.claude-plugin/plugin.json"),
            "other/hooks/register.js".to_string(),
            format!("{p}/hooks/register.js.bak"),
            "panel.json".to_string(),
        ] {
            assert!(!is_mod_file(&rel), "{rel}");
        }
    }

    #[test]
    fn found_plugin_tells_absent_identical_and_the_first_element_that_differs() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let put = |rel: &std::path::Path, text: &str| {
            std::fs::create_dir_all(root.join(rel).parent().unwrap()).unwrap();
            std::fs::write(root.join(rel), text).unwrap();
        };
        let ui = armadai_model::types::Ui {
            subagent_status_line: true,
            ..Default::default()
        };
        let files = plugin_files(&ui);
        let (m, p, s) = (&files[0], &files[1], &files[2]);
        let entries = json!({
            "enabledPlugins": {"armadai-ui@armadai-ui": true},
            "extraKnownMarketplaces": {"armadai-ui": {"source": {
                "source": "directory", "path": "./.armadai/plugins/armadai-ui"}}},
        })
        .to_string();
        let name = |f: &(std::path::PathBuf, String)| f.0.display().to_string();
        let found = |settings: Option<&str>| found_plugin(root, settings, &ui).unwrap();

        assert_eq!(found(None), FoundPlugin::Absent, "absent");
        assert_eq!(found(Some("{}")), FoundPlugin::Absent, "absent, empty file");
        // the entries alone: the first file is the first missing element
        assert_eq!(
            found(Some(&entries)),
            FoundPlugin::Differs(name(m)),
            "entries only"
        );
        for f in &files {
            put(&f.0, &f.1);
        }
        assert_eq!(found(Some(&entries)), FoundPlugin::Identical, "identical");
        // a file present with other content, whichever it is
        for f in [m, p, s] {
            let mut other = f.1.clone();
            other.push(' ');
            put(&f.0, &other);
            assert_eq!(
                found(Some(&entries)),
                FoundPlugin::Differs(name(f)),
                "{}",
                name(f)
            );
            put(&f.0, &f.1);
        }
        // the files without entries: the first entry is the first missing element
        assert_eq!(
            found(None),
            FoundPlugin::Differs(PLUGIN_ENABLED_KEY.into()),
            "no entries"
        );
        let only_market = json!({"extraKnownMarketplaces": serde_json::from_str::<Value>(&entries)
            .unwrap()["extraKnownMarketplaces"]})
        .to_string();
        assert_eq!(
            found(Some(&only_market)),
            FoundPlugin::Differs(PLUGIN_ENABLED_KEY.into()),
            "one entry missing"
        );
        // an entry with another value
        let off = json!({"enabledPlugins": {"armadai-ui@armadai-ui": false}}).to_string();
        assert_eq!(
            found(Some(&off)),
            FoundPlugin::Differs(PLUGIN_ENABLED_KEY.into()),
            "entry differs"
        );
        // a differing element is named before a missing one
        std::fs::remove_file(root.join(&m.0)).unwrap();
        let mut other = p.1.clone();
        other.push(' ');
        put(&p.0, &other);
        assert_eq!(
            found(Some(&entries)),
            FoundPlugin::Differs(name(p)),
            "differs before missing"
        );
    }
}
