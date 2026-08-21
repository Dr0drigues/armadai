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
//!
//! `placeholders()` and substitution share one scan (see `scan()` below), so
//! they cannot disagree on which occurrences a template contains — including
//! `{{ name }}` written with inner whitespace, which is trimmed and treated
//! exactly like `{{name}}` by both the missing-value check and the
//! substitution itself. A value is inserted literally and is never rescanned
//! for placeholders of its own, so substitution is single-pass and
//! order-independent.

use std::collections::BTreeMap;
use std::ops::Range;

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

/// One `{{name}}` occurrence, with the byte range of the whole `{{...}}` span
/// (inner whitespace included) so a caller can replace exactly that text.
struct Occurrence {
    name: String,
    span: Range<usize>,
}

/// Scan `template` once for `{{name}}` placeholders, trimming inner
/// whitespace so `{{ name }}` and `{{name}}` are the same placeholder. An
/// unterminated `{{` (no matching `}}` anywhere after it) is not a
/// placeholder and stops the scan, matching the pre-existing "left alone"
/// behaviour for trailing garbage.
fn scan(template: &str) -> Vec<Occurrence> {
    let mut found = Vec::new();
    let mut pos = 0;
    while let Some(rel_start) = template[pos..].find("{{") {
        let start = pos + rel_start;
        let after = start + 2;
        let Some(rel_end) = template[after..].find("}}") else {
            break; // unterminated: not a placeholder
        };
        let inner_end = after + rel_end;
        let end = inner_end + 2;
        let name = template[after..inner_end].trim().to_string();
        if !name.is_empty() {
            found.push(Occurrence {
                name,
                span: start..end,
            });
        }
        pos = end;
    }
    found
}

/// Every `{{name}}` placeholder in `template`, in order, without duplicates.
fn placeholders(template: &str) -> Vec<String> {
    let mut found = Vec::new();
    for occurrence in scan(template) {
        if !found.contains(&occurrence.name) {
            found.push(occurrence.name);
        }
    }
    found
}

/// Replace every recognised occurrence that has a value, in one pass over
/// `template`. An occurrence with no value is copied through verbatim
/// (original spacing included), so a caller can inspect what is still
/// missing without the output silently losing that placeholder's text.
fn substitute(template: &str, vars: &BTreeMap<String, String>) -> String {
    let mut out = String::with_capacity(template.len());
    let mut last = 0;
    for occurrence in scan(template) {
        out.push_str(&template[last..occurrence.span.start]);
        match vars.get(&occurrence.name) {
            Some(value) => out.push_str(value),
            None => out.push_str(&template[occurrence.span.clone()]),
        }
        last = occurrence.span.end;
    }
    out.push_str(&template[last..]);
    out
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
    Ok(substitute(template, vars))
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
    (substitute(template, vars), left)
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

    /// C1: `{{ name }}` (spaced) must be substituted, not shipped literally.
    /// `placeholders()` already trimmed the inner name for the missing-value
    /// check; substitution has to agree or a spaced placeholder passes as
    /// "present" and then never gets replaced.
    #[test]
    fn a_placeholder_with_inner_whitespace_is_substituted() {
        assert_eq!(
            render(
                "You are {{ name }}, spaced.",
                &vars(&[("name", "core-specialist")])
            )
            .unwrap(),
            "You are core-specialist, spaced."
        );
    }

    #[test]
    fn a_placeholder_with_extra_inner_whitespace_is_substituted() {
        assert_eq!(
            render("Hi {{  name  }}!", &vars(&[("name", "Ada")])).unwrap(),
            "Hi Ada!"
        );
    }

    /// `render` and `render_lenient` must agree on exactly the same set of
    /// occurrences, spaced or not — they share one scan for that reason.
    #[test]
    fn render_and_render_lenient_agree_on_spaced_placeholders() {
        let template = "You are {{ name }}, a {{ role }}.";
        let complete = vars(&[("name", "Ada"), ("role", "engineer")]);

        let strict = render(template, &complete).unwrap();
        let (lenient, left) = render_lenient(template, &complete);
        assert_eq!(strict, lenient);
        assert!(left.is_empty());

        let partial = vars(&[("name", "Ada")]);
        let err = render(template, &partial).unwrap_err();
        let (lenient_out, left) = render_lenient(template, &partial);
        match err {
            TemplateError::Unsubstituted(names) => assert_eq!(names, left),
        }
        // The missing placeholder is left exactly as written, spacing included.
        assert_eq!(lenient_out, "You are Ada, a {{ role }}.");
    }

    /// A substituted value is inserted literally, never rescanned for
    /// placeholders of its own — otherwise substitution order (BTreeMap key
    /// order today) would change the result.
    #[test]
    fn a_substituted_value_is_not_rescanned_for_nested_placeholders() {
        let out = render("{{a}}", &vars(&[("a", "{{b}}"), ("b", "BOOM")])).unwrap();
        assert_eq!(out, "{{b}}");
    }
}
