use anyhow::Context;

use crate::core::agent::AgentMetadata;

/// Parse the Metadata section content (YAML-like list format) into AgentMetadata.
pub fn parse_metadata(raw: &str) -> anyhow::Result<AgentMetadata> {
    let mut provider = None;
    let mut model = None;
    let mut command = None;
    let mut args = None;
    let mut temperature = 0.7_f32;
    let mut max_tokens = None;
    let mut timeout = None;
    let mut tags = Vec::new();
    let mut stacks = Vec::new();
    let mut scope = Vec::new();
    let mut model_fallback = Vec::new();
    let mut cost_limit = None;
    let mut rate_limit = None;
    let mut context_window = None;

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
    })
}

/// Validate metadata fields for consistency.
pub fn validate_metadata(metadata: &AgentMetadata) -> anyhow::Result<()> {
    match metadata.provider.as_str() {
        "cli" => {
            if metadata.command.is_none() {
                anyhow::bail!("CLI provider requires 'command' field in Metadata");
            }
        }
        "anthropic" | "openai" | "google" | "proxy" => {
            if metadata.model.is_none() {
                anyhow::bail!(
                    "API provider '{}' requires 'model' field in Metadata",
                    metadata.provider
                );
            }
        }
        other => {
            tracing::warn!("Unknown provider type: {other}");
        }
    }

    if metadata.temperature < 0.0 || metadata.temperature > 2.0 {
        anyhow::bail!("Temperature must be between 0.0 and 2.0");
    }

    Ok(())
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
