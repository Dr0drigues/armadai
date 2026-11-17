//! Claude Code's registry of marketplaces (`plugins/known_marketplaces.json`), read to warn
//! when its `armadai-ui` entry leads away from this project's plugin.
//!
//! Format measured on Claude Code with `claude plugin marketplace add <dir>` in a scratch
//! `CLAUDE_CONFIG_DIR` (plan lot 14, section 7): an object keyed by marketplace name, each
//! entry `{"source": {"source": "directory", "path": <abs>}, "installLocation": <abs>,
//! "lastUpdated": ...}`; a relative argument is stored resolved. Plugins root (docs,
//! `https://code.claude.com/docs/en/plugins/loading.md`): `~/.claude/plugins` unless
//! `CLAUDE_CODE_PLUGIN_CACHE_DIR`. Anything that does not have this shape gives no warning.

use std::path::{Path, PathBuf};

use serde_json::Value;

/// `$CLAUDE_CODE_PLUGIN_CACHE_DIR`, else `$CLAUDE_CONFIG_DIR/plugins`, else
/// `$HOME/.claude/plugins`, joined with `known_marketplaces.json`; `None` without any.
pub fn registry_file(env: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let set = |k: &str| env(k).filter(|d| !d.is_empty()).map(PathBuf::from);
    let plugins = set("CLAUDE_CODE_PLUGIN_CACHE_DIR")
        .or_else(|| set("CLAUDE_CONFIG_DIR").map(|d| d.join("plugins")))
        .or_else(|| set("HOME").map(|d| d.join(".claude").join("plugins")))?;
    Some(plugins.join("known_marketplaces.json"))
}

/// The reason of the warning line, or `None`: unexpected registry, no `armadai-ui`
/// entry, a relative location, or a location that is this project's plugin.
/// `registry` is the file `text` was read from; `project_plugin` is this project's
/// `.armadai/plugins/armadai-ui` with its `.armadai` already resolved.
pub fn armadai_ui_warning(registry: &Path, text: &str, project_plugin: &Path) -> Option<String> {
    let value: Value = serde_json::from_str(text).ok()?;
    let entry = value.as_object()?.get("armadai-ui")?.as_object()?;
    let source = entry.get("source")?.as_object()?;
    let kind = source.get("source")?.as_str()?;
    let known = registry.display();
    let fix = "run claude plugin marketplace remove armadai-ui, then start claude in this \
               project to register its own";
    if kind != "directory" && kind != "file" {
        return Some(format!(
            "the armadai-ui marketplace that Claude Code knows ({known}) comes from a {kind} \
             source, not from this project's .armadai/plugins/armadai-ui; {fix}"
        ));
    }
    let location = entry
        .get("installLocation")
        .or_else(|| source.get("path"))?
        .as_str()?;
    let location = Path::new(location);
    if !location.is_absolute() {
        return None;
    }
    if location == project_plugin {
        return None;
    }
    match location.canonicalize() {
        Err(_) => Some(format!(
            "the armadai-ui marketplace that Claude Code knows ({known}) leads to {}, which \
             does not exist, so this project's plugin may not load; {fix}",
            location.display()
        )),
        Ok(found) if project_plugin.canonicalize().is_ok_and(|p| p == found) => None,
        Ok(_) => Some(format!(
            "the armadai-ui marketplace that Claude Code knows ({known}) leads to {}, not to \
             this project's .armadai/plugins/armadai-ui; {fix}",
            location.display()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let v: Vec<(String, String)> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |k| v.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone())
    }

    #[test]
    fn the_registry_is_found_by_cache_dir_then_config_dir_then_home() {
        let all = [
            ("CLAUDE_CODE_PLUGIN_CACHE_DIR", "/cache"),
            ("CLAUDE_CONFIG_DIR", "/cfg"),
            ("HOME", "/home/u"),
        ];
        let file = |pairs: &[(&str, &str)]| registry_file(&env_of(pairs));
        let name = "known_marketplaces.json";
        assert_eq!(file(&all), Some(PathBuf::from("/cache").join(name)));
        assert_eq!(
            file(&all[1..]),
            Some(PathBuf::from("/cfg/plugins").join(name))
        );
        assert_eq!(
            file(&all[2..]),
            Some(PathBuf::from("/home/u/.claude/plugins").join(name))
        );
        assert_eq!(file(&[("CLAUDE_CONFIG_DIR", "")]), None);
    }

    fn entry(source: Value, location: &Path) -> String {
        serde_json::json!({"armadai-ui": {
            "source": source, "installLocation": location, "lastUpdated": "x"}})
        .to_string()
    }

    fn dir(path: &Path) -> Value {
        serde_json::json!({"source": "directory", "path": path})
    }

    #[test]
    fn a_registry_that_leads_elsewhere_is_named_and_the_project_plugin_is_not() {
        let tmp = tempfile::tempdir().unwrap();
        let t = tmp.path().canonicalize().unwrap();
        let (mine, other, gone) = (t.join("mine"), t.join("other"), t.join("gone"));
        std::fs::create_dir_all(&mine).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        let reg = t.join("known_marketplaces.json");
        let fix = "run claude plugin marketplace remove armadai-ui, then start claude in this \
                   project to register its own";
        let warn = |text: &str| armadai_ui_warning(&reg, text, &mine);

        assert_eq!(
            warn(&entry(dir(&gone), &gone)),
            Some(format!(
                "the armadai-ui marketplace that Claude Code knows ({}) leads to {}, which \
                 does not exist, so this project's plugin may not load; {fix}",
                reg.display(),
                gone.display()
            ))
        );
        assert_eq!(
            warn(&entry(dir(&other), &other)),
            Some(format!(
                "the armadai-ui marketplace that Claude Code knows ({}) leads to {}, not to \
                 this project's .armadai/plugins/armadai-ui; {fix}",
                reg.display(),
                other.display()
            ))
        );
        let github = serde_json::json!({"source": "github", "repo": "a/b"});
        assert_eq!(
            warn(&entry(github, &other)),
            Some(format!(
                "the armadai-ui marketplace that Claude Code knows ({}) comes from a github \
                 source, not from this project's .armadai/plugins/armadai-ui; {fix}",
                reg.display()
            ))
        );
        // the project's plugin, and a link that reaches it
        assert_eq!(warn(&entry(dir(&mine), &mine)), None);
        let via = t.join("via");
        std::os::unix::fs::symlink(&mine, &via).unwrap();
        assert_eq!(warn(&entry(dir(&via), &via)), None, "a link to the plugin");
        // not yet written: the registry already names the path link is about to write
        let later = t.join("later");
        assert_eq!(
            armadai_ui_warning(&reg, &entry(dir(&later), &later), &later),
            None
        );
    }

    #[test]
    fn a_registry_of_an_unexpected_shape_gives_no_warning() {
        let tmp = tempfile::tempdir().unwrap();
        let t = tmp.path().canonicalize().unwrap();
        let reg = t.join("known_marketplaces.json");
        let warn = |text: &str| armadai_ui_warning(&reg, text, &t.join("mine"));
        let rel = entry(dir(Path::new("old")), Path::new("old"));
        for text in [
            "not json",
            "[]",
            "{}",
            r#"{"other": {"installLocation": "/x"}}"#,
            r#"{"armadai-ui": []}"#,
            r#"{"armadai-ui": {"installLocation": "/x"}}"#,
            r#"{"armadai-ui": {"source": {"source": "directory"}}}"#,
            &rel,
        ] {
            assert_eq!(warn(text), None, "{text}");
        }
    }
}
