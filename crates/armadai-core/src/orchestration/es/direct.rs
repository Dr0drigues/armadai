//! Event-sourced `direct` pattern (OH1 Lot 4): the simplest orchestration
//! pattern — a single agent is invoked once with the run's input, and its
//! response is the run's final answer. No delegation, no board, no ring.
//!
//! `DirectDecider` is the pure decision half: it mirrors the mock `D` decider
//! from `es::engine`'s own tests (invoke once, then complete), plus the
//! `latest:auto` routing bookkeeping every other pattern's decider already
//! performs. `DirectEffectRunner` is the sole async/impure half, built on the
//! same `"latest:auto"` → concrete-model resolution shared by
//! `HierarchicalEffectRunner`/`BlackboardEffectRunner`/`RingEffectRunner`
//! (duplicated here rather than factored out — same rationale as those:
//! each pattern's effect runner is an unrelated struct with no common trait
//! beyond `EffectRunner` itself).
//!
//! Coexists with the legacy single-agent path in `cli::run::run_single_agent`
//! — this module does not import from it, and `run_single_agent` is not
//! modified; wiring `run_direct_es` in as the active engine is a later lot
//! (the bascule).

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;

use super::engine::{Action, Decider, EffectRunner, run_event_sourced};
use super::event::ExecutionEvent;
use super::log::EventLog;
use super::state::ExecutionState;
use crate::agent::Agent;
use crate::model_resolution::{ModelTier, resolve_model_for_tier};
use crate::provider::{ChatMessage, CompletionRequest, Provider};
use crate::routing::{RoutingRules, route};

/// Parse a tier string as stored in `ExecutionState::routed_tiers` back into
/// a `ModelTier`. Identical in spirit to the same-named helper in
/// `es::hierarchical`/`es::blackboard`/`es::ring` — duplicated rather than
/// shared, matching those modules' documented rationale. Unrecognized
/// strings fall back to `Pro`.
fn parse_routed_tier(tier: &str) -> ModelTier {
    match tier.to_lowercase().as_str() {
        "fast" => ModelTier::Fast,
        "max" => ModelTier::Max,
        _ => ModelTier::Pro,
    }
}

/// Pure [`Decider`] for the `direct` pattern: invoke a single `agent` once
/// with `input`, then complete with its response.
///
/// All fields are immutable inputs captured at construction time. `decide`
/// performs no I/O and reads no mutable state — every decision is a pure
/// function of `state`.
#[derive(Debug, Clone)]
pub struct DirectDecider {
    /// The single agent this run invokes.
    pub agent: String,
    /// The original user input, given to `agent`.
    pub input: String,
    /// All known agents by name, for model/tag lookups (routing).
    pub agents: BTreeMap<String, Agent>,
    /// Routing rules for a `latest:auto` agent.
    pub routing_rules: RoutingRules,
}

impl DirectDecider {
    /// Construct a new `DirectDecider`. All arguments become immutable
    /// fields read by `decide`.
    pub fn new(
        agent: impl Into<String>,
        input: impl Into<String>,
        agents: BTreeMap<String, Agent>,
        routing_rules: RoutingRules,
    ) -> Self {
        Self {
            agent: agent.into(),
            input: input.into(),
            agents,
            routing_rules,
        }
    }

    /// If `self.agent` is a known agent configured with the exact
    /// `"latest:auto"` model placeholder, resolve the tier for `self.input`
    /// (via `crate::routing::route`, pure) and return the
    /// `ModelRouted` event to emit before invoking it. Concrete models,
    /// other `latest:*` placeholders, and unknown agents all return `None`.
    ///
    /// Mirrors `run_single_agent`'s own `latest:auto` routing call
    /// (`route(input, &agent.metadata.tags, None, routing_rules)` — no
    /// budget threading, since the direct pattern has no shared multi-agent
    /// budget to report) — only decides *which tier* to record; resolving a
    /// tier to a concrete model string is `DirectEffectRunner`'s job.
    fn model_routed_event(&self) -> Option<ExecutionEvent> {
        let agent_def = self.agents.get(&self.agent)?;
        let raw_model = agent_def.metadata.model.as_deref().unwrap_or("default");
        if raw_model != "latest:auto" {
            return None;
        }
        let (tier, reason) = route(
            &self.input,
            &agent_def.metadata.tags,
            None,
            &self.routing_rules,
        );
        Some(ExecutionEvent::ModelRouted {
            agent: self.agent.clone(),
            tier: format!("{tier:?}"),
            reason: format!("{reason:?}"),
        })
    }

    /// The agent's latest `assistant` response, if it has been observed yet.
    fn observed_response<'a>(&self, state: &'a ExecutionState) -> Option<&'a str> {
        state
            .conversations
            .get(&self.agent)?
            .iter()
            .rev()
            .find(|m| m.role == "assistant")
            .map(|m| m.content.as_str())
    }
}

impl Decider for DirectDecider {
    fn decide(&self, state: &ExecutionState) -> Vec<Action> {
        // The agent has already responded: the run is done, its response is
        // the final answer.
        if let Some(content) = self.observed_response(state) {
            return vec![Action::Complete {
                content: content.to_string(),
            }];
        }

        // Nothing has happened yet: invoke the agent with the run's input,
        // preceded by a `ModelRouted` bookkeeping event if it routes
        // `latest:auto`.
        let mut actions = Vec::new();
        if let Some(event) = self.model_routed_event() {
            actions.push(Action::Emit(event));
        }
        actions.push(Action::Invoke {
            agent: self.agent.clone(),
            input: self.input.clone(),
        });
        actions
    }
}

/// Executes the actual LLM call behind `Action::Invoke` for the `direct`
/// pattern and turns the raw provider response into the `AgentObserved`
/// event the pure loop/`DirectDecider` expect.
///
/// This is the *only* impure/async piece of the event-sourced direct
/// engine — `DirectDecider` above never touches I/O. Deliberately minimal
/// compared to `HierarchicalEffectRunner`: a single agent, a single turn, no
/// enriched orchestration-protocol system prompt, no multi-agent
/// conversation history to thread through.
pub struct DirectEffectRunner {
    /// All known agents by name (system prompt, model, temperature, …).
    pub agents: BTreeMap<String, Agent>,
    /// Provider instance per agent name.
    pub providers: BTreeMap<String, Arc<dyn Provider>>,
}

impl DirectEffectRunner {
    /// Construct a new `DirectEffectRunner` from its immutable inputs.
    pub fn new(
        agents: BTreeMap<String, Agent>,
        providers: BTreeMap<String, Arc<dyn Provider>>,
    ) -> Self {
        Self { agents, providers }
    }
}

#[async_trait]
impl EffectRunner for DirectEffectRunner {
    async fn run_invoke(
        &self,
        agent: &str,
        input: &str,
        state: &ExecutionState,
        _batch_len: usize,
    ) -> anyhow::Result<ExecutionEvent> {
        let agent_def = self
            .agents
            .get(agent)
            .ok_or_else(|| anyhow::anyhow!("Unknown agent '{agent}' — no Agent definition"))?;
        let provider = self
            .providers
            .get(agent)
            .ok_or_else(|| anyhow::anyhow!("No provider configured for agent '{agent}'"))?;

        // Same `"latest:auto"` resolution as the other pattern effect
        // runners: `DirectDecider` always emits `ModelRouted{agent, tier,
        // ..}` ahead of the matching `Invoke` for such an agent, which
        // `es::state::apply` projects into `state.routed_tiers`. We read
        // that tier back here and resolve it to a concrete model — the
        // `None` branch is a defensive fallback for a hand-built state
        // (tests) or a future decider regression.
        let raw_model = agent_def
            .metadata
            .model
            .clone()
            .unwrap_or_else(|| "default".to_string());
        let model = if raw_model == "latest:auto" {
            let tier = match state.routed_tiers.get(agent) {
                Some(tier_str) => parse_routed_tier(tier_str),
                None => {
                    tracing::warn!(
                        agent,
                        "no ModelRouted tier recorded for latest:auto agent; falling back to Pro tier"
                    );
                    ModelTier::Pro
                }
            };
            resolve_model_for_tier(&agent_def.metadata.provider, tier)
        } else {
            raw_model
        };

        let request = CompletionRequest {
            model,
            system_prompt: agent_def.system_prompt.clone(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: input.to_string(),
            }],
            temperature: agent_def.metadata.temperature,
            max_tokens: agent_def.metadata.max_tokens,
        };

        let response = provider.complete(request).await?;

        Ok(ExecutionEvent::AgentObserved {
            agent: agent.to_string(),
            content: response.content,
            tokens_in: response.tokens_in,
            tokens_out: response.tokens_out,
            cost: response.cost,
            model: response.model,
        })
    }
}

/// Run a complete `direct` orchestration end-to-end through the
/// event-sourced engine: builds the initial `RunStarted` event, constructs a
/// [`DirectDecider`] + [`DirectEffectRunner`] from `agent`/`agents`/
/// `providers`/`routing_rules`, and drives them through
/// [`run_event_sourced`], returning the final [`ExecutionState`].
///
/// `run_id` is accepted explicitly rather than generated internally, so
/// callers — notably tests proving replay determinism — can pass a fixed id
/// and later reconstruct the same state purely from the log via
/// [`super::engine::replay`].
///
/// Coexists with the legacy `cli::run::run_single_agent` — this function is
/// not called from `run.rs`; wiring it in as the active engine is a later
/// lot (the bascule).
pub async fn run_direct_es(
    run_id: &str,
    agent: &str,
    input: &str,
    agents: BTreeMap<String, Agent>,
    providers: BTreeMap<String, Arc<dyn Provider>>,
    routing_rules: RoutingRules,
    log: &mut impl EventLog,
) -> anyhow::Result<ExecutionState> {
    // Direct's roster is the single invoked agent — scope `roster_from_agents`
    // to just that key (rather than the whole `agents` map, which today only
    // ever holds that one entry anyway, see `dispatch_direct_es`) so
    // `RunStarted.roster` stays correct even if a future caller passes a
    // larger map in.
    let roster: BTreeMap<String, (String, String)> = agents
        .get(agent)
        .map(|a| {
            (
                agent.to_string(),
                (
                    a.metadata.provider.clone(),
                    a.metadata.model.clone().unwrap_or_default(),
                ),
            )
        })
        .into_iter()
        .collect();
    let initial = vec![ExecutionEvent::RunStarted {
        run_id: run_id.to_string(),
        pattern: "direct".to_string(),
        agents: vec![agent.to_string()],
        input: input.to_string(),
        project: None,
        roster,
    }];

    let decider = DirectDecider::new(agent, input, agents.clone(), routing_rules);
    let effects = DirectEffectRunner::new(agents, providers);

    run_event_sourced(run_id, initial, &decider, &effects, log).await
}

/// Resume a previously interrupted `direct` run (OH1 Lot 6, Task 3): recovers
/// the invoked agent's name and the run's original `input` from the log's
/// `RunStarted` event (see [`super::engine::run_started_roster_and_input`] —
/// `direct`'s `RunStarted.agents` is always a single-element roster, the
/// invoked agent), rebuilds the SAME [`DirectDecider`]/[`DirectEffectRunner`]
/// pair [`run_direct_es`] would have from a fresh `agents`/`providers`
/// reload (the caller is expected to have re-parsed the agent from the
/// project on disk, exactly as a live run does — the log carries no `Agent`
/// definitions, only the roster's key), and drives
/// [`super::engine::resume_event_sourced`] instead of appending a fresh
/// `RunStarted`.
///
/// Bails if `run_id` has no recorded `RunStarted` (unknown run) or isn't
/// currently `Running` (see `resume_event_sourced`).
pub async fn resume_direct_es(
    run_id: &str,
    agents: BTreeMap<String, Agent>,
    providers: BTreeMap<String, Arc<dyn Provider>>,
    routing_rules: RoutingRules,
    log: &mut impl EventLog,
) -> anyhow::Result<ExecutionState> {
    use super::engine::{resume_event_sourced, run_started_roster_and_input};

    let events = log.events(run_id)?;
    let (roster, input) = run_started_roster_and_input(&events)
        .ok_or_else(|| anyhow::anyhow!("no run found for id {run_id}"))?;
    let agent = roster
        .first()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("run {run_id} has an empty agent roster"))?;

    let decider = DirectDecider::new(&agent, &input, agents.clone(), routing_rules);
    let effects = DirectEffectRunner::new(agents, providers);

    resume_event_sourced(run_id, &decider, &effects, log).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::AgentMetadata;
    use crate::model_resolution::fallback_model_for_tier;
    use crate::orchestration::es::log::InMemoryLog;
    use crate::orchestration::es::state::RunStatus;
    use crate::provider::{CompletionResponse, ProviderMetadata, TokenStream};
    use std::path::PathBuf;
    use std::sync::Mutex;

    /// Minimal `Agent` for direct-pattern tests. `model` controls whether
    /// this agent routes through `latest:auto` (pass `"latest:auto"`) or
    /// uses a concrete model string as-is.
    fn test_agent(name: &str, model: &str) -> Agent {
        Agent {
            name: name.to_string(),
            source: PathBuf::from(format!("{name}.md")),
            metadata: AgentMetadata {
                provider: "anthropic".to_string(),
                model: Some(model.to_string()),
                command: None,
                args: None,
                temperature: 0.7,
                max_tokens: None,
                timeout: None,
                tags: vec![],
                stacks: vec![],
                scope: vec![],
                model_fallback: vec![],
                cost_limit: None,
                rate_limit: None,
                context_window: None,
                mode: None,
                orchestration: None,
                triggers: None,
                ring_config: None,
            },
            system_prompt: format!("You are {name}."),
            instructions: None,
            output_format: None,
            pipeline: None,
            context: None,
        }
    }

    /// Provider that records every `CompletionRequest` it receives and
    /// always answers with a fixed `response` — mirrors `CapturingProvider`
    /// in `es::hierarchical`/`es::blackboard`/`es::ring`.
    struct CapturingProvider {
        requests: Mutex<Vec<CompletionRequest>>,
        response: String,
    }

    impl CapturingProvider {
        fn new(response: &str) -> Self {
            Self {
                requests: Mutex::new(Vec::new()),
                response: response.to_string(),
            }
        }

        fn requests(&self) -> Vec<CompletionRequest> {
            self.requests.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl Provider for CapturingProvider {
        async fn complete(&self, request: CompletionRequest) -> anyhow::Result<CompletionResponse> {
            let model = request.model.clone();
            self.requests.lock().unwrap().push(request);
            Ok(CompletionResponse {
                content: self.response.clone(),
                model,
                tokens_in: 3,
                tokens_out: 4,
                cost: 0.02,
            })
        }
        async fn stream(&self, _request: CompletionRequest) -> anyhow::Result<TokenStream> {
            anyhow::bail!("streaming not exercised by run_direct_es tests")
        }
        fn metadata(&self) -> ProviderMetadata {
            ProviderMetadata {
                name: "capturing".to_string(),
                models: vec![],
                supports_streaming: false,
            }
        }
    }

    fn event_kinds(log: &InMemoryLog, run_id: &str) -> Vec<&'static str> {
        log.events(run_id)
            .unwrap()
            .iter()
            .map(|e| match e {
                ExecutionEvent::RunStarted { .. } => "run_started",
                ExecutionEvent::AgentInvoked { .. } => "agent_invoked",
                ExecutionEvent::AgentObserved { .. } => "agent_observed",
                ExecutionEvent::ModelRouted { .. } => "model_routed",
                ExecutionEvent::Completed { .. } => "completed",
                _ => "other",
            })
            .collect()
    }

    // Step 1 (brief): `run_direct_es` with a concrete-model agent → log
    // RunStarted -> AgentInvoked -> AgentObserved -> Completed, status
    // Completed, content = the mock provider's response.
    #[tokio::test]
    async fn run_direct_es_completes_with_mock_response() {
        let mut agents = BTreeMap::new();
        agents.insert("solo".to_string(), test_agent("solo", "concrete-model"));
        let capturing = Arc::new(CapturingProvider::new("the answer"));
        let mut providers: BTreeMap<String, Arc<dyn Provider>> = BTreeMap::new();
        providers.insert("solo".to_string(), capturing.clone() as Arc<dyn Provider>);

        let mut log = InMemoryLog::default();
        let st = run_direct_es(
            "run-direct",
            "solo",
            "do the thing",
            agents,
            providers,
            RoutingRules::default(),
            &mut log,
        )
        .await
        .unwrap();

        assert_eq!(st.status, RunStatus::Completed);
        assert_eq!(
            event_kinds(&log, "run-direct"),
            vec![
                "run_started",
                "agent_invoked",
                "agent_observed",
                "completed"
            ]
        );

        let sent = capturing.requests();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].model, "concrete-model");
        assert!(
            sent[0]
                .messages
                .iter()
                .any(|m| m.role == "user" && m.content == "do the thing")
        );

        let final_content = log
            .events("run-direct")
            .unwrap()
            .into_iter()
            .find_map(|e| match e {
                ExecutionEvent::Completed { content } => Some(content),
                _ => None,
            })
            .unwrap();
        assert_eq!(final_content, "the answer");

        // Replay reconstructs the same state purely from the log.
        let replayed = super::super::engine::replay("run-direct", &log).unwrap();
        assert_eq!(format!("{st:?}"), format!("{replayed:?}"));
    }

    /// OH1 Lot 6, Task 3: a `direct` run interrupted right after
    /// `RunStarted` (crashed before ever invoking the agent) resumes to
    /// completion via `resume_direct_es`, invoking the provider exactly
    /// once — same end-to-end shape `run_direct_es` produces, but starting
    /// from a log that already has a `RunStarted` recorded.
    #[tokio::test]
    async fn resume_direct_es_completes_a_run_interrupted_before_any_invoke() {
        let mut agents = BTreeMap::new();
        agents.insert("solo".to_string(), test_agent("solo", "concrete-model"));
        let capturing = Arc::new(CapturingProvider::new("the answer"));
        let mut providers: BTreeMap<String, Arc<dyn Provider>> = BTreeMap::new();
        providers.insert("solo".to_string(), capturing.clone() as Arc<dyn Provider>);

        let mut log = InMemoryLog::default();
        // Simulate a crash: only `RunStarted` was persisted before the
        // process died — the agent was never invoked.
        log.append(
            "run-direct",
            &ExecutionEvent::RunStarted {
                run_id: "run-direct".to_string(),
                pattern: "direct".to_string(),
                agents: vec!["solo".to_string()],
                input: "do the thing".to_string(),
                project: None,
                roster: Default::default(),
            },
        )
        .unwrap();

        let st = resume_direct_es(
            "run-direct",
            agents,
            providers,
            RoutingRules::default(),
            &mut log,
        )
        .await
        .unwrap();

        assert_eq!(st.status, RunStatus::Completed);
        assert_eq!(capturing.requests().len(), 1);
        assert_eq!(
            event_kinds(&log, "run-direct"),
            vec![
                "run_started",
                "agent_invoked",
                "agent_observed",
                "completed"
            ]
        );
    }

    /// OH1 Lot 6, Task 3: a `direct` run interrupted AFTER the agent
    /// answered (crashed between `AgentObserved` and `Completed`) resumes to
    /// completion WITHOUT calling the provider again — the already-observed
    /// response from the log is reused, proving `resume_direct_es` doesn't
    /// re-invoke.
    #[tokio::test]
    async fn resume_direct_es_does_not_reinvoke_an_already_observed_agent() {
        let mut agents = BTreeMap::new();
        agents.insert("solo".to_string(), test_agent("solo", "concrete-model"));
        let capturing = Arc::new(CapturingProvider::new("should never be sent"));
        let mut providers: BTreeMap<String, Arc<dyn Provider>> = BTreeMap::new();
        providers.insert("solo".to_string(), capturing.clone() as Arc<dyn Provider>);

        let mut log = InMemoryLog::default();
        log.append(
            "run-direct",
            &ExecutionEvent::RunStarted {
                run_id: "run-direct".to_string(),
                pattern: "direct".to_string(),
                agents: vec!["solo".to_string()],
                input: "do the thing".to_string(),
                project: None,
                roster: Default::default(),
            },
        )
        .unwrap();
        log.append(
            "run-direct",
            &ExecutionEvent::AgentInvoked {
                agent: "solo".to_string(),
                input: "do the thing".to_string(),
            },
        )
        .unwrap();
        log.append(
            "run-direct",
            &ExecutionEvent::AgentObserved {
                agent: "solo".to_string(),
                content: "already answered".to_string(),
                tokens_in: 3,
                tokens_out: 4,
                cost: 0.02,
                model: "concrete-model".to_string(),
            },
        )
        .unwrap();

        let st = resume_direct_es(
            "run-direct",
            agents,
            providers,
            RoutingRules::default(),
            &mut log,
        )
        .await
        .unwrap();

        assert_eq!(st.status, RunStatus::Completed);
        // The provider was never called: the response came from the log.
        assert!(capturing.requests().is_empty());
        let final_content = log
            .events("run-direct")
            .unwrap()
            .into_iter()
            .find_map(|e| match e {
                ExecutionEvent::Completed { content } => Some(content),
                _ => None,
            })
            .unwrap();
        assert_eq!(final_content, "already answered");
    }

    #[tokio::test]
    async fn resume_direct_es_bails_on_completed_run() {
        let mut log = InMemoryLog::default();
        log.append(
            "run-direct",
            &ExecutionEvent::RunStarted {
                run_id: "run-direct".to_string(),
                pattern: "direct".to_string(),
                agents: vec!["solo".to_string()],
                input: "do the thing".to_string(),
                project: None,
                roster: Default::default(),
            },
        )
        .unwrap();
        log.append(
            "run-direct",
            &ExecutionEvent::Completed {
                content: "done".to_string(),
            },
        )
        .unwrap();

        let err = resume_direct_es(
            "run-direct",
            BTreeMap::new(),
            BTreeMap::new(),
            RoutingRules::default(),
            &mut log,
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("not resumable"));
    }

    #[tokio::test]
    async fn resume_direct_es_bails_on_unknown_run() {
        let mut log = InMemoryLog::default();
        let err = resume_direct_es(
            "nope",
            BTreeMap::new(),
            BTreeMap::new(),
            RoutingRules::default(),
            &mut log,
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("no run found"));
    }

    // `latest:auto` must route: a `ModelRouted` event is emitted before the
    // `Invoke`, and the effect runner resolves it to a concrete model
    // (never leaking the literal `"latest:auto"` string to the provider).
    //
    // The agent's provider is deliberately a name no real `models.dev`
    // cache on the machine running this test could ever contain, so
    // `resolve_model_for_tier` is forced onto its hardcoded, pure
    // `fallback_model_for_tier` path — keeping this test hermetic (same
    // rationale as the equivalent hierarchical/blackboard/ring tests).
    #[tokio::test]
    async fn run_direct_es_routes_latest_auto_and_resolves_concrete_model() {
        let mut agent = test_agent("solo", "latest:auto");
        agent.metadata.provider = "test-only-uncached-provider".to_string();
        let mut agents = BTreeMap::new();
        agents.insert("solo".to_string(), agent);
        let capturing = Arc::new(CapturingProvider::new("routed answer"));
        let mut providers: BTreeMap<String, Arc<dyn Provider>> = BTreeMap::new();
        providers.insert("solo".to_string(), capturing.clone() as Arc<dyn Provider>);

        let mut log = InMemoryLog::default();
        let st = run_direct_es(
            "run-direct-auto",
            "solo",
            "do the thing",
            agents,
            providers,
            RoutingRules::default(),
            &mut log,
        )
        .await
        .unwrap();

        assert_eq!(st.status, RunStatus::Completed);
        assert_eq!(
            event_kinds(&log, "run-direct-auto"),
            vec![
                "run_started",
                "model_routed",
                "agent_invoked",
                "agent_observed",
                "completed"
            ]
        );

        // The tier `route()` picks for this short input over the default
        // rules/tags — read it back from the log so the expected fallback
        // model doesn't hardcode a routing decision this test doesn't own.
        let tier_str = log
            .events("run-direct-auto")
            .unwrap()
            .into_iter()
            .find_map(|e| match e {
                ExecutionEvent::ModelRouted { tier, .. } => Some(tier),
                _ => None,
            })
            .unwrap();
        let tier = parse_routed_tier(&tier_str);
        let expected = fallback_model_for_tier("test-only-uncached-provider", tier).to_string();

        let sent = capturing.requests();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].model, expected);
        assert_ne!(sent[0].model, "latest:auto");
    }

    #[tokio::test]
    async fn run_invoke_errors_for_unknown_agent() {
        let runner = DirectEffectRunner::new(BTreeMap::new(), BTreeMap::new());
        let state = ExecutionState::default();
        let err = runner
            .run_invoke("missing", "go", &state, 1)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("missing"));
    }

    #[tokio::test]
    async fn run_invoke_errors_when_provider_missing_for_known_agent() {
        let mut agents = BTreeMap::new();
        agents.insert("solo".to_string(), test_agent("solo", "concrete-model"));
        let runner = DirectEffectRunner::new(agents, BTreeMap::new());
        let state = ExecutionState::default();
        let err = runner
            .run_invoke("solo", "go", &state, 1)
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("provider") && msg.contains("'solo'"),
            "expected a distinctive missing-provider message, got: {msg}"
        );
    }

    #[test]
    fn decide_invokes_then_completes_with_observed_content() {
        let mut agents = BTreeMap::new();
        agents.insert("solo".to_string(), test_agent("solo", "concrete-model"));
        let decider = DirectDecider::new("solo", "go", agents, RoutingRules::default());

        // Nothing has happened yet: invoke once, no ModelRouted (concrete
        // model).
        let state = super::super::state::fold(&[ExecutionEvent::RunStarted {
            run_id: "r".into(),
            pattern: "direct".into(),
            agents: vec!["solo".into()],
            input: "go".into(),
            project: None,
            roster: Default::default(),
        }]);
        let actions = decider.decide(&state);
        assert_eq!(actions.len(), 1);
        assert!(
            matches!(&actions[0], Action::Invoke { agent, input } if agent == "solo" && input == "go")
        );

        // The agent has been observed: complete with its response.
        let state = super::super::state::fold(&[
            ExecutionEvent::RunStarted {
                run_id: "r".into(),
                pattern: "direct".into(),
                agents: vec!["solo".into()],
                input: "go".into(),
                project: None,
                roster: Default::default(),
            },
            ExecutionEvent::AgentInvoked {
                agent: "solo".into(),
                input: "go".into(),
            },
            ExecutionEvent::AgentObserved {
                agent: "solo".into(),
                content: "done".into(),
                tokens_in: 1,
                tokens_out: 1,
                cost: 0.0,
                model: "m".into(),
            },
        ]);
        let actions = decider.decide(&state);
        assert_eq!(actions.len(), 1);
        assert!(matches!(&actions[0], Action::Complete { content } if content == "done"));
    }

    #[test]
    fn decide_emits_model_routed_before_invoke_for_latest_auto() {
        let mut agents = BTreeMap::new();
        agents.insert("solo".to_string(), test_agent("solo", "latest:auto"));
        let decider = DirectDecider::new("solo", "go", agents, RoutingRules::default());

        let state = super::super::state::fold(&[ExecutionEvent::RunStarted {
            run_id: "r".into(),
            pattern: "direct".into(),
            agents: vec!["solo".into()],
            input: "go".into(),
            project: None,
            roster: Default::default(),
        }]);
        let actions = decider.decide(&state);
        assert_eq!(actions.len(), 2);
        assert!(
            matches!(&actions[0], Action::Emit(ExecutionEvent::ModelRouted { agent, .. }) if agent == "solo")
        );
        assert!(matches!(&actions[1], Action::Invoke { agent, .. } if agent == "solo"));
    }
}
