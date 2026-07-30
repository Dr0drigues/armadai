//! Scenario-driven fake-`claude` engine, built on the `gaveldrop-fake` library.
//!
//! This crate ports the rule engine that used to live directly in
//! `crates/armadai/src/bin/fake-claude.rs` onto `gaveldrop-fake`'s call counter and
//! journal primitives. It defines armadai's own on-wire scenario format
//! (`Scenario`/`Rule`/`Match`) — opaque to gaveldrop, which only sees a `setup:` blob
//! in its own case format — and emits Claude Code's `stream-json` output so that
//! `armadai-providers`' `json_runner` parses it exactly like a real
//! `claude -p --output-format stream-json` invocation.
//!
//! See the task brief for why the scenario travels via a dedicated
//! `ARMADAI_FAKE_SCENARIO` env var rather than `GAVELDROP_SCENARIO`: gaveldrop's
//! `Isolation` always writes its own fallback scenario and points `GAVELDROP_SCENARIO`
//! at it, even when the case has no `fake:` section — that variable is not available
//! for armadai's own vocabulary. State and journal, on the other hand, do come from
//! gaveldrop: `Counter::from_env()` reads `GAVELDROP_STATE`, `Journal::from_env()`
//! reads `GAVELDROP_JOURNAL`, both set by `Isolation` and inherited by the `claude`
//! subprocess.

#[cfg(feature = "engine")]
use gaveldrop_fake::{Call, Counter, Invocation, Journal};
use serde::{Deserialize, Serialize};

/// Environment variable carrying the path to armadai's own scenario YAML.
pub const SCENARIO_ENV: &str = "ARMADAI_FAKE_SCENARIO";

/// Top-level scenario loaded from `ARMADAI_FAKE_SCENARIO` (YAML).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scenario {
    pub rules: Vec<Rule>,
}

/// A single rule: a `match` predicate plus the scripted response/metrics.
#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// Match predicate for a [`Rule`]. All fields are optional; an empty `Match` (all
/// `None`) is a catch-all that matches any agent/call/prompt.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
/// `(agent, call, prompt)`. `call` is the 1-indexed call count for `agent` (i.e. this
/// is the Nth time this agent has been invoked). An empty `Match` matches everything,
/// so a trailing catch-all rule (last in the list) always wins if nothing more
/// specific does.
///
/// Returns `None` if no rule matches — the caller decides what to do (journal a
/// catch-all miss and exit non-zero, per [`run`]), rather than panicking.
pub fn select_response<'s>(
    scenario: &'s Scenario,
    agent: &str,
    call: u32,
    prompt: &str,
) -> Option<&'s Rule> {
    scenario
        .rules
        .iter()
        .find(|rule| rule_matches(&rule.match_, agent, call, prompt))
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

/// Emit the two `stream-json` lines (assistant message + result) for `rule`, joined by
/// a newline.
pub fn emit_claude_jsonl(rule: &Rule) -> String {
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

/// Binary entry point: journal the call, select a rule, emit stream-json, exit.
#[cfg(feature = "engine")]
pub fn run() {
    let inv = Invocation::from_env(false);
    let prompt = std::env::args().next_back().unwrap_or_default();
    let agent = agent_id_from_prompt(&prompt).unwrap_or_else(|| "unknown".to_string());

    let counter = Counter::from_env().expect("GAVELDROP_STATE set by isolation");
    let call = counter.next(&agent).unwrap_or(1);

    let scenario: Option<Scenario> = std::env::var(SCENARIO_ENV).ok().map(|path| {
        let raw = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("fake-claude: cannot read {SCENARIO_ENV} {path}: {e}"));
        serde_yaml_ng::from_str(&raw)
            .unwrap_or_else(|e| panic!("fake-claude: invalid {SCENARIO_ENV} YAML {path}: {e}"))
    });

    let journal = Journal::from_env().ok();
    let rule = scenario
        .as_ref()
        .and_then(|s| select_response(s, &agent, call, &prompt));

    match rule {
        Some(rule) => {
            let exit = rule.exit_code.unwrap_or(0);
            if let Some(j) = &journal {
                let _ = j.record(&Call::from_invocation(
                    &inv, call, &agent, false, false, exit,
                ));
            }
            if let Some(ms) = rule.latency_ms {
                std::thread::sleep(std::time::Duration::from_millis(ms));
            }
            println!("{}", emit_claude_jsonl(rule));
            std::process::exit(exit);
        }
        None => {
            // No scenario (conformance probe) or no matching rule: journal a catch-all
            // and exit non-zero WITHOUT stream-json. This is what makes fake-claude usable
            // as the conformance kit's fake, and turns a missing catch-all rule into an
            // observable failed call rather than a panic.
            if let Some(j) = &journal {
                let _ = j.record(&Call::from_invocation(&inv, call, &agent, true, false, 127));
            }
            std::process::exit(127);
        }
    }
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
        let r = select_response(&s, "t-coord", 1, "prompt").unwrap();
        assert_eq!(r.respond, "@t-a: go");
        // fallback catch-all for another agent
        assert_eq!(select_response(&s, "t-x", 1, "p").unwrap().respond, "ok");
    }

    #[test]
    fn emits_parseable_claude_jsonl() {
        let r = &scen().rules[0];
        let out = emit_claude_jsonl(r);
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
            select_response(&s, "any", 1, "this has a special word")
                .unwrap()
                .respond,
            "special-path"
        );
        assert_eq!(
            select_response(&s, "any", 1, "plain prompt")
                .unwrap()
                .respond,
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
        let out = emit_claude_jsonl(&r);
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
    fn select_response_is_none_without_catch_all() {
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
        assert!(select_response(&s, "someone-else", 1, "p").is_none());
    }

    #[test]
    #[cfg(feature = "engine")]
    fn counter_increments_and_persists_per_key() {
        let dir = tempfile::tempdir().unwrap();
        let counter = Counter::new(dir.path());
        assert_eq!(counter.next("t-a").unwrap(), 1);
        assert_eq!(counter.next("t-a").unwrap(), 2);
        assert_eq!(counter.next("t-a").unwrap(), 3);
        // Independent counters per agent.
        assert_eq!(counter.next("t-b").unwrap(), 1);
    }
}
