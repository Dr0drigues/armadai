//! `{{var.name}}` substitution of pack variables, applied by `link` to the effective
//! `.armadai/` (never to the versioned files). Pure: the values arrive already
//! resolved (environment, `vars.local.yaml`, `vars.yaml` are read by the caller).
//!
//! `{{var.name}}` or `{{ var.name }}` with `name` a valid id is a variable; `\{{` is a
//! literal `{{`; anything else (`{{var.Bad Name}}`, `{{name}}`, which belongs to agent
//! templates) is kept as is. A value is inserted verbatim: never rendered again.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

use serde_json::Value;

use crate::types::NativeValue;

/// The environment variable read for `{{var.<name>}}`: `ARMADAI_VAR_` then the name
/// in capitals, `-` written `_` (`home-dir` -> `ARMADAI_VAR_HOME_DIR`).
pub fn env_name(name: &str) -> String {
    format!(
        "ARMADAI_VAR_{}",
        name.to_ascii_uppercase().replace('-', "_")
    )
}

/// `text` with every variable name handed to `var` (which writes its replacement).
fn scan<'a>(text: &'a str, mut var: impl FnMut(&'a str, &mut String)) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find("{{") {
        let after = &rest[i + 2..];
        if rest[..i].ends_with('\\') {
            out.push_str(&rest[..i - 1]);
            out.push_str("{{");
            rest = after;
            continue;
        }
        out.push_str(&rest[..i]);
        let name = after.find("}}").and_then(|j| {
            let name = after[..j].trim_matches(' ').strip_prefix("var.")?;
            crate::is_valid_id(name).then_some((name, j))
        });
        match name {
            Some((name, j)) => {
                var(name, &mut out);
                rest = &after[j + 2..];
            }
            None => {
                out.push_str("{{");
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// The names of the `{{var.<name>}}` / `{{ var.<name> }}` of `text` (`name` an id).
pub fn names(text: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    scan(text, |name, _| {
        found.insert(name.to_string());
    });
    found
}

/// `text` with each variable replaced by its value, verbatim (never rendered again);
/// a name `values` lacks becomes the empty string and is returned in the set.
pub fn render(text: &str, values: &BTreeMap<String, String>) -> (String, BTreeSet<String>) {
    let mut missing = BTreeSet::new();
    let out = scan(text, |name, out| match values.get(name) {
        Some(v) => out.push_str(v),
        None => {
            missing.insert(name.to_string());
        }
    });
    (out, missing)
}

/// Every string of `v`, nested ones included, rendered in place; keys and every other
/// JSON type (number, bool, null) untouched.
pub fn render_value(
    v: &mut Value,
    values: &BTreeMap<String, String>,
    missing: &mut BTreeSet<String>,
) {
    match v {
        Value::String(s) => {
            let (r, m) = render(s, values);
            *s = r;
            missing.extend(m);
        }
        Value::Array(a) => a.iter_mut().for_each(|v| render_value(v, values, missing)),
        Value::Object(m) => m
            .values_mut()
            .for_each(|v| render_value(v, values, missing)),
        _ => {}
    }
}

fn value_names(v: &Value, out: &mut BTreeSet<String>) {
    match v {
        Value::String(s) => out.extend(names(s)),
        Value::Array(a) => a.iter().for_each(|v| value_names(v, out)),
        Value::Object(m) => m.values().for_each(|v| value_names(v, out)),
        _ => {}
    }
}

/// What `render_files` did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Rendered {
    /// Every variable name met.
    pub used: BTreeSet<String>,
    /// The used names `values` lacks (rendered as empty).
    pub missing: BTreeSet<String>,
}

enum Scope {
    Manifest,
    Body,
}

/// The part of a file (path relative to `.armadai/`) that is rendered.
fn scope(path: &Path) -> Option<Scope> {
    let parts: Vec<_> = path
        .components()
        .map(|c| match c {
            Component::Normal(s) => s.to_str(),
            _ => None,
        })
        .collect::<Option<_>>()?;
    match parts.as_slice() {
        ["project.yaml"] => Some(Scope::Manifest),
        ["agents" | "templates" | "commands", f] if f.ends_with(".md") => Some(Scope::Body),
        ["skills", _, "SKILL.md"] => Some(Scope::Body),
        _ => None,
    }
}

fn text<'a>(path: &Path, bytes: &'a [u8]) -> Result<&'a str, String> {
    std::str::from_utf8(bytes).map_err(|e| format!("{}: {e}", path.display()))
}

/// The byte offset where the body starts (after the frontmatter; 0 without one).
fn body_start(text: &str) -> usize {
    crate::load::split_frontmatter(text).map_or(0, |(_, body, _)| text.len() - body.len())
}

fn manifest(path: &Path, text: &str) -> Result<Value, String> {
    let v: Value = crate::yaml::from_str(text).map_err(|e| format!("{}: {e}", path.display()))?;
    match v {
        Value::Null => Ok(Value::Object(Default::default())),
        v @ Value::Object(_) => Ok(v),
        _ => Err(format!("{}: not a mapping", path.display())),
    }
}

/// The names used by the files `render_files` renders.
pub fn files_names(files: &BTreeMap<PathBuf, Vec<u8>>) -> Result<BTreeSet<String>, String> {
    let mut out = BTreeSet::new();
    for (p, b) in files {
        let Some(s) = scope(p) else { continue };
        let t = text(p, b)?;
        match s {
            Scope::Body => out.extend(names(&t[body_start(t)..])),
            Scope::Manifest => {
                if let Some(n) = manifest(p, t)?.get("native") {
                    value_names(n, &mut out);
                }
            }
        }
    }
    Ok(out)
}

/// Render, in place: the string values under `native` of `project.yaml` (a file with no
/// variable keeps its bytes), and the body (after the frontmatter) of `agents/*.md`,
/// `templates/*.md`, `commands/*.md` and `skills/*/SKILL.md`. Frontmatters and every
/// other file are left as they are.
pub fn render_files(
    files: &mut BTreeMap<PathBuf, Vec<u8>>,
    values: &BTreeMap<String, String>,
) -> Result<Rendered, String> {
    let mut done = Rendered::default();
    for (p, b) in files.iter_mut() {
        let Some(s) = scope(p) else { continue };
        let t = text(p, b)?;
        let new = match s {
            Scope::Body => {
                let at = body_start(t);
                let used = names(&t[at..]);
                if used.is_empty() {
                    continue;
                }
                let (body, missing) = render(&t[at..], values);
                done.used.extend(used);
                done.missing.extend(missing);
                format!("{}{body}", &t[..at])
            }
            Scope::Manifest => {
                let mut v = manifest(p, t)?;
                let mut used = BTreeSet::new();
                let Some(native) = v.get_mut("native") else {
                    continue;
                };
                value_names(native, &mut used);
                if used.is_empty() {
                    continue;
                }
                render_value(native, values, &mut done.missing);
                done.used.extend(used);
                crate::yaml::to_string(&NativeValue(v))
                    .map_err(|e| format!("{}: {e}", p.display()))?
            }
        };
        *b = new.into_bytes();
    }
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn vals(kv: &[(&str, &str)]) -> BTreeMap<String, String> {
        kv.iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn set(n: &[&str]) -> BTreeSet<String> {
        n.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn both_spellings_and_missing_names() {
        assert_eq!(
            render("{{var.a}} {{ var.a }} {{var.b}}", &vals(&[("a", "X")])),
            ("X X ".to_string(), set(&["b"]))
        );
    }

    #[test]
    fn a_value_is_never_rendered_again() {
        let (out, missing) = render("{{var.a}}", &vals(&[("a", "{{var.b}}"), ("b", "Y")]));
        assert_eq!(out, "{{var.b}}");
        assert!(missing.is_empty());
    }

    #[test]
    fn escaped_and_malformed_stay_literal() {
        for t in [
            r"\{{var.a}}",
            "{{var.A}}",
            "{{var.}}",
            "{{vara}}",
            "{{var.a",
        ] {
            assert_eq!(render(t, &vals(&[("a", "X")])).0, t.replace(r"\{{", "{{"));
            assert!(names(t).is_empty(), "{t}");
        }
        assert_eq!(render(r"\{{var.a}}", &vals(&[])).0, "{{var.a}}");
    }

    #[test]
    fn agent_template_vars_are_not_pack_vars() {
        assert_eq!(render("{{name}}", &vals(&[("name", "X")])).0, "{{name}}");
        assert_eq!(
            crate::agent_templates::render("{{var.x}}", &BTreeMap::new()),
            Ok("{{var.x}}".to_string())
        );
    }

    #[test]
    fn render_value_touches_strings_only() {
        let mut v = json!({"p": ["{{var.a}}", 7, true, null, {"k": "{{var.a}}"}], "{{var.a}}": 1});
        let mut missing = BTreeSet::new();
        render_value(&mut v, &vals(&[("a", "X")]), &mut missing);
        assert_eq!(
            v,
            json!({"p": ["X", 7, true, null, {"k": "X"}], "{{var.a}}": 1})
        );
        assert!(missing.is_empty());
    }

    fn files(kv: &[(&str, &str)]) -> BTreeMap<PathBuf, Vec<u8>> {
        kv.iter()
            .map(|(k, v)| (PathBuf::from(k), v.as_bytes().to_vec()))
            .collect()
    }

    #[test]
    fn render_files_scope() {
        let agent = "---\nname: x\ndescription: {{var.a}}\n---\nbody {{var.a}} {{var.z}}\n";
        let mut f = files(&[
            ("agents/x.md", agent),
            ("skills/s/SKILL.md", "---\nname: s\n---\n{{var.a}}"),
            ("skills/s/scripts/run.sh", "echo {{var.a}}"),
            ("native/claude/x.md", "{{var.a}}"),
            (
                "project.yaml",
                "name: \"{{var.a}}\"\nnative:\n  claude:\n    x: \"{{var.a}}\"\n    n: 3\n",
            ),
        ]);
        let before = f.clone();
        let r = render_files(&mut f, &vals(&[("a", "X")])).unwrap();
        assert_eq!(r.used, set(&["a", "z"]));
        assert_eq!(r.missing, set(&["z"]));
        assert_eq!(
            f[Path::new("agents/x.md")],
            b"---\nname: x\ndescription: {{var.a}}\n---\nbody X \n"
        );
        assert_eq!(f[Path::new("skills/s/SKILL.md")], b"---\nname: s\n---\nX");
        for same in ["skills/s/scripts/run.sh", "native/claude/x.md"] {
            assert_eq!(f[Path::new(same)], before[Path::new(same)], "{same}");
        }
        let m: Value =
            crate::yaml::from_str(std::str::from_utf8(&f[Path::new("project.yaml")]).unwrap())
                .unwrap();
        assert_eq!(
            m,
            json!({"name": "{{var.a}}", "native": {"claude": {"x": "X", "n": 3}}})
        );
        assert_eq!(files_names(&before).unwrap(), set(&["a", "z"]));
    }

    #[test]
    fn a_project_file_without_variable_keeps_its_bytes() {
        let src = "# keep me\nname:   x\nnative:\n  claude: {a: 1}\n";
        let mut f = files(&[("project.yaml", src)]);
        let r = render_files(&mut f, &vals(&[])).unwrap();
        assert_eq!(r, Rendered::default());
        assert_eq!(f[Path::new("project.yaml")], src.as_bytes());
    }

    #[test]
    fn unreadable_files_are_errors() {
        let mut f = files(&[("project.yaml", "native: [")]);
        assert!(render_files(&mut f, &vals(&[])).is_err());
        assert!(files_names(&f).is_err());
    }

    #[test]
    fn env_name_maps_dashes() {
        assert_eq!(env_name("home-dir"), "ARMADAI_VAR_HOME_DIR");
        assert_eq!(env_name("x1"), "ARMADAI_VAR_X1");
    }
}
