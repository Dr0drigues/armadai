//! `{{var}}` substitution of agent templates (`templates/<id>.md`).
//!
//! `{{name}}` or `{{ name }}` with `name` a valid id is a variable; `\{{` is a literal
//! `{{`; anything else between braces (`{{Bad Name}}`) is kept as is. A value is
//! inserted verbatim: it is never rendered again.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

/// `text` with every variable handed to `var` (which writes its replacement).
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
        match after.find("}}") {
            Some(j) if crate::is_valid_id(after[..j].trim_matches(' ')) => {
                var(after[..j].trim_matches(' '), &mut out);
                rest = &after[j + 2..];
            }
            _ => {
                out.push_str("{{");
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// `text` rendered with `vars`, or the names of the missing variables (sorted).
pub fn render(text: &str, vars: &BTreeMap<String, String>) -> Result<String, Vec<String>> {
    let mut missing = BTreeSet::new();
    let out = scan(text, |name, out| match vars.get(name) {
        Some(v) => out.push_str(v),
        None => {
            missing.insert(name.to_string());
        }
    });
    if missing.is_empty() {
        Ok(out)
    } else {
        Err(missing.into_iter().collect())
    }
}

/// The variables `text` uses.
pub fn variables(text: &str) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    scan(text, |name, _| {
        names.insert(name.to_string());
    });
    names
}

/// Render every string of `v` in place (values only, never keys); missing names are
/// added to `missing`.
pub(crate) fn render_value(
    v: &mut Value,
    vars: &BTreeMap<String, String>,
    missing: &mut BTreeSet<String>,
) {
    match v {
        Value::String(s) => match render(s, vars) {
            Ok(r) => *s = r,
            Err(m) => missing.extend(m),
        },
        Value::Array(a) => a.iter_mut().for_each(|v| render_value(v, vars, missing)),
        Value::Object(m) => m.values_mut().for_each(|v| render_value(v, vars, missing)),
        _ => {}
    }
}

/// Add the variables of every string of `v` to `out`.
pub fn value_variables(v: &Value, out: &mut BTreeSet<String>) {
    match v {
        Value::String(s) => out.extend(variables(s)),
        Value::Array(a) => a.iter().for_each(|v| value_variables(v, out)),
        Value::Object(m) => m.values().for_each(|v| value_variables(v, out)),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(kv: &[(&str, &str)]) -> BTreeMap<String, String> {
        kv.iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn both_spellings_are_replaced() {
        assert_eq!(
            render("{{a}} and {{ a }}.", &vars(&[("a", "x")])),
            Ok("x and x.".to_string())
        );
    }

    #[test]
    fn escaped_braces_are_literal_and_not_a_variable() {
        assert_eq!(
            render(r"\{{a}} {{a}}", &vars(&[("a", "x")])),
            Ok("{{a}} x".to_string())
        );
        assert_eq!(variables(r"\{{a}}"), BTreeSet::new());
    }

    #[test]
    fn a_value_is_inserted_verbatim_never_rendered_again() {
        assert_eq!(
            render("{{a}}", &vars(&[("a", "x {{b}}"), ("b", "no")])),
            Ok("x {{b}}".to_string())
        );
    }

    #[test]
    fn invalid_name_is_kept_and_not_a_variable() {
        assert_eq!(
            render("{{Bad Name}}", &vars(&[])),
            Ok("{{Bad Name}}".to_string())
        );
        assert_eq!(
            variables("{{Bad Name}} {{ok}}"),
            BTreeSet::from(["ok".to_string()])
        );
    }

    #[test]
    fn missing_variable_is_named() {
        assert_eq!(
            render("{{a}} {{b}} {{a}}", &vars(&[("b", "x")])),
            Err(vec!["a".to_string()])
        );
    }

    #[test]
    fn strings_in_lists_and_maps_are_rendered_keys_are_not() {
        let mut v = serde_json::json!({ "{{a}}": ["{{a}}", { "k": "{{ a }}" }, 1] });
        let mut missing = BTreeSet::new();
        render_value(&mut v, &vars(&[("a", "x")]), &mut missing);
        assert_eq!(v, serde_json::json!({ "{{a}}": ["x", { "k": "x" }, 1] }));
        assert_eq!(missing, BTreeSet::new());
    }
}
