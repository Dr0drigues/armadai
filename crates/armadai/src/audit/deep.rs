//! Deep pass: build the LLM auditor payload/prompt and parse its response.
//!
//! The static rules (`rules/`) run without any LLM call. The deep pass is an
//! optional second stage (`audit --deep`) that hands the same imported
//! config plus the static findings to an LLM and asks it to surface
//! cross-cutting issues (role overlap, vague prompts, semantic duplication,
//! team topology suggestions, CLAUDE.md contradictions) that a purely
//! syntactic pass cannot see.

use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};

use super::reverse::ImportedConfig;
use super::rules::references::secret_res;
use super::rules::{Finding, Severity};

/// CLI tools that can act as the deep-pass auditor, in preference order.
///
/// Limited to CLIs verified to run non-interactively and read-only via the
/// `-p` flag (unified-tool path in `providers::factory`): `codex`/`copilot`/
/// `opencode` invoked bare (no subcommand/flags) open an interactive/TUI
/// mode that hangs until timeout, and `aider` auto-commits edits to the
/// audited repo, which is unacceptable during a read-only audit.
const DEEP_CLIS: [&str; 2] = ["claude", "gemini"];

#[cfg(not(windows))]
fn cli_is_available(cli: &str) -> bool {
    Command::new("which")
        .arg(cli)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

#[cfg(windows)]
fn cli_is_available(cli: &str) -> bool {
    Command::new("where")
        .arg(cli)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Detect the first available CLI tool usable as the deep-pass auditor.
pub(crate) fn available_cli() -> Option<&'static str> {
    DEEP_CLIS.into_iter().find(|cli| cli_is_available(cli))
}

/// Embedded instructions for the deep-pass auditor persona.
const AUDITOR_INSTRUCTIONS: &str = include_str!("deep_auditor.md");

/// Deep-pass finding kinds, in the order they are documented.
pub(crate) const DEEP_RULES: [&str; 5] = ["D01", "D02", "D03", "D04", "D05"];

#[derive(Debug, Serialize)]
struct PayloadAgent {
    name: String,
    description: Option<String>,
    model: Option<String>,
    tools: Option<Vec<String>>,
    scope: Vec<String>,
    prompt_excerpt: String,
}

#[derive(Debug, Serialize)]
struct PayloadSkill {
    name: String,
    description: Option<String>,
}

#[derive(Debug, Serialize)]
struct PayloadFinding {
    rule: &'static str,
    severity: &'static str,
    file: String,
    message: String,
}

#[derive(Debug, Serialize)]
struct DeepPayload {
    agents: Vec<PayloadAgent>,
    skills: Vec<PayloadSkill>,
    instructions_excerpt: Option<String>,
    static_findings: Vec<PayloadFinding>,
}

/// Truncate `text` to at most `n` characters, respecting char boundaries.
fn truncate_chars(text: &str, n: usize) -> String {
    text.chars().take(n).collect()
}

/// Redact plaintext secrets from `text` before it leaves the process.
///
/// Reuses the exact patterns A11 (`rules::references::secret_res`) uses to
/// *detect* secrets, so the deep pass never sends to an external LLM CLI a
/// secret that the static pass simultaneously flags as leaked.
fn redact_secrets(text: &str) -> String {
    let mut redacted = text.to_string();
    for re in secret_res() {
        redacted = re.replace_all(&redacted, "[REDACTED]").into_owned();
    }
    redacted
}

/// Redact secrets, then truncate to at most `n` characters. Redaction must
/// happen first: truncating before redacting could cut a secret in half,
/// leaving a still-sensitive fragment in the payload.
fn sanitize_excerpt(text: &str, n: usize) -> String {
    truncate_chars(&redact_secrets(text), n)
}

fn severity_label(severity: Severity) -> &'static str {
    match severity {
        Severity::Critical => "critical",
        Severity::Warning => "warning",
        Severity::Info => "info",
    }
}

/// Build the compact JSON payload sent to the LLM auditor.
///
/// Agents carrying `ParseIssue`s are still included (the model may comment
/// on them); their prompt is truncated like any other.
pub(crate) fn build_payload(
    config: &ImportedConfig,
    findings: &[Finding],
    truncation: usize,
) -> String {
    let agents = config
        .agents
        .iter()
        .map(|a| PayloadAgent {
            name: a.name.clone(),
            description: a.metadata.description.clone(),
            model: a.metadata.model.clone(),
            tools: a.metadata.tools.clone(),
            scope: a.metadata.scope_globs(),
            prompt_excerpt: sanitize_excerpt(&a.system_prompt, truncation),
        })
        .collect();

    let skills = config
        .skills
        .iter()
        .map(|s| PayloadSkill {
            name: s.name.clone(),
            description: s.description.clone(),
        })
        .collect();

    let instructions_excerpt = config
        .instructions
        .as_ref()
        .map(|i| sanitize_excerpt(&i.content, truncation));

    let static_findings = findings
        .iter()
        .map(|f| PayloadFinding {
            rule: f.rule,
            severity: severity_label(f.severity),
            file: f.file.display().to_string(),
            message: f.message.clone(),
        })
        .collect();

    let payload = DeepPayload {
        agents,
        skills,
        instructions_excerpt,
        static_findings,
    };

    // The DTOs above are built exclusively from in-memory data (no I/O), so
    // serialization cannot fail; fall back to an empty object rather than
    // panicking in the unreachable error path.
    serde_json::to_string(&payload).unwrap_or_default()
}

/// Build the full prompt sent to the auditor: embedded instructions, the
/// input payload, and a reminder of the expected output format.
pub(crate) fn build_prompt(payload_json: &str) -> String {
    format!(
        "{AUDITOR_INSTRUCTIONS}\n\nINPUT JSON:\n{payload_json}\n\n\
Respond with ONLY a JSON object of the form:\n\
{{\"findings\":[{{\"kind\":\"D01\",\"severity\":\"warning\",\"file\":\"...\",\"message\":\"...\",\"suggestion\":\"...\"}}]}}\n\
No prose, no markdown fences."
    )
}

/// Result of parsing the LLM auditor's response.
pub(crate) enum DeepOutcome {
    Findings(Vec<Finding>),
    Raw(String),
}

#[derive(Debug, Deserialize)]
struct DeepResponse {
    findings: Vec<DeepItem>,
}

#[derive(Debug, Deserialize)]
struct DeepItem {
    kind: String,
    severity: String,
    file: String,
    message: String,
    suggestion: Option<String>,
}

/// Extract the first JSON object from `text`, which may be wrapped in prose
/// and/or a ```json fence.
fn extract_json(text: &str) -> Option<&str> {
    if let Some(fence_start) = text.find("```json") {
        let after_fence = &text[fence_start + "```json".len()..];
        if let Some(fence_end) = after_fence.find("```") {
            return Some(after_fence[..fence_end].trim());
        }
    }
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    if end < start {
        return None;
    }
    Some(&text[start..=end])
}

fn map_severity(severity: &str) -> Severity {
    match severity {
        "critical" => Severity::Critical,
        "warning" => Severity::Warning,
        _ => Severity::Info,
    }
}

/// Parse the LLM auditor's raw response into findings, or keep the raw text
/// if no valid JSON could be extracted.
pub(crate) fn parse_deep_response(text: &str) -> DeepOutcome {
    let Some(json) = extract_json(text) else {
        return DeepOutcome::Raw(text.trim().to_string());
    };
    let Ok(parsed) = serde_json::from_str::<DeepResponse>(json) else {
        return DeepOutcome::Raw(text.trim().to_string());
    };

    let findings = parsed
        .findings
        .into_iter()
        .filter_map(|item| {
            let kind = item.kind.to_uppercase();
            let rule = *DEEP_RULES.iter().find(|r| **r == kind)?;
            Some(Finding {
                rule,
                severity: map_severity(&item.severity),
                file: item.file.into(),
                related: Vec::new(),
                message: format!("[deep] {}", item.message),
                suggestion: item.suggestion,
            })
        })
        .collect();

    DeepOutcome::Findings(findings)
}

/// Run the deep pass: build the payload/prompt from `config`/`findings`,
/// invoke `run` to obtain the auditor's raw response, and parse it.
///
/// A failure of `run` itself (e.g. the CLI could not be invoked) is
/// propagated as an error, distinct from an invalid/unparsable response
/// which is surfaced as `DeepOutcome::Raw`.
pub(crate) fn run_deep(
    config: &ImportedConfig,
    findings: &[Finding],
    truncation: usize,
    run: impl Fn(&str) -> anyhow::Result<String>,
) -> anyhow::Result<DeepOutcome> {
    let payload = build_payload(config, findings, truncation);
    let prompt = build_prompt(&payload);
    let output = run(&prompt)?;
    Ok(parse_deep_response(&output))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::rules::test_support::{agent, config_with};
    use crate::audit::rules::{Finding, Severity};

    #[test]
    fn build_payload_truncates_prompts_and_includes_findings() {
        let a = agent("reviewer", &"x".repeat(5000));
        let config = config_with(vec![a]);
        let findings = vec![Finding {
            rule: "A08",
            severity: Severity::Info,
            file: ".claude/agents/reviewer.md".into(),
            related: vec![],
            message: "inherits all tools".into(),
            suggestion: None,
        }];
        let json = build_payload(&config, &findings, 100);
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["agents"][0]["name"], "reviewer");
        assert_eq!(
            v["agents"][0]["prompt_excerpt"]
                .as_str()
                .unwrap()
                .chars()
                .count(),
            100
        );
        assert_eq!(v["static_findings"][0]["rule"], "A08");
    }

    #[test]
    fn build_payload_redacts_secret_patterns() {
        let fake_key = "sk-ant-abcdefghijklmnopqrstuvwx"; // matches A11's sk-ant- pattern
        let a = agent(
            "leaky",
            &format!("Do not leak this: {fake_key}\nEnd of prompt."),
        );
        let config = config_with(vec![a]);
        let json = build_payload(&config, &[], 5000);
        assert!(json.contains("[REDACTED]"), "payload was: {json}");
        assert!(
            !json.contains(fake_key),
            "payload leaked the secret: {json}"
        );
    }

    #[test]
    fn parse_deep_response_maps_valid_json_to_findings() {
        let text = "Here is my analysis:\n```json\n{\"findings\":[{\"kind\":\"D01\",\"severity\":\"warning\",\"file\":\"a.md\",\"message\":\"roles overlap\",\"suggestion\":\"merge\"}]}\n```\nDone.";
        let DeepOutcome::Findings(f) = parse_deep_response(text) else {
            panic!("expected findings");
        };
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].rule, "D01");
        assert_eq!(f[0].severity, crate::audit::rules::Severity::Warning);
        assert!(f[0].message.starts_with("[deep] "));
    }

    #[test]
    fn parse_deep_response_unknown_kind_is_dropped() {
        let text = "{\"findings\":[{\"kind\":\"D99\",\"severity\":\"info\",\"file\":\"a\",\"message\":\"m\"}]}";
        let DeepOutcome::Findings(f) = parse_deep_response(text) else {
            panic!("expected findings (possibly empty)");
        };
        assert!(f.is_empty());
    }

    #[test]
    fn parse_deep_response_invalid_json_falls_back_to_raw() {
        let text = "The config looks fine overall, no structured output.";
        let DeepOutcome::Raw(r) = parse_deep_response(text) else {
            panic!("expected raw");
        };
        assert!(r.contains("looks fine"));
    }

    #[test]
    fn run_deep_with_fake_runner_returns_findings() {
        let config = config_with(vec![agent("a", "prompt")]);
        let run = |_prompt: &str| {
            Ok("{\"findings\":[{\"kind\":\"D02\",\"severity\":\"info\",\"file\":\"a.md\",\"message\":\"vague\"}]}".to_string())
        };
        let outcome = run_deep(&config, &[], 2000, run).unwrap();
        let DeepOutcome::Findings(f) = outcome else {
            panic!("expected findings")
        };
        assert_eq!(f[0].rule, "D02");
    }

    #[test]
    fn run_deep_propagates_runner_error() {
        let config = config_with(vec![agent("a", "prompt")]);
        let run = |_: &str| Err(anyhow::anyhow!("cli not found"));
        assert!(run_deep(&config, &[], 2000, run).is_err());
    }
}
