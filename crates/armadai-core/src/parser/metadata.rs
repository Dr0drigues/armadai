use anyhow::Context;

use crate::agent::{AgentMetadata, AgentMode, default_temperature};
use crate::orchestration::OrchestrationPattern;

/// Parse the Metadata section content (YAML-like list format) into AgentMetadata.
pub fn parse_metadata(raw: &str) -> anyhow::Result<AgentMetadata> {
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
        let line = line.trim().trim_start_matches('-').trim();
        if line.is_empty() {
            continue;
        }

        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim().to_lowercase();
        let value = value.trim();

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

    #[test]
    fn test_parse_scope() {
        let raw = "\
- provider: google
- model: gemini-2.5-pro
- temperature: 0.3
- tags: [review, quality]
- scope: [src/**/*.rs, tests/]
";
        let meta = parse_metadata(raw).unwrap();
        assert_eq!(meta.scope, vec!["src/**/*.rs", "tests/"]);
    }

    #[test]
    fn test_parse_scope_empty() {
        let raw = "\
- provider: google
- model: gemini-2.5-pro
";
        let meta = parse_metadata(raw).unwrap();
        assert!(meta.scope.is_empty());
    }

    #[test]
    fn test_parse_model_fallback() {
        let raw = "\
- provider: google
- model: gemini-3.0-pro
- model_fallback: [gemini-2.5-pro, gemini-2.5-flash]
";
        let meta = parse_metadata(raw).unwrap();
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
        let meta = parse_metadata(raw).unwrap();
        assert_eq!(meta.model_fallback, vec!["claude-sonnet-4-5-20250929"]);
    }

    #[test]
    fn test_parse_model_fallback_empty_by_default() {
        let raw = "\
- provider: google
- model: gemini-2.5-pro
";
        let meta = parse_metadata(raw).unwrap();
        assert!(meta.model_fallback.is_empty());
    }

    #[test]
    fn test_parse_mode_guided() {
        let raw = "\
- provider: anthropic
- model: claude-sonnet-4-5-20250929
- mode: guided
";
        let meta = parse_metadata(raw).unwrap();
        assert_eq!(meta.mode, Some(AgentMode::Guided));
    }

    #[test]
    fn test_parse_mode_autonomous() {
        let raw = "\
- provider: anthropic
- model: claude-sonnet-4-5-20250929
- mode: autonomous
";
        let meta = parse_metadata(raw).unwrap();
        assert_eq!(meta.mode, Some(AgentMode::Autonomous));
    }

    #[test]
    fn test_parse_mode_default_none() {
        let raw = "\
- provider: anthropic
- model: claude-sonnet-4-5-20250929
";
        let meta = parse_metadata(raw).unwrap();
        assert!(meta.mode.is_none());
    }

    #[test]
    fn test_parse_mode_invalid() {
        let raw = "\
- provider: anthropic
- model: claude-sonnet-4-5-20250929
- mode: interactive
";
        assert!(parse_metadata(raw).is_err());
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
            let meta = parse_metadata(&raw)
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
        let meta = parse_metadata(raw).unwrap();
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
        let err = parse_metadata(raw)
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
        let meta = parse_metadata(raw).unwrap();
        assert_eq!(meta.provider, "anthropic");
        assert_eq!(meta.model.as_deref(), Some("claude-sonnet-4-5-20250929"));
        assert_eq!(meta.tags, vec!["dev", "test"]);
        assert_eq!(meta.stacks, vec!["rust"]);
        assert_eq!(meta.scope, vec!["src/", "docs/*.md"]);
        assert_eq!(meta.cost_limit, Some(1.50));
    }
}
