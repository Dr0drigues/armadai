//! Pure merge of a project onto the pack it references (reference mode).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::types::NativeValue;

/// Where a file of the effective `.armadai/` comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layer {
    Pack,
    /// The `i`-th pack of `standards:`.
    Standards(usize),
    Project,
    Merged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OverlayError {
    /// `project.yaml` of `layer` is not YAML.
    InvalidYaml { layer: Layer, message: String },
    /// The pack's own `project.yaml` has a `pack:` key.
    NestedPack,
    /// The pack's own `project.yaml` has a `standards:` key.
    NestedStandards,
}

/// A skill id brought by two pack sources (`None` = the team pack, `Some(i)` = `standards[i]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillClash {
    pub skill: String,
    pub first: Option<usize>,
    pub second: usize,
}

/// The layered view: files plus what the caller must report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stacked {
    pub files: BTreeMap<PathBuf, (Layer, Vec<u8>)>,
    pub clashes: Vec<SkillClash>,
    /// `(index of the standards layer, relative path)` of a file outside `skills/<id>/`.
    pub foreign: Vec<(usize, PathBuf)>,
}

/// Two `project.yaml` values merged: maps key by key (recursively), lists completed
/// (the pack's items, then the project's absent from them, order kept), any other
/// value taken from the project; a `policy.grants` level is the exception, see
/// `replace_grants`.
pub fn merge_manifest_values(pack: &Value, project: &Value) -> Value {
    let mut merged = merge_values(pack, project);
    replace_grants(&mut merged, project);
    merged
}

fn merge_values(pack: &Value, project: &Value) -> Value {
    match (pack, project) {
        (Value::Object(a), Value::Object(b)) => {
            let mut out = a.clone();
            for (k, v) in b {
                let merged = match a.get(k) {
                    Some(p) => merge_values(p, v),
                    None => v.clone(),
                };
                out.insert(k.clone(), merged);
            }
            Value::Object(out)
        }
        (Value::Array(a), Value::Array(b)) => {
            let mut out = a.clone();
            for v in b {
                if !out.contains(v) {
                    out.push(v.clone());
                }
            }
            Value::Array(out)
        }
        (_, p) => p.clone(),
    }
}

/// A grant level is one unit: the project's level replaces the pack's level of the same
/// key, its `allow` is not completed with the pack's rules (a level never widens silently).
fn replace_grants(merged: &mut Value, project: &Value) {
    let Some(mine) = project.pointer("/policy/grants").and_then(Value::as_object) else {
        return;
    };
    if let Some(Value::Object(out)) = merged.pointer_mut("/policy/grants") {
        for (k, v) in mine {
            out.insert(k.clone(), v.clone());
        }
    }
}

fn manifest_path() -> &'static Path {
    Path::new("project.yaml")
}

fn read(layer: Layer, bytes: &[u8]) -> Result<Value, OverlayError> {
    let bad = |message: String| OverlayError::InvalidYaml { layer, message };
    let text = std::str::from_utf8(bytes).map_err(|e| bad(e.to_string()))?;
    match crate::yaml::from_str_manifest(text).map_err(|e| bad(e.to_string()))? {
        Value::Null => Ok(Value::Object(Default::default())),
        v @ Value::Object(_) => Ok(v),
        _ => Err(bad("not a mapping".into())),
    }
}

/// The effective `.armadai/` of a project referencing a pack (paths relative to
/// `.armadai/`): every pack file, replaced by the project's file at the same path,
/// plus the project's other files; `project.yaml` merged by `merge_manifest_values`,
/// without the project's `pack` key, written with `crate::yaml` (numbers kept plain
/// through `NativeValue`).
pub fn overlay(
    pack: &BTreeMap<PathBuf, Vec<u8>>,
    project: &BTreeMap<PathBuf, Vec<u8>>,
) -> Result<BTreeMap<PathBuf, (Layer, Vec<u8>)>, OverlayError> {
    Ok(overlay_standards(Some(pack), &[], project)?.files)
}

/// The id of the skill a path belongs to (`skills/<id>/…`).
fn skill_id(path: &Path) -> Option<&str> {
    let mut c = path.components();
    let (first, id, _file) = (c.next()?, c.next()?, c.next()?);
    (first.as_os_str() == "skills")
        .then(|| id.as_os_str().to_str())
        .flatten()
}

/// `overlay` over N layers: the team pack (if any), then each standards pack in order, then
/// the project. A standards pack contributes `skills/<id>/…` only; any other file goes to
/// `foreign`. An id held by two pack sources is a clash (the later folder is left out)
/// unless the project has `skills/<id>/`, which then replaces the standards folder whole.
pub fn overlay_standards(
    team: Option<&BTreeMap<PathBuf, Vec<u8>>>,
    standards: &[&BTreeMap<PathBuf, Vec<u8>>],
    project: &BTreeMap<PathBuf, Vec<u8>>,
) -> Result<Stacked, OverlayError> {
    let mut out: BTreeMap<PathBuf, (Layer, Vec<u8>)> = BTreeMap::new();
    let mut owners: BTreeMap<String, Option<usize>> = BTreeMap::new();
    for (p, b) in team.into_iter().flatten() {
        if let Some(id) = skill_id(p) {
            owners.entry(id.to_string()).or_insert(None);
        }
        out.insert(p.clone(), (Layer::Pack, b.clone()));
    }
    let project_skills: BTreeSet<&str> = project.keys().filter_map(|p| skill_id(p)).collect();
    let (mut clashes, mut foreign) = (Vec::new(), Vec::new());
    for (i, layer) in standards.iter().enumerate() {
        let mut clashed = BTreeSet::new();
        let mut taken = BTreeSet::new();
        for (p, b) in *layer {
            let Some(id) = skill_id(p) else {
                foreign.push((i, p.clone()));
                continue;
            };
            if project_skills.contains(id) {
                continue;
            }
            match owners.get(id) {
                Some(&first) => {
                    if clashed.insert(id) {
                        clashes.push(SkillClash {
                            skill: id.to_string(),
                            first,
                            second: i,
                        });
                    }
                }
                _ => {
                    taken.insert(id);
                    out.insert(p.clone(), (Layer::Standards(i), b.clone()));
                }
            }
        }
        for id in taken {
            owners.insert(id.to_string(), Some(i));
        }
    }
    for (p, b) in project {
        out.insert(p.clone(), (Layer::Project, b.clone()));
    }

    let finish = |files| Stacked {
        files,
        clashes: clashes.clone(),
        foreign: foreign.clone(),
    };
    let pack_value = team
        .and_then(|t| t.get(manifest_path()))
        .map(|b| read(Layer::Pack, b))
        .transpose()?;
    if let Some(v) = &pack_value {
        if v.get("pack").is_some() {
            return Err(OverlayError::NestedPack);
        }
        if v.get("standards").is_some() {
            return Err(OverlayError::NestedStandards);
        }
    }
    let project_value = project
        .get(manifest_path())
        .map(|b| read(Layer::Project, b))
        .transpose()?;

    let merged = match (pack_value, project_value) {
        (None, None) => return Ok(finish(out)),
        (Some(_), None) => return Ok(finish(out)), // already Layer::Pack
        (None, Some(mut p)) => {
            p.as_object_mut().map(|m| m.remove("pack"));
            p
        }
        (Some(a), Some(mut p)) => {
            p.as_object_mut().map(|m| m.remove("pack"));
            merge_manifest_values(&a, &p)
        }
    };
    let text =
        crate::yaml::to_string(&NativeValue(merged)).map_err(|e| OverlayError::InvalidYaml {
            layer: Layer::Merged,
            message: e.to_string(),
        })?;
    out.insert(manifest_path().into(), (Layer::Merged, text.into_bytes()));
    Ok(finish(out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn y(s: &str) -> Value {
        crate::yaml::from_str(s).unwrap()
    }

    #[test]
    fn a_project_grant_level_replaces_the_packs_whole() {
        let pack = b"policy:\n  grants:\n    keep: { allow: [\"Bash(a)\"] }\n    swap: { description: p, allow: [\"Bash(p)\"], until: s }\n".to_vec();
        let project = b"policy:\n  grants:\n    swap: { allow: [\"Bash(q)\"] }\n    add: { allow: [\"Bash(n)\"] }\n".to_vec();
        let out = overlay(
            &BTreeMap::from([(manifest_path().into(), pack)]),
            &BTreeMap::from([(manifest_path().into(), project)]),
        )
        .unwrap();
        let v = y(std::str::from_utf8(&out[manifest_path()].1).unwrap());
        assert_eq!(
            v,
            json!({"policy": {"grants": {
                "keep": {"allow": ["Bash(a)"]},
                "swap": {"allow": ["Bash(q)"]},
                "add": {"allow": ["Bash(n)"]},
            }}})
        );
    }

    #[test]
    fn lists_are_completed_from_both_sides() {
        let v = merge_manifest_values(
            &json!({"policy": {"free_agents": ["a", "b"]}}),
            &json!({"policy": {"free_agents": ["c", "a"]}}),
        );
        assert_eq!(v, json!({"policy": {"free_agents": ["a", "b", "c"]}}));
    }

    #[test]
    fn project_scalar_wins_and_pack_keys_stay() {
        let v = merge_manifest_values(
            &json!({"coordinator": "lead", "targets": ["claude"]}),
            &json!({"coordinator": "boss"}),
        );
        assert_eq!(v, json!({"coordinator": "boss", "targets": ["claude"]}));
    }

    #[test]
    fn explicit_off_of_the_project_wins() {
        let pack = BTreeMap::from([(
            "project.yaml".into(),
            b"policy:\n  delegation: strict\n".to_vec(),
        )]);
        let project = BTreeMap::from([(
            "project.yaml".into(),
            b"policy:\n  delegation: off\n".to_vec(),
        )]);
        let out = overlay(&pack, &project).unwrap();
        let (layer, bytes) = &out[Path::new("project.yaml")];
        assert_eq!(*layer, Layer::Merged);
        assert_eq!(
            y(std::str::from_utf8(bytes).unwrap()),
            json!({"policy": {"delegation": "off"}})
        );
    }

    #[test]
    fn native_maps_merge_recursively() {
        let pack = BTreeMap::from([(
            "project.yaml".into(),
            b"native:\n  claude:\n    permissions:\n      allow: [x]\n    env:\n      A: \"1\"\n    port: 7\n    b: true\n".to_vec(),
        )]);
        let project = BTreeMap::from([(
            "project.yaml".into(),
            b"native:\n  claude:\n    permissions:\n      allow: [z]\n".to_vec(),
        )]);
        let out = overlay(&pack, &project).unwrap();
        let text = std::str::from_utf8(&out[Path::new("project.yaml")].1).unwrap();
        assert_eq!(
            y(text),
            json!({"native": {"claude": {
                "permissions": {"allow": ["x", "z"]},
                "env": {"A": "1"}, "port": 7, "b": true}}})
        );
        assert!(
            text.contains("port: 7") && text.contains("b: true"),
            "{text}"
        );
    }

    #[test]
    fn files_overlay_by_path() {
        let f = |s: &str| s.as_bytes().to_vec();
        let pack = BTreeMap::from([
            (PathBuf::from("agents/a.md"), f("A1")),
            (PathBuf::from("skills/s/SKILL.md"), f("S1")),
            (PathBuf::from("skills/s/scripts/x.sh"), f("X1")),
        ]);
        let project = BTreeMap::from([
            (PathBuf::from("agents/a.md"), f("A2")),
            (PathBuf::from("agents/b.md"), f("B2")),
        ]);
        let out = overlay(&pack, &project).unwrap();
        assert_eq!(
            out,
            BTreeMap::from([
                (PathBuf::from("agents/a.md"), (Layer::Project, f("A2"))),
                (PathBuf::from("agents/b.md"), (Layer::Project, f("B2"))),
                (PathBuf::from("skills/s/SKILL.md"), (Layer::Pack, f("S1"))),
                (
                    PathBuf::from("skills/s/scripts/x.sh"),
                    (Layer::Pack, f("X1"))
                ),
            ])
        );
    }

    #[test]
    fn the_project_pack_key_is_dropped_and_a_nested_pack_refused() {
        let f = |s: &str| s.as_bytes().to_vec();
        let pack = BTreeMap::from([("project.yaml".into(), f("coordinator: lead\n"))]);
        let project = BTreeMap::from([(
            "project.yaml".into(),
            f("pack:\n  name: demo\n  source: s\n  version: '1'\n"),
        )]);
        let out = overlay(&pack, &project).unwrap();
        let text = std::str::from_utf8(&out[Path::new("project.yaml")].1).unwrap();
        assert_eq!(y(text), json!({"coordinator": "lead"}));
        // no pack side: the project's file alone, still without `pack`
        let out = overlay(&BTreeMap::new(), &project).unwrap();
        assert_eq!(
            out[Path::new("project.yaml")],
            (Layer::Merged, b"{}\n".to_vec())
        );

        let nested = BTreeMap::from([("project.yaml".into(), f("pack:\n  name: x\n"))]);
        assert_eq!(overlay(&nested, &project), Err(OverlayError::NestedPack));
    }

    #[test]
    fn pack_only_manifest_is_kept_and_bad_yaml_is_named() {
        let pack = BTreeMap::from([("project.yaml".into(), b"coordinator: lead\n".to_vec())]);
        let out = overlay(&pack, &BTreeMap::new()).unwrap();
        assert_eq!(out[Path::new("project.yaml")].0, Layer::Pack);
        let bad = BTreeMap::from([("project.yaml".into(), b"a: [\n".to_vec())]);
        assert!(matches!(
            overlay(&pack, &bad),
            Err(OverlayError::InvalidYaml {
                layer: Layer::Project,
                ..
            })
        ));
    }

    fn merged_text(pack: &str, project: &str) -> String {
        let f = |s: &str| BTreeMap::from([(PathBuf::from("project.yaml"), s.as_bytes().to_vec())]);
        let out = overlay(&f(pack), &f(project)).unwrap();
        String::from_utf8(out[Path::new("project.yaml")].1.clone()).unwrap()
    }

    #[test]
    fn empty_or_comment_only_manifest_is_an_empty_mapping() {
        for empty in ["", "# nothing here\n"] {
            assert_eq!(
                y(&merged_text("coordinator: lead\n", empty)),
                json!({"coordinator": "lead"})
            );
        }
    }

    #[test]
    fn a_manifest_that_is_not_a_mapping_is_refused() {
        let f = |s: &str| BTreeMap::from([(PathBuf::from("project.yaml"), s.as_bytes().to_vec())]);
        assert_eq!(
            overlay(&f("coordinator: lead\n"), &f("[a, b]\n")),
            Err(OverlayError::InvalidYaml {
                layer: Layer::Project,
                message: "not a mapping".into()
            })
        );
    }

    #[test]
    fn a_duplicate_within_the_project_list_is_dropped() {
        let v = merge_manifest_values(&json!({"l": ["a"]}), &json!({"l": ["c", "c"]}));
        assert_eq!(v, json!({"l": ["a", "c"]}));
    }

    #[test]
    fn decimal_looking_text_keeps_its_spelling_but_native_numbers_stay_numbers() {
        let text = merged_text(
            "schema_version: 1\nname: 1.10\ntargets: [2.50]\nnative:\n  claude:\n    ratio: 1.5\n",
            "coordinator: 3.0\nextra: 1e3\n",
        );
        assert_eq!(
            text,
            "schema_version: 1\nname: \"1.10\"\ntargets:\n- \"2.50\"\nnative:\n  claude:\n    ratio: 1.5\ncoordinator: \"3.0\"\nextra: \"1e3\"\n"
        );
    }

    fn tree(files: &[(&str, &str)]) -> BTreeMap<PathBuf, Vec<u8>> {
        files
            .iter()
            .map(|(p, b)| (PathBuf::from(p), b.as_bytes().to_vec()))
            .collect()
    }

    #[test]
    fn two_standards_layers_add_their_skills() {
        let a = tree(&[("skills/a-std/SKILL.md", "A")]);
        let b = tree(&[("skills/b-std/SKILL.md", "B")]);
        let st = overlay_standards(None, &[&a, &b], &BTreeMap::new()).unwrap();
        assert_eq!(
            st.files,
            BTreeMap::from([
                (
                    PathBuf::from("skills/a-std/SKILL.md"),
                    (Layer::Standards(0), b"A".to_vec())
                ),
                (
                    PathBuf::from("skills/b-std/SKILL.md"),
                    (Layer::Standards(1), b"B".to_vec())
                ),
            ])
        );
        assert!(st.clashes.is_empty() && st.foreign.is_empty());
    }

    #[test]
    fn a_skill_in_two_standards_is_a_clash() {
        let a = tree(&[("skills/s/SKILL.md", "A"), ("skills/s/x.sh", "AX")]);
        let b = tree(&[("skills/s/SKILL.md", "B"), ("skills/s/y.sh", "BY")]);
        let st = overlay_standards(None, &[&a, &b], &BTreeMap::new()).unwrap();
        assert_eq!(
            st.clashes,
            [SkillClash {
                skill: "s".into(),
                first: Some(0),
                second: 1
            }]
        );
        // the later folder stays out, whole
        assert!(!st.files.contains_key(Path::new("skills/s/y.sh")));
        assert_eq!(st.files[Path::new("skills/s/SKILL.md")].1, b"A");
    }

    #[test]
    fn a_skill_in_the_team_pack_and_a_standards_is_a_clash() {
        let team = tree(&[("skills/s/SKILL.md", "T")]);
        let a = tree(&[("skills/s/SKILL.md", "A")]);
        let st = overlay_standards(Some(&team), &[&a], &BTreeMap::new()).unwrap();
        assert_eq!(
            st.clashes,
            [SkillClash {
                skill: "s".into(),
                first: None,
                second: 0
            }]
        );
        assert_eq!(st.files[Path::new("skills/s/SKILL.md")].0, Layer::Pack);
    }

    #[test]
    fn the_project_skill_lifts_the_clash_and_takes_the_whole_folder() {
        let a = tree(&[("skills/s/SKILL.md", "A"), ("skills/s/scripts/x.sh", "AX")]);
        let b = tree(&[("skills/s/SKILL.md", "B")]);
        let project = tree(&[("skills/s/SKILL.md", "P")]);
        let st = overlay_standards(None, &[&a, &b], &project).unwrap();
        assert!(st.clashes.is_empty());
        assert_eq!(
            st.files,
            BTreeMap::from([(
                PathBuf::from("skills/s/SKILL.md"),
                (Layer::Project, b"P".to_vec())
            )])
        );
    }

    #[test]
    fn a_standards_layer_keeps_only_skills() {
        let a = tree(&[
            ("agents/a.md", "AG"),
            ("project.yaml", "coordinator: x\n"),
            ("skills/s/SKILL.md", "S"),
        ]);
        let st = overlay_standards(None, &[&a], &BTreeMap::new()).unwrap();
        assert_eq!(
            st.foreign,
            [
                (0, PathBuf::from("agents/a.md")),
                (0, PathBuf::from("project.yaml"))
            ]
        );
        assert_eq!(
            st.files.keys().collect::<Vec<_>>(),
            [Path::new("skills/s/SKILL.md")]
        );
    }

    #[test]
    fn a_team_pack_with_standards_is_refused() {
        let team = tree(&[(
            "project.yaml",
            "standards:\n- {name: x, source: s, version: '1'}\n",
        )]);
        assert_eq!(
            overlay_standards(Some(&team), &[], &BTreeMap::new()),
            Err(OverlayError::NestedStandards)
        );
    }

    #[test]
    fn the_merged_manifest_keeps_standards_and_drops_pack() {
        let project = tree(&[(
            "project.yaml",
            "pack: {name: t, source: s1, version: '1'}\nstandards:\n- {name: x, source: s2, version: '2'}\n",
        )]);
        let st = overlay_standards(None, &[], &project).unwrap();
        let (layer, bytes) = &st.files[Path::new("project.yaml")];
        assert_eq!(*layer, Layer::Merged);
        assert_eq!(
            y(std::str::from_utf8(bytes).unwrap()),
            json!({"standards": [{"name": "x", "source": "s2", "version": "2"}]})
        );
    }
}
