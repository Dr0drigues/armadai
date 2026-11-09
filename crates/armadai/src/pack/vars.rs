//! The three layers of `{{var.name}}` values: `vars.yaml` (versioned), overridden by
//! `vars.local.yaml` (not versioned), overridden by `ARMADAI_VAR_<NAME>`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::bail;

pub const SHARED: &str = ".armadai/vars.yaml";
pub const LOCAL: &str = ".armadai/vars.local.yaml";

/// One vars file as name -> value; absent is empty, a bad key or bad YAML names the file.
fn read(root: &Path, file: &str) -> anyhow::Result<BTreeMap<String, String>> {
    let text = match std::fs::read_to_string(root.join(file)) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(e) => bail!("{file}: {e}"),
    };
    if text.trim().is_empty() {
        return Ok(BTreeMap::new());
    }
    let map: BTreeMap<String, String> =
        armadai_model::yaml::from_str(&text).map_err(|e| anyhow::anyhow!("{file}: {e}"))?;
    if let Some(key) = map.keys().find(|k| !armadai_model::is_valid_id(k)) {
        bail!("{file}: `{key}` is not a valid variable name (ids match [a-z0-9][a-z0-9-]{{0,63}})");
    }
    Ok(map)
}

/// The value of each of `names` that a layer sets: `vars.yaml`, overridden by
/// `vars.local.yaml`, overridden by `ARMADAI_VAR_<NAME>` read through `env`. A name no
/// layer sets is absent. Both files are read (and checked) even when no name is asked.
pub fn values(
    root: &Path,
    names: &BTreeSet<String>,
    env: &dyn Fn(&str) -> Option<String>,
) -> anyhow::Result<BTreeMap<String, String>> {
    let mut all = read(root, SHARED)?;
    all.extend(read(root, LOCAL)?);
    let mut out = BTreeMap::new();
    for n in names {
        if let Some(v) = env(&armadai_model::pack_vars::env_name(n)).or_else(|| all.remove(n)) {
            out.insert(n.clone(), v);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(shared: Option<&str>, local: Option<&str>) -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join(".armadai")).unwrap();
        for (f, c) in [(SHARED, shared), (LOCAL, local)] {
            if let Some(c) = c {
                std::fs::write(d.path().join(f), c).unwrap();
            }
        }
        d
    }

    fn names() -> BTreeSet<String> {
        ["a", "b", "c", "d"].map(String::from).into()
    }

    fn got(d: &tempfile::TempDir, env: &[(&str, &str)]) -> BTreeMap<String, String> {
        let env: BTreeMap<String, String> = env
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        values(d.path(), &names(), &|k| env.get(k).cloned()).unwrap()
    }

    fn map(kv: &[(&str, &str)]) -> BTreeMap<String, String> {
        kv.iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn layers_override_in_order() {
        let shared = "a: a-shared\nb: b-shared\nc: c-shared\n";
        let local = "a: a-local\nb: b-local\n";
        let env = [("ARMADAI_VAR_A", "a-env")];
        // every combination of the three layers
        let d = project(Some(shared), Some(local));
        assert_eq!(
            got(&d, &env),
            map(&[("a", "a-env"), ("b", "b-local"), ("c", "c-shared")])
        );
        assert_eq!(
            got(&d, &[]),
            map(&[("a", "a-local"), ("b", "b-local"), ("c", "c-shared")])
        );
        let d = project(Some(shared), None);
        assert_eq!(
            got(&d, &env),
            map(&[("a", "a-env"), ("b", "b-shared"), ("c", "c-shared")])
        );
        assert_eq!(
            got(&d, &[]),
            map(&[("a", "a-shared"), ("b", "b-shared"), ("c", "c-shared")])
        );
        let d = project(None, Some(local));
        assert_eq!(got(&d, &env), map(&[("a", "a-env"), ("b", "b-local")]));
        assert_eq!(got(&d, &[]), map(&[("a", "a-local"), ("b", "b-local")]));
        let d = project(None, None);
        assert_eq!(got(&d, &env), map(&[("a", "a-env")]));
        assert_eq!(got(&d, &[]), map(&[]));
    }

    #[test]
    fn a_dash_in_a_name_is_an_underscore_in_the_environment() {
        let d = project(None, None);
        let names: BTreeSet<String> = ["home-dir".to_string()].into();
        let v = values(d.path(), &names, &|k| {
            (k == "ARMADAI_VAR_HOME_DIR").then(|| "/h".to_string())
        })
        .unwrap();
        assert_eq!(v, map(&[("home-dir", "/h")]));
    }

    #[test]
    fn a_bad_vars_file_names_itself() {
        let d = project(Some("a: x\n"), Some("Bad: x\n"));
        let e = values(d.path(), &names(), &|_| None).unwrap_err();
        assert_eq!(
            e.to_string(),
            ".armadai/vars.local.yaml: `Bad` is not a valid variable name (ids match [a-z0-9][a-z0-9-]{0,63})"
        );
        let d = project(Some("a: [1\n"), None);
        let e = values(d.path(), &names(), &|_| None).unwrap_err();
        assert!(e.to_string().starts_with(".armadai/vars.yaml: "), "{e}");
    }
}
