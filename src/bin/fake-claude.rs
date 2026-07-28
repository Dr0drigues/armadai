//! `fake-claude` — deterministic stand-in for the `claude` CLI, used by the e2e harness.
//!
//! It is a rule engine driven by a YAML scenario (`FAKE_SCENARIO`): given the agent id
//! (extracted from the composed prompt via the `FAKE_AGENT_ID:` marker) and a per-agent
//! call counter (persisted under `FAKE_STATE_DIR/<agent>.count`), it picks the first
//! matching rule and emits Claude Code's `stream-json` output format on stdout so that
//! `src/providers/json_runner.rs` parses it exactly like a real `claude -p --output-format
//! stream-json` invocation.
//!
//! The e2e runner shadows `claude` on `PATH` with this binary (see the e2e harness plan),
//! so `armadai`'s `CliProvider` (command = `claude`) shells out to us transparently.

use serde::Deserialize;
use std::env;
use std::fs;
use std::path::Path;

/// Top-level scenario loaded from `FAKE_SCENARIO` (YAML).
#[derive(Debug, Clone, Deserialize)]
pub struct Scenario {
    pub rules: Vec<Rule>,
}

/// A single rule: a `match` predicate plus the scripted response/metrics.
#[derive(Debug, Clone, Deserialize)]
pub struct Rule {
    #[serde(rename = "match", default)]
    pub match_: Match,
    pub respond: String,
    #[serde(default)]
    pub tokens_in: Option<u32>,
    #[serde(default)]
    pub tokens_out: Option<u32>,
    #[serde(default)]
    pub cost: Option<f64>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub latency_ms: Option<u64>,
    #[serde(default)]
    pub exit_code: Option<i32>,
}

/// Match predicate for a [`Rule`]. All fields are optional; an empty `Match` (all `None`)
/// is a catch-all that matches any agent/call/prompt.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Match {
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub call: Option<u32>,
    #[serde(default)]
    pub prompt_contains: Option<String>,
}

/// The marker inserted into an agent's system prompt so the fake can identify which
/// agent is calling it (see `<system>…FAKE_AGENT_ID: <agent>…</system>` in the composed
/// prompt built by the CLI provider).
const AGENT_ID_MARKER: &str = "FAKE_AGENT_ID:";

/// Extract the agent id from the composed prompt via the `FAKE_AGENT_ID:` marker.
///
/// The marker is expected on its own line as `FAKE_AGENT_ID: <agent>`; the agent id is
/// the first whitespace-delimited token following the marker. Returns `None` if the
/// marker is absent.
pub fn agent_id_from_prompt(prompt: &str) -> Option<String> {
    for line in prompt.lines() {
        if let Some(rest) = line.trim_start().strip_prefix(AGENT_ID_MARKER) {
            let id = rest.split_whitespace().next()?;
            return Some(id.to_string());
        }
    }
    None
}

/// Select the first rule in `scenario` whose `match` predicate is satisfied by
/// `(agent, call, prompt)`. `call` is the 1-indexed call count for `agent` (i.e. this is
/// the Nth time this agent has been invoked). An empty `Match` matches everything, so a
/// trailing catch-all rule (last in the list) always wins if nothing more specific does.
///
/// # Panics
/// Panics if no rule matches — the scenario author must always provide a catch-all rule
/// (a `Rule` whose `match` is `Match::default()`), which is the documented contract.
pub fn select_response<'s>(
    scenario: &'s Scenario,
    agent: &str,
    call: u32,
    prompt: &str,
) -> &'s Rule {
    scenario
        .rules
        .iter()
        .find(|rule| rule_matches(&rule.match_, agent, call, prompt))
        .unwrap_or_else(|| panic!("fake-claude: no rule matched agent={agent} call={call} — scenario needs a catch-all rule"))
}

fn rule_matches(m: &Match, agent: &str, call: u32, prompt: &str) -> bool {
    if let Some(want_agent) = &m.agent
        && want_agent != agent
    {
        return false;
    }
    if let Some(want_call) = m.call
        && want_call != call
    {
        return false;
    }
    if let Some(needle) = &m.prompt_contains
        && !prompt.contains(needle.as_str())
    {
        return false;
    }
    true
}

/// Emit the two `stream-json` lines (assistant message + result) for `rule`, joined by a
/// newline. `agent` is currently unused by the payload itself (Claude Code's stream-json
/// carries no agent id) but is accepted for symmetry with the rest of the engine and to
/// leave room for future per-agent metadata.
pub fn emit_claude_jsonl(rule: &Rule, _agent: &str) -> String {
    let assistant = serde_json::json!({
        "type": "assistant",
        "message": {
            "content": [
                { "type": "text", "text": rule.respond }
            ]
        }
    });

    let model = rule
        .model
        .clone()
        .unwrap_or_else(|| "fake-model".to_string());
    let tokens_in = rule.tokens_in.unwrap_or(0);
    let tokens_out = rule.tokens_out.unwrap_or(0);
    let cost = rule.cost.unwrap_or(0.0);

    let result = serde_json::json!({
        "type": "result",
        "subtype": "success",
        "is_error": false,
        "result": rule.respond,
        "total_cost_usd": cost,
        "usage": {
            "input_tokens": tokens_in,
            "output_tokens": tokens_out
        },
        "modelUsage": {
            model: { "outputTokens": tokens_out }
        }
    });

    format!("{assistant}\n{result}")
}

/// Read the current per-agent call count from `FAKE_STATE_DIR/<agent>.count`, increment
/// it, persist the new value, and return the incremented (1-indexed) count.
fn next_call_count(state_dir: &Path, agent: &str) -> u32 {
    let path = state_dir.join(format!("{agent}.count"));
    let current: u32 = fs::read_to_string(&path)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);
    let next = current + 1;
    // Best-effort: an e2e harness controls this directory and creates it upfront, but we
    // don't want a missing dir to crash the fake mid-suite.
    let _ = fs::create_dir_all(state_dir);
    let _ = fs::write(&path, next.to_string());
    next
}

fn main() {
    // The composed prompt is passed as the last CLI argument by the CLI provider
    // (`-p`/`--print`-style invocation): `claude -p --output-format stream-json --verbose <prompt>`.
    let prompt = env::args().next_back().unwrap_or_default();

    let scenario_path = env::var("FAKE_SCENARIO")
        .unwrap_or_else(|_| panic!("fake-claude: FAKE_SCENARIO env var is required"));
    let scenario_raw = fs::read_to_string(&scenario_path)
        .unwrap_or_else(|e| panic!("fake-claude: cannot read FAKE_SCENARIO {scenario_path}: {e}"));
    let scenario: Scenario = serde_yaml_ng::from_str(&scenario_raw)
        .unwrap_or_else(|e| panic!("fake-claude: invalid FAKE_SCENARIO YAML {scenario_path}: {e}"));

    let agent = agent_id_from_prompt(&prompt).unwrap_or_else(|| "unknown".to_string());

    let state_dir = env::var("FAKE_STATE_DIR")
        .unwrap_or_else(|_| panic!("fake-claude: FAKE_STATE_DIR env var is required"));
    let call = next_call_count(Path::new(&state_dir), &agent);

    let rule = select_response(&scenario, &agent, call, &prompt);

    if let Some(ms) = rule.latency_ms {
        std::thread::sleep(std::time::Duration::from_millis(ms));
    }

    println!("{}", emit_claude_jsonl(rule, &agent));

    std::process::exit(rule.exit_code.unwrap_or(0));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scen() -> Scenario {
        Scenario {
            rules: vec![
                Rule {
                    match_: Match {
                        agent: Some("t-coord".into()),
                        call: Some(1),
                        prompt_contains: None,
                    },
                    respond: "@t-a: go".into(),
                    tokens_in: Some(10),
                    tokens_out: Some(3),
                    cost: Some(0.001),
                    model: Some("m".into()),
                    latency_ms: None,
                    exit_code: None,
                },
                Rule {
                    match_: Match::default(),
                    respond: "ok".into(),
                    tokens_in: None,
                    tokens_out: None,
                    cost: None,
                    model: None,
                    latency_ms: None,
                    exit_code: None,
                },
            ],
        }
    }

    #[test]
    fn selects_by_agent_and_call() {
        let s = scen();
        let r = select_response(&s, "t-coord", 1, "prompt");
        assert_eq!(r.respond, "@t-a: go");
        // fallback catch-all for another agent
        assert_eq!(select_response(&s, "t-x", 1, "p").respond, "ok");
    }

    #[test]
    fn emits_parseable_claude_jsonl() {
        let r = &scen().rules[0];
        let out = emit_claude_jsonl(r, "t-coord");
        // two lines: assistant (content) + result (metrics)
        assert!(out.contains(r#""type":"assistant""#));
        assert!(out.contains(r#""type":"result""#));
        assert!(out.contains(r#""text":"@t-a: go""#));
        assert!(out.contains(r#""input_tokens":10"#));
        assert!(out.contains(r#""total_cost_usd":0.001"#));
    }

    #[test]
    fn extracts_agent_id_from_prompt() {
        let p = "<system>\nyou are X\nFAKE_AGENT_ID: t-coord\n</system>\n\ntask";
        assert_eq!(agent_id_from_prompt(p), Some("t-coord".to_string()));
        assert_eq!(agent_id_from_prompt("no marker"), None);
    }

    #[test]
    fn selects_by_prompt_contains() {
        let s = Scenario {
            rules: vec![
                Rule {
                    match_: Match {
                        agent: None,
                        call: None,
                        prompt_contains: Some("special".into()),
                    },
                    respond: "special-path".into(),
                    tokens_in: None,
                    tokens_out: None,
                    cost: None,
                    model: None,
                    latency_ms: None,
                    exit_code: None,
                },
                Rule {
                    match_: Match::default(),
                    respond: "default-path".into(),
                    tokens_in: None,
                    tokens_out: None,
                    cost: None,
                    model: None,
                    latency_ms: None,
                    exit_code: None,
                },
            ],
        };
        assert_eq!(
            select_response(&s, "any", 1, "this has a special word").respond,
            "special-path"
        );
        assert_eq!(
            select_response(&s, "any", 1, "plain prompt").respond,
            "default-path"
        );
    }

    #[test]
    fn emits_defaults_when_metrics_absent() {
        let r = Rule {
            match_: Match::default(),
            respond: "ok".into(),
            tokens_in: None,
            tokens_out: None,
            cost: None,
            model: None,
            latency_ms: None,
            exit_code: None,
        };
        let out = emit_claude_jsonl(&r, "any");
        assert!(out.contains(r#""total_cost_usd":0.0"#));
        assert!(out.contains(r#""input_tokens":0"#));
        assert!(out.contains(r#""output_tokens":0"#));
        assert!(out.contains(r#""fake-model""#));
    }

    #[test]
    fn extracts_agent_id_ignores_leading_whitespace() {
        let p = "  FAKE_AGENT_ID: t-writer\nrest of prompt";
        assert_eq!(agent_id_from_prompt(p), Some("t-writer".to_string()));
    }

    #[test]
    #[should_panic(expected = "no rule matched")]
    fn select_response_panics_without_catch_all() {
        let s = Scenario {
            rules: vec![Rule {
                match_: Match {
                    agent: Some("only-this".into()),
                    call: None,
                    prompt_contains: None,
                },
                respond: "x".into(),
                tokens_in: None,
                tokens_out: None,
                cost: None,
                model: None,
                latency_ms: None,
                exit_code: None,
            }],
        };
        select_response(&s, "someone-else", 1, "p");
    }

    #[test]
    fn next_call_count_increments_and_persists() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(next_call_count(dir.path(), "t-a"), 1);
        assert_eq!(next_call_count(dir.path(), "t-a"), 2);
        assert_eq!(next_call_count(dir.path(), "t-a"), 3);
        // Independent counters per agent.
        assert_eq!(next_call_count(dir.path(), "t-b"), 1);
    }
}
