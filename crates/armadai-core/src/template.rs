//! `{{var}}` substitution, shared by agent templates and prompt fragments.
//!
//! Extracted from `cli/new.rs`, which substituted inline and hand-rolled its
//! own leftover scan.
//!
//! Two policies over one scan. `render` refuses a leftover: a prompt
//! containing a literal `{{module}}` is text the model will try to interpret,
//! and an agent built from it is quietly wrong. `render_lenient` substitutes
//! what it can and hands the leftovers back, which is what `armadai new`
//! needs — most templates carry `{{description}}`, and `new` warns rather
//! than refusing.

use std::collections::BTreeMap;

/// A template could not be fully rendered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TemplateError {
    /// Placeholder names with no value supplied, in order of appearance.
    Unsubstituted(Vec<String>),
}

impl std::fmt::Display for TemplateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsubstituted(names) => write!(
                f,
                "no value supplied for {}",
                names
                    .iter()
                    .map(|n| format!("{{{{{n}}}}}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }
}

impl std::error::Error for TemplateError {}

/// Every `{{name}}` placeholder in `template`, in order, without duplicates.
fn placeholders(template: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            break; // unterminated: not a placeholder
        };
        let name = after[..end].trim().to_string();
        if !name.is_empty() && !found.contains(&name) {
            found.push(name);
        }
        rest = &after[end + 2..];
    }
    found
}

/// Substitute every `{{name}}` with its value.
///
/// Errors when a placeholder has no value: see the module docs.
pub fn render(template: &str, vars: &BTreeMap<String, String>) -> Result<String, TemplateError> {
    let missing: Vec<String> = placeholders(template)
        .into_iter()
        .filter(|n| !vars.contains_key(n))
        .collect();
    if !missing.is_empty() {
        return Err(TemplateError::Unsubstituted(missing));
    }
    let mut out = template.to_string();
    for (name, value) in vars {
        out = out.replace(&format!("{{{{{name}}}}}"), value);
    }
    Ok(out)
}

/// Substitute what the variables cover, and report what is left.
///
/// For callers that must not fail on a leftover — `armadai new` warns about
/// them instead, because most templates legitimately carry a `{{description}}`
/// the user did not supply.
pub fn render_lenient(template: &str, vars: &BTreeMap<String, String>) -> (String, Vec<String>) {
    let left: Vec<String> = placeholders(template)
        .into_iter()
        .filter(|n| !vars.contains_key(n))
        .collect();
    let mut out = template.to_string();
    for (name, value) in vars {
        out = out.replace(&format!("{{{{{name}}}}}"), value);
    }
    (out, left)
}

/// Values supplied that the template never uses — almost always a typo.
pub fn unused_vars(template: &str, vars: &BTreeMap<String, String>) -> Vec<String> {
    let used = placeholders(template);
    vars.keys().filter(|k| !used.contains(k)).cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn substitutes_every_occurrence() {
        let out = render(
            "{{a}} then {{b}} then {{a}}",
            &vars(&[("a", "1"), ("b", "2")]),
        )
        .unwrap();
        assert_eq!(out, "1 then 2 then 1");
    }

    #[test]
    fn a_template_without_variables_is_returned_as_is() {
        assert_eq!(render("plain text", &vars(&[])).unwrap(), "plain text");
    }

    /// A prompt still containing `{{module}}` is text the model will try to
    /// interpret. Refusing to build the agent is the whole point.
    #[test]
    fn an_unsubstituted_variable_is_an_error_naming_it() {
        let err = render("hello {{who}} and {{whom}}", &vars(&[])).unwrap_err();
        match err {
            TemplateError::Unsubstituted(names) => {
                assert_eq!(names, vec!["who".to_string(), "whom".to_string()])
            }
        }
    }

    #[test]
    fn a_provided_but_unused_variable_is_reported_not_fatal() {
        // Almost always a typo — worth saying, never worth failing.
        assert!(render("nothing here", &vars(&[("spare", "x")])).is_ok());
        assert_eq!(
            unused_vars("nothing here", &vars(&[("spare", "x")])),
            vec!["spare".to_string()]
        );
    }

    /// `armadai new` must keep working on the 10 templates that carry
    /// `{{description}}`: substitute what we have, hand back the rest.
    #[test]
    fn lenient_rendering_substitutes_what_it_can_and_returns_the_rest() {
        let (out, left) = render_lenient("{{a}} and {{b}}", &vars(&[("a", "1")]));
        assert_eq!(out, "1 and {{b}}");
        assert_eq!(left, vec!["b".to_string()]);
    }

    #[test]
    fn lenient_rendering_reports_nothing_left_when_all_are_supplied() {
        let (out, left) = render_lenient("{{a}}", &vars(&[("a", "1")]));
        assert_eq!(out, "1");
        assert!(left.is_empty());
    }

    #[test]
    fn an_unterminated_placeholder_is_left_alone() {
        // `{{` with no closing `}}` is not a placeholder, just text.
        assert_eq!(render("a {{ b", &vars(&[])).unwrap(), "a {{ b");
    }
}
