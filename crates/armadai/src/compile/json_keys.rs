//! Owned keys of a JSON config file shared with the user (`opencode.json`): values
//! at a dotted path (`agent.build.permission.task`) and entries of arrays
//! (`instructions`), merged without touching anything else.
//!
//! As for `settings.json` ([`super::claude_settings`]), ownership is the exact values
//! recorded in the link manifest ([`OwnedJson`]); removal is by value equality.

use std::collections::BTreeMap;

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// What a link wants in the file, keyed by dotted path.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JsonKeys {
    pub values: BTreeMap<String, Value>,
    pub array_entries: BTreeMap<String, Vec<Value>>,
}

/// What a link added to the file, as exact values.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct OwnedJson {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub values: BTreeMap<String, Value>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub array_entries: BTreeMap<String, Vec<Value>>,
    /// Objects and arrays link created: removed once empty again; one the user had,
    /// even empty, is kept.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub containers: Vec<String>,
}

fn parse(existing: Option<&str>) -> anyhow::Result<Value> {
    match existing {
        None => Ok(Value::Object(Map::new())),
        Some(t) => match serde_json::from_str(t).context("not valid JSON")? {
            v @ Value::Object(_) => Ok(v),
            _ => bail!("not a JSON object"),
        },
    }
}

fn render(doc: Value) -> String {
    let mut s = serde_json::to_string_pretty(&doc).expect("JSON value serializes");
    s.push('\n');
    s
}

// shortcut: a dot always separates keys, so a key containing `.` cannot be owned;
// upgrade to a list of segments if one is ever needed.
fn split(path: &str) -> (Vec<&str>, &str) {
    let mut parts: Vec<&str> = path.split('.').collect();
    let leaf = parts.pop().expect("split yields at least one part");
    (parts, leaf)
}

fn get<'a>(doc: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.').try_fold(doc, |v, k| v.get(k))
}

/// The object holding `path`'s leaf, or `None` when a step is missing.
fn parent_mut<'a>(doc: &'a mut Value, path: &str) -> Option<&'a mut Map<String, Value>> {
    let (parents, _) = split(path);
    parents
        .into_iter()
        .try_fold(doc, |v, k| v.get_mut(k))?
        .as_object_mut()
}

/// The object holding `path`'s leaf, creating the missing objects on the way (each
/// recorded in `created`).
fn parent_or_create<'a>(
    doc: &'a mut Value,
    path: &str,
    created: &mut Vec<String>,
) -> anyhow::Result<&'a mut Map<String, Value>> {
    let (parents, _) = split(path);
    let mut cur = doc;
    let mut at = String::new();
    for k in parents {
        if !at.is_empty() {
            at.push('.');
        }
        at.push_str(k);
        let obj = cur
            .as_object_mut()
            .with_context(|| format!("`{at}` is inside a value that is not an object"))?;
        if !obj.contains_key(k) {
            obj.insert(k.into(), Value::Object(Map::new()));
            created.push(at.clone());
        }
        cur = &mut obj[k];
    }
    cur.as_object_mut()
        .with_context(|| format!("`{at}` is not an object"))
}

/// Remove what `values` and `entries` own, by value; a value the user changed is kept.
fn unmerge(doc: &mut Value, values: &[(&String, &Value)], entries: &[(&String, &Value)]) {
    for (k, v) in values {
        let (_, leaf) = split(k);
        if let Some(obj) = parent_mut(doc, k)
            && obj.get(leaf) == Some(*v)
        {
            obj.shift_remove(leaf);
        }
    }
    for (k, e) in entries {
        let (_, leaf) = split(k);
        if let Some(arr) = parent_mut(doc, k)
            .and_then(|o| o.get_mut(leaf))
            .and_then(Value::as_array_mut)
            && let Some(i) = arr.iter().position(|x| x == *e)
        {
            arr.remove(i);
        }
    }
}

/// Drop the containers link created once empty, deepest first; keep listed only those
/// still there.
fn prune(doc: &mut Value, containers: &mut Vec<String>) {
    containers.sort_by_key(|c| std::cmp::Reverse(c.matches('.').count()));
    for c in containers.iter() {
        let empty = get(doc, c).is_some_and(|v| match v {
            Value::Object(m) => m.is_empty(),
            Value::Array(a) => a.is_empty(),
            _ => false,
        });
        if empty && let Some(obj) = parent_mut(doc, c) {
            obj.shift_remove(split(c).1);
        }
    }
    containers.retain(|c| get(doc, c).is_some());
    containers.sort();
    containers.dedup();
}

/// Merge `wanted` into `existing` (absent = new file). What `previous` owned and is no
/// longer wanted is removed by value; a wanted value already present with the same
/// value is not claimed, with another value is an error (naming the key), as is an
/// owned value changed by hand since the last link. Returns the new text and what
/// this link owns.
pub fn merge_json_keys(
    existing: Option<&str>,
    wanted: &JsonKeys,
    previous: &OwnedJson,
) -> anyhow::Result<(String, OwnedJson)> {
    let mut doc = parse(existing)?;
    // a key still wanted is replaced in place below, not removed and appended
    let gone_values: Vec<(&String, &Value)> = (previous.values.iter())
        .filter(|(k, _)| !wanted.values.contains_key(*k))
        .collect();
    let gone_entries: Vec<(&String, &Value)> = (previous.array_entries.iter())
        .flat_map(|(k, es)| es.iter().map(move |e| (k, e)))
        .filter(|(k, e)| !wanted.array_entries.get(*k).is_some_and(|w| w.contains(e)))
        .collect();
    unmerge(&mut doc, &gone_values, &gone_entries);

    let mut owned = OwnedJson {
        containers: previous.containers.clone(),
        ..OwnedJson::default()
    };
    for (k, v) in &wanted.values {
        let (_, leaf) = split(k);
        let obj = parent_or_create(&mut doc, k, &mut owned.containers)?;
        match obj.get(leaf) {
            Some(cur) if cur == v => {
                if previous.values.get(k) == Some(v) {
                    owned.values.insert(k.clone(), v.clone());
                }
            }
            Some(cur) if previous.values.get(k) == Some(cur) => {
                obj[leaf] = v.clone();
                owned.values.insert(k.clone(), v.clone());
            }
            Some(_) if previous.values.contains_key(k) => {
                bail!("key `{k}` was changed by hand since the last link")
            }
            Some(_) => bail!("key `{k}` is already set to another value"),
            None => {
                obj.insert(leaf.into(), v.clone());
                owned.values.insert(k.clone(), v.clone());
            }
        }
    }
    for (k, es) in &wanted.array_entries {
        let (_, leaf) = split(k);
        let obj = parent_or_create(&mut doc, k, &mut owned.containers)?;
        if !obj.contains_key(leaf) {
            obj.insert(leaf.into(), Value::Array(Vec::new()));
            owned.containers.push(k.clone());
        }
        let arr = obj[leaf]
            .as_array_mut()
            .with_context(|| format!("key `{k}` is not an array"))?;
        let prev = previous.array_entries.get(k);
        let mut mine = Vec::new();
        for e in es {
            if arr.contains(e) {
                if prev.is_some_and(|p| p.contains(e)) {
                    mine.push(e.clone());
                }
                continue;
            }
            arr.push(e.clone());
            mine.push(e.clone());
        }
        if !mine.is_empty() {
            owned.array_entries.insert(k.clone(), mine);
        }
    }
    prune(&mut doc, &mut owned.containers);
    Ok((render(doc), owned))
}

/// Remove exactly what `previous` owns from `existing`.
pub fn remove_json_keys(existing: &str, previous: &OwnedJson) -> anyhow::Result<String> {
    let mut doc = parse(Some(existing))?;
    let values: Vec<(&String, &Value)> = previous.values.iter().collect();
    let entries: Vec<(&String, &Value)> = (previous.array_entries.iter())
        .flat_map(|(k, es)| es.iter().map(move |e| (k, e)))
        .collect();
    unmerge(&mut doc, &values, &entries);
    prune(&mut doc, &mut previous.containers.clone());
    Ok(render(doc))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const TEAM: &str = ".opencode/armadai-team.md";

    fn value(t: &str) -> Value {
        serde_json::from_str(t).unwrap()
    }

    fn wanted(default_agent: &str) -> JsonKeys {
        JsonKeys {
            values: [("default_agent".to_string(), json!(default_agent))].into(),
            array_entries: [("instructions".to_string(), vec![json!(TEAM)])].into(),
        }
    }

    fn nested() -> JsonKeys {
        JsonKeys {
            values: [
                (
                    "agent.build.permission.task".to_string(),
                    json!({"*": "deny", "dev": "allow"}),
                ),
                ("experimental.subagent_depth".to_string(), json!(3)),
            ]
            .into(),
            array_entries: BTreeMap::new(),
        }
    }

    const USER: &str =
        r#"{"theme":"x","instructions":["mine.md"],"agent":{"build":{"model":"m"}}}"#;

    #[test]
    fn creates_the_file_from_nothing_and_lists_the_containers_it_created() {
        let (text, owned) = merge_json_keys(None, &wanted("lead"), &OwnedJson::default()).unwrap();
        assert_eq!(
            value(&text),
            json!({"default_agent": "lead", "instructions": [TEAM]})
        );
        assert_eq!(
            owned,
            OwnedJson {
                values: [("default_agent".to_string(), json!("lead"))].into(),
                array_entries: [("instructions".to_string(), vec![json!(TEAM)])].into(),
                containers: vec!["instructions".into()],
            }
        );
        let (text, owned) = merge_json_keys(None, &nested(), &OwnedJson::default()).unwrap();
        assert_eq!(
            value(&text),
            json!({
                "agent": {"build": {"permission": {"task": {"*": "deny", "dev": "allow"}}}},
                "experimental": {"subagent_depth": 3}
            })
        );
        assert_eq!(
            owned.containers,
            [
                "agent",
                "agent.build",
                "agent.build.permission",
                "experimental"
            ]
        );
        assert!(text.ends_with("}\n"));
    }

    #[test]
    fn user_keys_are_kept_and_an_owned_entry_is_appended() {
        let mut keys = wanted("lead");
        keys.values.extend(nested().values);
        let (text, owned) = merge_json_keys(Some(USER), &keys, &OwnedJson::default()).unwrap();
        let v = value(&text);
        assert_eq!(v["theme"], json!("x"));
        assert_eq!(v["instructions"], json!(["mine.md", TEAM]));
        assert_eq!(v["agent"]["build"]["model"], json!("m"));
        assert_eq!(
            owned.containers,
            ["agent.build.permission", "experimental"],
            "`agent`, `agent.build` and `instructions` are the user's"
        );
        let (twice, owned2) = merge_json_keys(Some(&text), &keys, &owned).unwrap();
        assert_eq!((twice.as_str(), &owned2), (text.as_str(), &owned));
        assert_eq!(
            value(&remove_json_keys(&text, &owned).unwrap()),
            value(USER)
        );
    }

    #[test]
    fn relinking_replaces_an_owned_value_and_drops_an_entry_no_longer_wanted() {
        let (first, owned) =
            merge_json_keys(Some(USER), &wanted("lead"), &OwnedJson::default()).unwrap();
        let other = JsonKeys {
            values: [("default_agent".to_string(), json!("boss"))].into(),
            array_entries: BTreeMap::new(),
        };
        let (second, owned2) = merge_json_keys(Some(&first), &other, &owned).unwrap();
        let mut expected = value(USER);
        expected["default_agent"] = json!("boss");
        assert_eq!(value(&second), expected);
        assert_eq!(
            owned2,
            OwnedJson {
                values: [("default_agent".to_string(), json!("boss"))].into(),
                ..OwnedJson::default()
            }
        );
        // a container link created goes once nothing is wanted in it any more
        let (first, owned) = merge_json_keys(None, &nested(), &OwnedJson::default()).unwrap();
        let (second, owned2) = merge_json_keys(Some(&first), &JsonKeys::default(), &owned).unwrap();
        assert_eq!(value(&second), json!({}));
        assert_eq!(owned2, OwnedJson::default());
    }

    #[test]
    fn an_owned_value_changed_by_hand_is_an_error_naming_the_key() {
        let (first, owned) = merge_json_keys(None, &wanted("lead"), &OwnedJson::default()).unwrap();
        let edited = first.replace("\"lead\"", "\"other\"");
        for keys in [wanted("lead"), wanted("boss")] {
            let err = merge_json_keys(Some(&edited), &keys, &owned).unwrap_err();
            assert_eq!(
                err.to_string(),
                "key `default_agent` was changed by hand since the last link"
            );
        }
    }

    #[test]
    fn a_wanted_key_set_by_the_user_to_another_value_is_an_error_naming_the_key() {
        let err = merge_json_keys(
            Some(r#"{"default_agent": "mine"}"#),
            &wanted("lead"),
            &OwnedJson::default(),
        )
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            "key `default_agent` is already set to another value"
        );
        let err = merge_json_keys(
            Some(r#"{"agent": {"build": 1}}"#),
            &nested(),
            &OwnedJson::default(),
        )
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            "`agent.build.permission` is inside a value that is not an object"
        );
        let err = merge_json_keys(
            Some(r#"{"instructions": "one.md"}"#),
            &wanted("lead"),
            &OwnedJson::default(),
        )
        .unwrap_err();
        assert_eq!(err.to_string(), "key `instructions` is not an array");
    }

    #[test]
    fn an_identical_value_or_entry_already_present_is_not_claimed() {
        let user = format!(r#"{{"default_agent": "lead", "instructions": ["{TEAM}"]}}"#);
        let (text, owned) =
            merge_json_keys(Some(&user), &wanted("lead"), &OwnedJson::default()).unwrap();
        assert_eq!(value(&text), value(&user));
        assert_eq!(owned, OwnedJson::default());
        assert_eq!(
            value(&remove_json_keys(&text, &owned).unwrap()),
            value(&user)
        );
    }

    #[test]
    fn removing_drops_owned_values_entries_and_empty_created_containers_only() {
        let (text, owned) = merge_json_keys(None, &nested(), &OwnedJson::default()).unwrap();
        assert_eq!(value(&remove_json_keys(&text, &owned).unwrap()), json!({}));

        // the user had `agent` (empty) and adds a key beside an owned one
        let user = r#"{"agent": {}}"#;
        let (text, owned) = merge_json_keys(Some(user), &nested(), &OwnedJson::default()).unwrap();
        let mut v = value(&text);
        v["experimental"]["mine"] = json!(true);
        let rest = remove_json_keys(&v.to_string(), &owned).unwrap();
        assert_eq!(
            value(&rest),
            json!({"agent": {}, "experimental": {"mine": true}})
        );

        // an owned value the user changed stays
        let (text, owned) = merge_json_keys(None, &wanted("lead"), &OwnedJson::default()).unwrap();
        let edited = text.replace("\"lead\"", "\"other\"");
        assert_eq!(
            value(&remove_json_keys(&edited, &owned).unwrap()),
            json!({"default_agent": "other"})
        );
    }

    /// OpenCode permission rules are ordered (the last match wins): removing or
    /// replacing an owned key must leave every other key where it was, in the text.
    #[test]
    fn the_key_order_of_the_user_is_kept_on_unlink_and_relink() {
        let user = r#"{"agent":{"build":{"permission":{"read":"deny","*":"ask"}}}}"#;
        let task = |t: &str| JsonKeys {
            values: [("agent.build.permission.task".to_string(), json!({"*": t}))].into(),
            array_entries: BTreeMap::new(),
        };
        let (text, owned) =
            merge_json_keys(Some(user), &task("deny"), &OwnedJson::default()).unwrap();
        assert_eq!(
            text,
            render(value(
                r#"{"agent":{"build":{"permission":{"read":"deny","*":"ask","task":{"*":"deny"}}}}}"#
            ))
        );
        // the user then adds rules after the owned one
        let edited = render(value(
            r#"{"agent":{"build":{"permission":{"read":"deny","*":"ask","task":{"*":"deny"},"edit":"allow","shell":"deny"}}}}"#,
        ));
        assert_eq!(
            remove_json_keys(&edited, &owned).unwrap(),
            render(value(
                r#"{"agent":{"build":{"permission":{"read":"deny","*":"ask","edit":"allow","shell":"deny"}}}}"#
            )),
            "unlink"
        );
        // a new rule replaces the owned one in place
        let (relinked, _) = merge_json_keys(Some(&edited), &task("ask"), &owned).unwrap();
        assert_eq!(
            relinked,
            render(value(
                r#"{"agent":{"build":{"permission":{"read":"deny","*":"ask","task":{"*":"ask"},"edit":"allow","shell":"deny"}}}}"#
            )),
            "relink"
        );
        // a container link created goes without moving the keys added after it
        let depth = JsonKeys {
            values: [("experimental.subagent_depth".to_string(), json!(2))].into(),
            array_entries: BTreeMap::new(),
        };
        let (_, owned) =
            merge_json_keys(Some(r#"{"theme":"x"}"#), &depth, &OwnedJson::default()).unwrap();
        let edited = render(value(
            r#"{"theme":"x","experimental":{"subagent_depth":2},"a":1,"b":2}"#,
        ));
        assert_eq!(
            remove_json_keys(&edited, &owned).unwrap(),
            render(value(r#"{"theme":"x","a":1,"b":2}"#)),
            "pruned container"
        );
    }

    #[test]
    fn invalid_json_is_an_error() {
        let none = OwnedJson::default();
        assert!(merge_json_keys(Some("{ not json"), &wanted("lead"), &none).is_err());
        assert!(merge_json_keys(Some("[1]"), &wanted("lead"), &none).is_err());
        assert!(remove_json_keys("{ not json", &none).is_err());
    }
}
