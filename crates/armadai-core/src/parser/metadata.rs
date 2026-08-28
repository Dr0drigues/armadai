use std::path::Path;

use anyhow::Context;

use crate::agent::{AgentMetadata, AgentMode, default_temperature};
use crate::orchestration::OrchestrationPattern;

/// Split one line of a `- key: value` configuration section into its lowercased
/// key and its trimmed value, or `None` for a line the parser skips.
///
/// `## Metadata`, `## Triggers` and `## Ring Config` all have this shape, and
/// so does [`duplicate_keys`] — which has to see *exactly* the lines the
/// parsers see, or it would report keys that never take effect and miss ones
/// that do. One function, so the two passes cannot drift apart.
pub(super) fn config_line(line: &str) -> Option<(String, &str)> {
    let line = line.trim().trim_start_matches('-').trim();
    if line.is_empty() {
        return None;
    }
    let (key, value) = line.split_once(':')?;
    Some((key.trim().to_lowercase(), value.trim()))
}

/// The canonical field a `## Metadata` key sets, or `None` when the parser
/// ignores the key.
///
/// This must list exactly the keys the `match` in [`parse_metadata`]
/// recognises. A key missing here simply gets no duplicate warning (the
/// pre-#396 silence); a key listed here that the parser ignores would warn
/// about a line that changes nothing. `model_fallback`/`model_fallbacks` are
/// two spellings of one field, so a section using both collides with itself.
fn metadata_field(key: &str) -> Option<&'static str> {
    Some(match key {
        "provider" => "provider",
        "model" => "model",
        "command" => "command",
        "args" => "args",
        "temperature" => "temperature",
        "max_tokens" => "max_tokens",
        "timeout" => "timeout",
        "tags" => "tags",
        "stacks" => "stacks",
        "scope" => "scope",
        "model_fallback" | "model_fallbacks" => "model_fallback",
        "cost_limit" => "cost_limit",
        "rate_limit" => "rate_limit",
        "context_window" => "context_window",
        "mode" => "mode",
        "orchestration" => "orchestration",
        _ => return None,
    })
}

/// Every override a `- key: value` section performs on itself, in the order it
/// happens: `(canonical key, value being replaced, value replacing it)`.
///
/// A section that sets one key three times yields two entries, each naming the
/// pair it supersedes, so the whole chain is visible rather than only its ends.
pub(super) fn duplicate_keys(
    raw: &str,
    field: fn(&str) -> Option<&'static str>,
) -> Vec<(&'static str, String, String)> {
    let mut seen: std::collections::HashMap<&'static str, String> =
        std::collections::HashMap::new();
    let mut overrides = Vec::new();
    for line in raw.lines() {
        let Some((key, value)) = config_line(line) else {
            continue;
        };
        let Some(field) = field(&key) else {
            continue;
        };
        if let Some(previous) = seen.insert(field, value.to_string()) {
            overrides.push((field, previous, value.to_string()));
        }
    }
    overrides
}

/// Report every key a configuration section sets more than once (#396).
///
/// These sections are read line by line and the last value wins. Nothing here
/// changes that — only the silence. Since #392 a `###` sub-block inside one of
/// them is no longer truncated away, so a commented-out "alternative setup"
/// block silently reconfigures the agent, and an unparsable value in one makes
/// the whole file fail to load. The warning is emitted before the values are
/// parsed, so it reaches the user in that second case too.
pub(super) fn warn_duplicate_keys(
    raw: &str,
    source: &Path,
    section: &str,
    field: fn(&str) -> Option<&'static str>,
) {
    for (key, replaced, winner) in duplicate_keys(raw, field) {
        tracing::warn!(
            "{}: ## {section} sets '{key}' twice: '{replaced}' is overridden by \
             '{winner}' (the last value wins)",
            source.display()
        );
    }
}

/// Parse the Metadata section content (YAML-like list format) into AgentMetadata.
///
/// `source` is only used to name the file in diagnostics; it is never read.
pub fn parse_metadata(raw: &str, source: &Path) -> anyhow::Result<AgentMetadata> {
    warn_duplicate_keys(raw, source, "Metadata", metadata_field);

    let mut provider = None;
    let mut model = None;
    let mut command = None;
    let mut args = None;
    let mut temperature = default_temperature();
    let mut max_tokens = None;
    let mut timeout = None;
    let mut tags = Vec::new();
    let mut stacks = Vec::new();
    let mut scope = Vec::new();
    let mut model_fallback = Vec::new();
    let mut cost_limit = None;
    let mut rate_limit = None;
    let mut context_window = None;
    let mut mode = None;
    let mut orchestration = None;

    for line in raw.lines() {
        let Some((key, value)) = config_line(line) else {
            continue;
        };

        match key.as_str() {
            "provider" => provider = Some(value.to_string()),
            "model" => model = Some(value.to_string()),
            "command" => command = Some(value.to_string()),
            "args" => args = Some(parse_string_list(value)),
            "temperature" => temperature = value.parse().context("invalid temperature")?,
            "max_tokens" => max_tokens = Some(value.parse().context("invalid max_tokens")?),
            "timeout" => timeout = Some(value.parse().context("invalid timeout")?),
            "tags" => tags = parse_string_list(value),
            "stacks" => stacks = parse_string_list(value),
            "scope" => scope = parse_string_list(value),
            "model_fallback" | "model_fallbacks" => model_fallback = parse_string_list(value),
            "cost_limit" => cost_limit = Some(value.parse().context("invalid cost_limit")?),
            "rate_limit" => rate_limit = Some(value.to_string()),
            "context_window" => {
                context_window = Some(value.parse().context("invalid context_window")?)
            }
            "mode" => {
                mode = Some(match value.to_lowercase().as_str() {
                    "guided" => AgentMode::Guided,
                    "autonomous" => AgentMode::Autonomous,
                    _ => {
                        anyhow::bail!("Invalid mode: '{value}'. Expected 'guided' or 'autonomous'")
                    }
                })
            }
            // All five `OrchestrationPattern` variants are accepted (#415).
            // Rejecting `hierarchical`/`auto` was not "ignoring a field": the
            // `bail!` propagates out of `parse_agent_file`, so the whole agent
            // file became unloadable and the agent vanished from `run`, `link`,
            // `list`, the TUI and the audit at once — while `armadai.yaml`'s
            // own `orchestration.pattern` accepted all five through serde's
            // lowercase derive on the same enum.
            //
            // Widening cannot change how any run behaves: this per-agent field
            // is descriptive only. Its sole readers are `tui/views/agent_detail`
            // and `web/api`, both of which just display it; the pattern an
            // orchestrated run actually uses comes from `armadai.yaml` or
            // `--orchestrate`.
            "orchestration" => {
                orchestration = Some(match value.to_lowercase().as_str() {
                    "direct" => OrchestrationPattern::Direct,
                    "blackboard" => OrchestrationPattern::Blackboard,
                    "ring" => OrchestrationPattern::Ring,
                    "hierarchical" => OrchestrationPattern::Hierarchical,
                    "auto" => OrchestrationPattern::Auto,
                    _ => {
                        anyhow::bail!(
                            "Invalid orchestration: '{value}'. Expected 'direct', \
                             'blackboard', 'ring', 'hierarchical' or 'auto'"
                        )
                    }
                })
            }
            _ => {
                tracing::debug!("Unknown metadata field: {key}");
            }
        }
    }

    Ok(AgentMetadata {
        provider: provider.context("Missing 'provider' in Metadata")?,
        model,
        command,
        args,
        temperature,
        max_tokens,
        timeout,
        tags,
        stacks,
        scope,
        model_fallback,
        cost_limit,
        rate_limit,
        context_window,
        mode,
        orchestration,
        triggers: None,
        ring_config: None,
    })
}

/// Parse a bracket-delimited list like `[rust, typescript, java]` into a Vec<String>.
fn parse_string_list(value: &str) -> Vec<String> {
    let trimmed = value.trim().trim_start_matches('[').trim_end_matches(']');
    trimmed
        .split(',')
        .map(|s| s.trim().trim_matches('"').trim_matches('\'').to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Stand-in for the agent file these fixtures pretend to come from:
    /// `parse_metadata` only uses `source` to name the file in diagnostics.
    fn source() -> &'static Path {
        Path::new("test-agent.md")
    }

    #[test]
    fn test_parse_scope() {
        let raw = "\
- provider: google
- model: gemini-2.5-pro
- temperature: 0.3
- tags: [review, quality]
- scope: [src/**/*.rs, tests/]
";
        let meta = parse_metadata(raw, source()).unwrap();
        assert_eq!(meta.scope, vec!["src/**/*.rs", "tests/"]);
    }

    #[test]
    fn test_parse_scope_empty() {
        let raw = "\
- provider: google
- model: gemini-2.5-pro
";
        let meta = parse_metadata(raw, source()).unwrap();
        assert!(meta.scope.is_empty());
    }

    #[test]
    fn test_parse_model_fallback() {
        let raw = "\
- provider: google
- model: gemini-3.0-pro
- model_fallback: [gemini-2.5-pro, gemini-2.5-flash]
";
        let meta = parse_metadata(raw, source()).unwrap();
        assert_eq!(
            meta.model_fallback,
            vec!["gemini-2.5-pro", "gemini-2.5-flash"]
        );
    }

    #[test]
    fn test_parse_model_fallbacks_plural_alias() {
        let raw = "\
- provider: anthropic
- model: claude-opus-4-6
- model_fallbacks: [claude-sonnet-4-5-20250929]
";
        let meta = parse_metadata(raw, source()).unwrap();
        assert_eq!(meta.model_fallback, vec!["claude-sonnet-4-5-20250929"]);
    }

    #[test]
    fn test_parse_model_fallback_empty_by_default() {
        let raw = "\
- provider: google
- model: gemini-2.5-pro
";
        let meta = parse_metadata(raw, source()).unwrap();
        assert!(meta.model_fallback.is_empty());
    }

    #[test]
    fn test_parse_mode_guided() {
        let raw = "\
- provider: anthropic
- model: claude-sonnet-4-5-20250929
- mode: guided
";
        let meta = parse_metadata(raw, source()).unwrap();
        assert_eq!(meta.mode, Some(AgentMode::Guided));
    }

    #[test]
    fn test_parse_mode_autonomous() {
        let raw = "\
- provider: anthropic
- model: claude-sonnet-4-5-20250929
- mode: autonomous
";
        let meta = parse_metadata(raw, source()).unwrap();
        assert_eq!(meta.mode, Some(AgentMode::Autonomous));
    }

    #[test]
    fn test_parse_mode_default_none() {
        let raw = "\
- provider: anthropic
- model: claude-sonnet-4-5-20250929
";
        let meta = parse_metadata(raw, source()).unwrap();
        assert!(meta.mode.is_none());
    }

    #[test]
    fn test_parse_mode_invalid() {
        let raw = "\
- provider: anthropic
- model: claude-sonnet-4-5-20250929
- mode: interactive
";
        assert!(parse_metadata(raw, source()).is_err());
    }

    // -----------------------------------------------------------------
    // #396 — a duplicated key overwrites in silence.
    //
    // These pin *which* duplicates are detected. That the warning actually
    // reaches the user is a different property, and no unit test can hold it:
    // it is measured on the real binary in
    // `crates/armadai/tests/duplicate_metadata_key_warns.rs`.
    // -----------------------------------------------------------------

    #[test]
    fn test_duplicate_keys_reports_loser_and_winner_in_order() {
        let raw = "\
- provider: anthropic
- model: claude-sonnet-4-5-20250929
- temperature: 0.2

### Alternative setup (not in use)
- provider: openai
- temperature: 1.0
";
        assert_eq!(
            duplicate_keys(raw, metadata_field),
            vec![
                ("provider", "anthropic".to_string(), "openai".to_string()),
                ("temperature", "0.2".to_string(), "1.0".to_string()),
            ],
            "the overrides must be reported in the order they happen, each \
             naming the value it replaces and the value that wins"
        );
    }

    /// The parser's precedence is deliberately untouched: the LAST value still
    /// wins. Without this, a fix that "made duplicates safe" by keeping the
    /// first value would pass every warning assertion above.
    #[test]
    fn test_duplicate_key_still_lets_the_last_value_win() {
        let raw = "\
- provider: anthropic
- temperature: 0.2
- provider: openai
- temperature: 1.0
";
        let meta = parse_metadata(raw, source()).unwrap();
        assert_eq!(meta.provider, "openai");
        assert_eq!(meta.temperature, 1.0);
    }

    /// Three settings of one key are two overrides, each naming its own pair —
    /// not one entry collapsing the first value onto the last.
    #[test]
    fn test_duplicate_keys_reports_every_link_of_a_chain() {
        let raw = "\
- provider: anthropic
- provider: openai
- provider: google
";
        assert_eq!(
            duplicate_keys(raw, metadata_field),
            vec![
                ("provider", "anthropic".to_string(), "openai".to_string()),
                ("provider", "openai".to_string(), "google".to_string()),
            ]
        );
    }

    /// Keys the parser ignores must not warn: they configure nothing, and any
    /// prose line carrying a colon would otherwise qualify.
    #[test]
    fn test_duplicate_keys_ignores_keys_the_parser_does_not_read() {
        let raw = "\
- provider: anthropic
- reviewer: someone
- reviewer: someone else
- Rationale: kept at 0.2 for reproducibility
- Rationale: or maybe not
";
        assert!(
            duplicate_keys(raw, metadata_field).is_empty(),
            "only recognised keys may be reported"
        );
    }

    /// A section that says nothing twice reports nothing — including one whose
    /// keys merely look alike.
    #[test]
    fn test_duplicate_keys_is_empty_without_a_duplicate() {
        let raw = "\
- provider: anthropic
- model: claude-sonnet-4-5-20250929
- model_fallback: [claude-opus-4-6]
- temperature: 0.2
- max_tokens: 4096
";
        assert!(duplicate_keys(raw, metadata_field).is_empty());
    }

    /// `model_fallback` and `model_fallbacks` are two spellings of one field,
    /// so using both is a duplicate even though the literal keys differ — the
    /// second silently replaces the first, exactly like a repeated spelling.
    #[test]
    fn test_duplicate_keys_sees_through_the_model_fallback_alias() {
        let raw = "\
- provider: anthropic
- model_fallback: [claude-opus-4-6]
- model_fallbacks: [gemini-2.5-pro]
";
        assert_eq!(
            duplicate_keys(raw, metadata_field),
            vec![(
                "model_fallback",
                "[claude-opus-4-6]".to_string(),
                "[gemini-2.5-pro]".to_string()
            )]
        );
    }

    /// The case that hurts most: an unparsable value in the losing block makes
    /// the whole file fail to load, and the duplicate is the reason why. Both
    /// halves are pinned here — the parse still fails, and the override is
    /// still detected.
    ///
    /// What this test does NOT prove is the *ordering* that makes the second
    /// half useful (the warning must be emitted before the `?` bails, or the
    /// user never sees it). Measured: moving the pass below the parse loop
    /// leaves this test green and reddens
    /// `link_names_the_duplicate_that_makes_an_agent_unloadable` in
    /// `crates/armadai/tests/duplicate_metadata_key_warns.rs`, which is where
    /// that property lives.
    #[test]
    fn test_duplicate_key_is_reported_even_when_the_second_value_is_fatal() {
        let raw = "\
- provider: anthropic
- timeout: 300
- timeout: to be decided
";
        assert!(
            parse_metadata(raw, source()).is_err(),
            "an unparsable timeout must still fail the parse"
        );
        assert_eq!(
            duplicate_keys(raw, metadata_field),
            vec![("timeout", "300".to_string(), "to be decided".to_string())]
        );
    }

    /// #415: every `OrchestrationPattern` variant must be declarable from a
    /// `## Metadata` section. Before the fix, `hierarchical` and `auto` made
    /// `parse_metadata` `bail!`, and since `parse_agent_file` propagates that
    /// error the WHOLE agent file became unloadable — the agent vanished from
    /// `run`, `link`, `list`, the TUI and the audit at once.
    ///
    /// Table-driven on purpose: it doubles as the negative control the
    /// per-variant assertions need. A fix that maps every value to one variant
    /// (say `Direct`) satisfies "hierarchical parses" but fails here.
    #[test]
    fn test_parse_orchestration_accepts_every_pattern() {
        let cases = [
            ("direct", OrchestrationPattern::Direct),
            ("blackboard", OrchestrationPattern::Blackboard),
            ("ring", OrchestrationPattern::Ring),
            ("hierarchical", OrchestrationPattern::Hierarchical),
            ("auto", OrchestrationPattern::Auto),
        ];
        for (value, expected) in cases {
            let raw = format!(
                "\
- provider: anthropic
- model: claude-sonnet-4-5-20250929
- orchestration: {value}
"
            );
            let meta = parse_metadata(&raw, source())
                .unwrap_or_else(|e| panic!("`- orchestration: {value}` must parse, got: {e}"));
            assert_eq!(
                meta.orchestration,
                Some(expected),
                "`- orchestration: {value}` parsed to the wrong variant"
            );
        }
    }

    /// The value is matched case-insensitively, like `mode` above.
    #[test]
    fn test_parse_orchestration_is_case_insensitive() {
        let raw = "\
- provider: anthropic
- model: claude-sonnet-4-5-20250929
- orchestration: Hierarchical
";
        let meta = parse_metadata(raw, source()).unwrap();
        assert_eq!(meta.orchestration, Some(OrchestrationPattern::Hierarchical));
    }

    /// An unknown pattern is still rejected — and the message must enumerate
    /// the five real variants. The old message listed three "as if the list
    /// were complete" (#415), which is what sent an author down the wrong path.
    #[test]
    fn test_parse_orchestration_rejects_unknown_and_lists_all_five() {
        let raw = "\
- provider: anthropic
- model: claude-sonnet-4-5-20250929
- orchestration: mesh
";
        let err = parse_metadata(raw, source())
            .expect_err("`- orchestration: mesh` is not a pattern and must be refused")
            .to_string();
        assert_eq!(
            err,
            "Invalid orchestration: 'mesh'. Expected 'direct', 'blackboard', \
             'ring', 'hierarchical' or 'auto'"
        );
    }

    #[test]
    fn test_parse_metadata_full() {
        let raw = "\
- provider: anthropic
- model: claude-sonnet-4-5-20250929
- temperature: 0.5
- max_tokens: 4096
- tags: [dev, test]
- stacks: [rust]
- scope: [src/, docs/*.md]
- cost_limit: 1.50
- rate_limit: 10/min
";
        let meta = parse_metadata(raw, source()).unwrap();
        assert_eq!(meta.provider, "anthropic");
        assert_eq!(meta.model.as_deref(), Some("claude-sonnet-4-5-20250929"));
        assert_eq!(meta.tags, vec!["dev", "test"]);
        assert_eq!(meta.stacks, vec!["rust"]);
        assert_eq!(meta.scope, vec!["src/", "docs/*.md"]);
        assert_eq!(meta.cost_limit, Some(1.50));
    }
}
