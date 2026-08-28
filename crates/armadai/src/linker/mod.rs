mod claude;
mod codex;
mod copilot;
mod gemini;
pub mod manifest;
pub mod model_resolution;
mod opencode;

pub use claude::ClaudeLinker;
pub use codex::CodexLinker;
pub use copilot::CopilotLinker;
pub use gemini::GeminiLinker;
pub use opencode::OpencodeLinker;

use std::path::PathBuf;

use armadai_core::agent::Agent;

/// Supported link targets for autocompletion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum LinkTarget {
    Claude,
    Codex,
    Copilot,
    Gemini,
    Opencode,
}

impl LinkTarget {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Copilot => "copilot",
            Self::Gemini => "gemini",
            Self::Opencode => "opencode",
        }
    }
}

impl std::fmt::Display for LinkTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A resolved agent ready for linking.
#[allow(dead_code)]
pub struct LinkAgent {
    pub name: String,
    pub system_prompt: String,
    pub instructions: Option<String>,
    pub output_format: Option<String>,
    pub context: Option<String>,
    pub description: Option<String>,
    pub tags: Vec<String>,
    pub stacks: Vec<String>,
    pub scope: Vec<String>,
    pub model: Option<String>,
    pub model_fallback: Vec<String>,
    pub temperature: f32,
    pub provider: Option<String>,
}

/// A file to be written by a linker.
pub struct OutputFile {
    pub path: PathBuf,
    pub content: String,
}

/// Trait for generating target-specific config files.
#[allow(dead_code)]
pub trait Linker: Send + Sync {
    fn name(&self) -> &str;
    fn default_output_dir(&self) -> &str;
    fn generate(
        &self,
        agents: &[LinkAgent],
        coordinator: Option<&LinkAgent>,
        sources: &[String],
    ) -> Vec<OutputFile>;
}

/// Every target `create_linker` can build, as a comma-separated list for an
/// error message.
///
/// Derived from `LinkTarget` rather than typed out. Three separate messages
/// (`create_linker`, `link`, `unlink`) each carried their own copy of the
/// five names — the same shape as the `Unknown provider` message that had
/// silently stopped advertising `proxy` (issue #369). A target added to the
/// enum now reaches all three at once.
pub fn supported_target_names() -> Vec<&'static str> {
    use clap::ValueEnum;
    LinkTarget::value_variants()
        .iter()
        .map(|t| t.as_str())
        .collect()
}

/// The sentence every "no/unknown target" message ends with.
pub fn supported_targets_sentence() -> String {
    format!("Supported targets: {}", supported_target_names().join(", "))
}

/// Create a linker for the given target name.
pub fn create_linker(target: &str) -> anyhow::Result<Box<dyn Linker>> {
    match target {
        "claude" => Ok(Box::new(ClaudeLinker)),
        "codex" => Ok(Box::new(CodexLinker)),
        "copilot" => Ok(Box::new(CopilotLinker)),
        "gemini" => Ok(Box::new(GeminiLinker)),
        "opencode" => Ok(Box::new(OpencodeLinker)),
        _ => anyhow::bail!(
            "Unknown link target: '{target}'. {}",
            supported_targets_sentence()
        ),
    }
}

/// Convert an agent name to a kebab-case slug suitable for filenames.
///
/// Moved to `armadai_core::agent` — normalising a name into a filename-safe
/// slug is agent-domain-model territory, and `agent_source::shadowing_conflict`
/// needs the exact same normalisation to catch a declaration/file collision
/// that only matches after folding. Re-exported here so no call site in this
/// crate has to change.
pub use armadai_core::agent::slugify;

/// The coordinator a command is working from: the reference itself, and
/// the field that supplied it.
///
/// This type exists to keep two questions apart that a single
/// `take_coordinator` used to answer at once, and wrongly:
///
/// - **"Does this reference name an agent the project declares?"** — a
///   statement about the *configuration*, answered by
///   [`Self::no_match_warning`] against the declared roster.
/// - **"Which agent of the roster I am about to write is it?"** — the
///   resolver's own job, answered by [`Self::take_from`] against whatever
///   roster the command is writing.
///
/// Deriving the first from the second is what shipped
/// `armadai link --agents Worker` printing
/// `link.coordinator 'dev-lead' matches no agent` on a project where
/// `dev-lead` matches `Dev Lead` perfectly — the user's own filter had
/// removed it from the roster the resolver searched, and the message then
/// sent them to fix an H1 title that was never wrong. `armadai validate`,
/// which asks the first question directly, reported no warning at all on
/// the same project: two surfaces contradicting each other over one
/// config key.
///
/// The write behaviour is not the same question and does not change with
/// it: with `--agents Worker`, `Dev Lead` is genuinely not among the
/// agents being written, so no root instructions file is written or
/// removed for it (pinned by
/// `tests/link_manifest.rs::unlink_agents_filter_leaves_the_coordinator_and_unmatched_agents_alone`).
/// Only the message was wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoordinatorRef {
    /// The field the reference came from — `link.coordinator` for the
    /// project config, `--coordinator` for the CLI flag that overrides it.
    /// Reported verbatim, so the user is sent to the field they actually
    /// wrote.
    pub origin: &'static str,
    /// The reference as configured, before any slug folding.
    pub reference: String,
}

/// Which coordinator reference is in force — the CLI flag when given,
/// otherwise the project config, otherwise none.
///
/// One priority rule, in one place, for the three callers (`link`,
/// `unlink`, `armadai shell`'s setup wizard) that used to spell it out
/// themselves. `None` — no coordinator configured at all — is the normal
/// case for most projects and warrants nothing.
pub fn coordinator_reference(
    coordinator_flag: Option<String>,
    configured: Option<String>,
) -> Option<CoordinatorRef> {
    match (coordinator_flag, configured) {
        (Some(flag), _) => Some(CoordinatorRef {
            origin: "--coordinator",
            reference: flag,
        }),
        (None, Some(from_config)) => Some(CoordinatorRef {
            origin: armadai_core::agent::LINK_COORDINATOR_KEY,
            reference: from_config,
        }),
        (None, None) => None,
    }
}

impl CoordinatorRef {
    /// What to tell the user when this reference designates no agent of
    /// `declared_roster` (issue #371), or `None` when it designates one.
    ///
    /// `declared_roster` must be the project's roster as loaded, **never**
    /// a `--agents`-filtered subset — see this type's own doc. The
    /// wording, and the decision of whether there is anything to say at
    /// all, both live in
    /// [`armadai_core::agent::coordinator_no_match_warning`], which
    /// `validate`'s R7 calls directly: one question, one answer, four
    /// surfaces.
    pub fn no_match_warning(&self, declared_roster: &[LinkAgent]) -> Option<String> {
        let titles: Vec<String> = declared_roster.iter().map(|a| a.name.clone()).collect();
        armadai_core::agent::coordinator_no_match_warning(self.origin, &self.reference, &titles)
    }

    /// Take the agent this reference designates out of `link_agents`.
    ///
    /// Removing it from the roster is what turns it into the target's root
    /// instructions file instead of one more per-agent file, so a caller
    /// that skipped this would write both. Matching goes through
    /// [`armadai_core::agent::name_matches_reference`] — by title *or* by
    /// that title's slug. This replaced a re-export of that primitive on
    /// which each caller wrote its own three-line resolution: two of those
    /// copies had already drifted apart once (issue #341: `link` matched
    /// by name or slug, `unlink` by name alone, and the root context file
    /// stayed on disk unnamed), and a third appeared in the shell wizard.
    ///
    /// Returning `None` here is **not** a defect to report: a roster that
    /// legitimately excludes the coordinator (a `--agents` filter that
    /// does not name it) reaches this exact branch. Reporting is
    /// [`Self::no_match_warning`]'s job, on the declared roster.
    pub fn take_from(&self, link_agents: &mut Vec<LinkAgent>) -> Option<LinkAgent> {
        let idx = link_agents
            .iter()
            .position(|a| armadai_core::agent::name_matches_reference(&a.name, &self.reference))?;
        Some(link_agents.remove(idx))
    }
}

impl From<&Agent> for LinkAgent {
    fn from(agent: &Agent) -> Self {
        let description = agent
            .system_prompt
            .lines()
            .find(|l| !l.trim().is_empty())
            .map(|l| l.trim().to_string());

        Self {
            name: agent.name.clone(),
            system_prompt: agent.system_prompt.clone(),
            instructions: agent.instructions.clone(),
            output_format: agent.output_format.clone(),
            context: agent.context.clone(),
            description,
            tags: agent.metadata.tags.clone(),
            stacks: agent.metadata.stacks.clone(),
            scope: agent.metadata.scope.clone(),
            model: agent.metadata.model.clone(),
            model_fallback: agent.metadata.model_fallback.clone(),
            temperature: agent.metadata.temperature,
            provider: Some(agent.metadata.provider.clone()),
        }
    }
}

impl LinkAgent {
    /// This agent's sections composed into one prompt body, through the same
    /// core definition `run` uses (`armadai_core::agent::compose_agent_prompt`).
    ///
    /// The five linkers each carried their own copy of this loop until #395,
    /// and `run` carried none — which is how the same agent came to behave
    /// differently depending on the path that executed it. Sharing the
    /// composition is what makes "same order, same separators" a fact rather
    /// than a convention.
    pub fn composed_prompt(&self) -> String {
        armadai_core::agent::compose_agent_prompt(
            &self.system_prompt,
            self.instructions.as_deref(),
            self.output_format.as_deref(),
            self.context.as_deref(),
        )
    }
}

/// Protocol block appended to linked config files for ArmadAI shell parsing.
pub fn armadai_protocol_block() -> &'static str {
    r"

## ArmadAI Response Protocol

Follow this protocol for all responses:

1. When finished responding, end with this marker on its own line:
   <!--ARMADAI_END-->

2. When delegating to a sub-agent, prefix with:
   <!--ARMADAI_DELEGATE:agent-name-->

3. Before the END marker, include metadata:
   <!--ARMADAI_META:status=complete-->
"
}

#[cfg(test)]
mod tests {
    use super::*;
    use armadai_core::agent::AgentMetadata;

    /// [`CoordinatorRef`]'s own branches, including the two the black-box
    /// suite (`tests/link_coordinator_no_match.rs`) cannot reach: the
    /// `--coordinator` flag's priority over the config, and the origin
    /// label that flag puts in the message.
    mod coordinator_ref_tests {
        use super::*;

        fn roster() -> Vec<LinkAgent> {
            ["Worker", "Dev Lead", "QA"]
                .iter()
                .map(|name| LinkAgent {
                    name: (*name).to_string(),
                    system_prompt: format!("I am {name}."),
                    instructions: None,
                    output_format: None,
                    context: None,
                    description: None,
                    tags: Vec::new(),
                    stacks: Vec::new(),
                    scope: Vec::new(),
                    model: None,
                    model_fallback: Vec::new(),
                    temperature: 0.7,
                    provider: None,
                })
                .collect()
        }

        /// The match is by title *or* slug, and the matched agent leaves
        /// the roster — that removal is what makes it the root
        /// instructions file instead of one more per-agent file.
        ///
        /// `Dev Lead` sits at index 1 on purpose: with the coordinator at
        /// index 0 this assertion also holds under a predicate that
        /// always matches, which is exactly the fixture defect measured
        /// on #370.
        ///
        /// Mutation: replacing `name_matches_reference` with `==` (the
        /// `Dev Lead`/`dev-lead` pair fails it) or with an always-true
        /// closure (which takes `Worker`) fails this test.
        #[test]
        fn a_slug_reference_resolves_the_titled_agent_and_removes_it() {
            let mut agents = roster();
            let coordinator = coordinator_reference(None, Some("dev-lead".to_string()))
                .expect("a configured reference");

            assert_eq!(
                coordinator.no_match_warning(&agents),
                None,
                "a resolved coordinator warrants no message"
            );
            assert_eq!(
                coordinator.take_from(&mut agents).map(|c| c.name),
                Some("Dev Lead".to_string())
            );
            assert_eq!(
                agents.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(),
                vec!["Worker", "QA"],
                "the coordinator must leave the roster"
            );
        }

        /// No reference at all is the normal case for most projects: no
        /// coordinator, and nothing to say about it.
        ///
        /// Mutation: returning `Some(CoordinatorRef { .. })` for the
        /// `(None, None)` arm fails this test.
        #[test]
        fn no_reference_at_all_is_silent() {
            assert_eq!(coordinator_reference(None, None), None);
        }

        /// Issue #371: a reference that designates nobody must produce a
        /// message naming the key at fault, the H1-title namespace it is
        /// matched in, and the titles actually available.
        ///
        /// Mutation: having `coordinator_no_match_warning` always return
        /// `None` — the pre-fix behaviour — fails every assertion below.
        #[test]
        fn an_unmatched_config_reference_reports_the_config_key() {
            let mut agents = roster();
            let coordinator = coordinator_reference(None, Some("dev-led".to_string()))
                .expect("a configured reference");

            let message = coordinator
                .no_match_warning(&agents)
                .expect("an unmatched reference must be reported");
            assert!(
                message.starts_with("link.coordinator 'dev-led' matches no agent"),
                "must name the config key and the value: {message}"
            );
            assert!(
                message.contains("H1 title"),
                "must send the user to the title namespace, not the `agents:` key: {message}"
            );
            assert!(
                message.contains("Titles in this roster: Worker, Dev Lead, QA."),
                "must list what was actually available: {message}"
            );
            assert!(coordinator.take_from(&mut agents).is_none());
            assert_eq!(agents.len(), 3, "nothing is removed when nothing matched");
        }

        /// The flag wins over the config — one priority rule, in one
        /// place, for all three callers — and the message must then name
        /// the flag, not the config key the user did not get wrong.
        ///
        /// Mutation: reversing the priority resolves `Dev Lead` (the
        /// config value matches) and returns no warning at all, failing
        /// both assertions.
        #[test]
        fn the_flag_wins_over_the_config_and_names_itself() {
            let mut agents = roster();
            let coordinator =
                coordinator_reference(Some("nobody".to_string()), Some("dev-lead".to_string()))
                    .expect("a configured reference");

            assert!(
                coordinator.take_from(&mut agents).is_none(),
                "the flag's value is what gets resolved, and it matches nothing"
            );
            let message = coordinator
                .no_match_warning(&agents)
                .expect("an unmatched flag must be reported too");
            assert!(
                message.starts_with("--coordinator 'nobody' matches no agent"),
                "must name the flag, not the config key: {message}"
            );
        }

        /// **The D1 defect, at the unit that carries it.** The two
        /// questions must not answer each other: a roster narrowed by
        /// `--agents` legitimately excludes the coordinator, and
        /// `take_from` then finds nothing — but the *configuration* is
        /// still correct, so `no_match_warning` on the declared roster
        /// must stay silent. Deriving one from the other is what shipped
        /// `link --agents Worker` claiming `dev-lead` matched no agent.
        ///
        /// Mutation: making `no_match_warning` answer against the roster
        /// `take_from` is given (the pre-fix coupling) turns this green
        /// assertion into a `Some(_)`.
        #[test]
        fn a_filtered_out_coordinator_is_a_missing_agent_not_a_broken_config() {
            let declared = roster();
            // What `link`'s step 3 does to the roster: `--agents Worker`.
            let mut filtered = roster();
            filtered.retain(|a| a.name == "Worker");
            let coordinator = coordinator_reference(None, Some("dev-lead".to_string()))
                .expect("a configured reference");

            assert_eq!(
                coordinator.no_match_warning(&declared),
                None,
                "the declared roster satisfies the reference, so there is nothing to report"
            );
            assert!(
                coordinator.take_from(&mut filtered).is_none(),
                "the filtered roster has no coordinator to hand over, and that is not a defect"
            );
            assert_eq!(
                filtered.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(),
                vec!["Worker"],
                "a reference that resolves to nobody here removes nobody"
            );
        }

        /// The other half of D1: a filter must not *silence* a genuine
        /// defect either. `dev-led` is a typo whatever `--agents` says, and
        /// the titles listed are the declared ones — the user is told what
        /// their project actually offers, not what this invocation kept.
        ///
        /// Mutation: answering against the filtered roster still warns
        /// (the reference matches nothing there either), but lists
        /// `Titles in this roster: Worker.` and fails the last assertion.
        #[test]
        fn a_filter_does_not_silence_a_genuinely_unmatched_reference() {
            let declared = roster();
            let coordinator = coordinator_reference(None, Some("dev-led".to_string()))
                .expect("a configured reference");

            let message = coordinator
                .no_match_warning(&declared)
                .expect("a typo stays a typo under any filter");
            assert!(
                message.contains("Titles in this roster: Worker, Dev Lead, QA."),
                "the titles offered must be the declared ones, not the filtered ones: {message}"
            );
        }
    }

    /// The three provider inventories, checked against each other.
    ///
    /// They live in three crates and drifted apart unnoticed until a user hit
    /// it (issue #369): `armadai link --target codex` wrote a config, the
    /// shell relayed `codex` in JSON, and `armadai run` answered
    /// `Unknown provider: 'codex'`. A comment saying "keep these in sync"
    /// is what was there before; this is the same statement, executable.
    mod provider_inventories {
        use super::*;
        use armadai_core::test_support::IsolatedConfigDir;
        use armadai_providers::factory::{accepted_provider_names, create_provider};
        use armadai_providers::json_runner::json_capable_clis;
        use clap::ValueEnum;

        /// An agent whose `provider:` is under test and whose `command:`
        /// exists everywhere, so the CLI branch is reached regardless of
        /// which tools happen to be installed on the machine.
        fn agent_with_provider(provider: &str) -> Agent {
            Agent {
                name: "inventory-probe".into(),
                source: std::path::PathBuf::from("probe.md"),
                metadata: AgentMetadata {
                    provider: provider.into(),
                    model: None,
                    command: Some("echo".into()),
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
                system_prompt: String::new(),
                instructions: None,
                output_format: None,
                pipeline: None,
                context: None,
            }
        }

        fn sorted(mut v: Vec<&str>) -> Vec<&str> {
            v.sort_unstable();
            v
        }

        /// Inventory 1 (`linker::LinkTarget`) vs inventory 2
        /// (`factory::KNOWN_TOOLS`): writing a native config for a CLI is
        /// worth nothing if `armadai run` then refuses the same name.
        ///
        /// `value_variants()` comes from the `ValueEnum` derive, so a target
        /// added to the enum is in this loop without anyone remembering to
        /// add it.
        #[test]
        fn every_link_target_is_a_provider_run_can_execute() {
            // `create_provider` reads the user config for rate limits; keep
            // it off the developer's real one.
            let _cfg = IsolatedConfigDir::enter();
            for target in LinkTarget::value_variants() {
                let name = target.as_str();
                let provider = create_provider(&agent_with_provider(name)).unwrap_or_else(|e| {
                    panic!(
                        "`armadai link --target {name}` writes a config for a CLI that \
                         `armadai run` refuses: {e}"
                    )
                });
                assert_eq!(
                    provider.metadata().name,
                    "cli:echo",
                    "link target '{name}' must resolve to the CLI branch"
                );
            }
        }

        /// Inventory 3 (`json_runner`, the shell relay) vs inventory 2: a CLI
        /// the shell can drive in JSON must be a name `armadai run` accepts,
        /// or the same agent works in `armadai shell` and fails in
        /// `armadai run`.
        #[test]
        fn every_json_relayable_cli_is_a_provider_run_can_execute() {
            let _cfg = IsolatedConfigDir::enter();
            for name in json_capable_clis() {
                let provider = create_provider(&agent_with_provider(name)).unwrap_or_else(|e| {
                    panic!("the shell relays '{name}' in JSON but `armadai run` refuses it: {e}")
                });
                assert_eq!(provider.metadata().name, "cli:echo", "'{name}'");
            }
        }

        /// Inventory 1 vs inventory 3, both directions. These two sets being
        /// equal is not a coincidence: a linker exists to configure a CLI
        /// ArmadAI will then drive, and the shell drives it in JSON.
        #[test]
        fn the_link_targets_and_the_json_relayable_clis_are_the_same_set() {
            let targets = sorted(
                LinkTarget::value_variants()
                    .iter()
                    .map(|t| t.as_str())
                    .collect(),
            );
            assert_eq!(targets, sorted(json_capable_clis()));
        }

        /// The remaining gap, pinned so widening it is a decision.
        ///
        /// `gpt` and `aider` are runnable but have no linker: neither writes
        /// an ArmadAI-shaped agent config, so there is nothing to generate.
        /// That is legitimate — unlike the codex/copilot/opencode gap, which
        /// was not.
        #[test]
        fn the_only_runnable_tools_without_a_link_target_are_gpt_and_aider() {
            let targets: Vec<&str> = LinkTarget::value_variants()
                .iter()
                .map(|t| t.as_str())
                .collect();
            let api_or_generic = ["cli", "anthropic", "openai", "google", "proxy"];
            let orphans = sorted(
                accepted_provider_names()
                    .into_iter()
                    .filter(|n| !targets.contains(n) && !api_or_generic.contains(n))
                    .collect(),
            );
            assert_eq!(orphans, vec!["aider", "gpt"]);
        }

        /// The `armadai new -i` wizard is a fourth inventory: a provider the
        /// wizard does not offer is one no interactive author can produce an
        /// agent for, even though `run` accepts it.
        #[test]
        fn the_new_wizard_offers_exactly_the_providers_run_accepts() {
            assert_eq!(
                sorted(crate::cli::new::WIZARD_PROVIDER_CHOICES.to_vec()),
                sorted(accepted_provider_names())
            );
        }

        /// The names the "unknown target" messages advertise must be the
        /// names `create_linker` can actually build.
        ///
        /// `link` and `unlink` print the same sentence from the same helper,
        /// so this covers all three sites; before, each carried its own copy
        /// of the five names.
        #[test]
        fn every_advertised_link_target_can_actually_be_built() {
            for name in supported_target_names() {
                let linker = create_linker(name)
                    .unwrap_or_else(|e| panic!("target '{name}' is advertised but: {e}"));
                assert_eq!(linker.name(), name);
            }
            let err = match create_linker("kilocode") {
                Ok(_) => panic!("an unknown target must be refused"),
                Err(e) => e.to_string(),
            };
            for name in supported_target_names() {
                assert!(err.contains(name), "refusal must advertise '{name}': {err}");
            }

            // And the converse. The loops above only prove advertised is a
            // subset of buildable; *under*-advertising is the exact defect this
            // derivation exists to prevent — `proxy` had silently dropped out
            // of the provider message the same way. Measured before this
            // existed: filtering `gemini` out of `supported_target_names()`
            // left 746 tests green. This half iterates the enum, not the
            // helper, so the helper cannot vouch for itself.
            use clap::ValueEnum;
            let sentence = supported_targets_sentence();
            for target in LinkTarget::value_variants() {
                let name = target.as_str();
                assert!(
                    supported_target_names().contains(&name),
                    "link target '{name}' exists but is not advertised"
                );
                assert!(sentence.contains(name), "'{name}' missing from: {sentence}");
            }
        }

        /// And a fifth: the model-resolution preview shown in the TUI's agent
        /// detail view must cover every target `link` can write, or it shows
        /// the user a preview of something other than what will happen.
        #[test]
        fn the_model_resolution_preview_covers_every_link_target() {
            let previewed: Vec<&str> = model_resolution::preview_model_resolution(Some("m"))
                .into_iter()
                .map(|(target, _)| target)
                .collect();
            let targets: Vec<&str> = LinkTarget::value_variants()
                .iter()
                .map(|t| t.as_str())
                .collect();
            assert_eq!(sorted(previewed), sorted(targets));
        }
    }

    #[test]
    fn test_slugify_simple() {
        assert_eq!(slugify("Code Reviewer"), "code-reviewer");
    }

    #[test]
    fn test_slugify_already_kebab() {
        assert_eq!(slugify("code-reviewer"), "code-reviewer");
    }

    #[test]
    fn test_slugify_underscores() {
        assert_eq!(slugify("my_test_agent"), "my-test-agent");
    }

    #[test]
    fn test_slugify_special_chars() {
        assert_eq!(slugify("Agent (v2.0)"), "agent-v20");
    }

    #[test]
    fn test_slugify_multiple_separators() {
        assert_eq!(slugify("a--b__c  d"), "a-b-c-d");
    }

    #[test]
    fn test_create_linker_claude() {
        assert!(create_linker("claude").is_ok());
    }

    #[test]
    fn test_create_linker_copilot() {
        assert!(create_linker("copilot").is_ok());
    }

    #[test]
    fn test_create_linker_gemini() {
        assert!(create_linker("gemini").is_ok());
    }

    #[test]
    fn test_create_linker_codex() {
        assert!(create_linker("codex").is_ok());
    }

    #[test]
    fn test_create_linker_opencode() {
        assert!(create_linker("opencode").is_ok());
    }

    #[test]
    fn test_create_linker_unknown() {
        assert!(create_linker("unknown").is_err());
    }

    /// Assert every field of two `Agent`s that this invariant is responsible
    /// for is equal: every field of `AgentMetadata`, plus `system_prompt`,
    /// `instructions`, `output_format`, `context`. Deliberately **not**
    /// `Agent::source` — it differs by construction (one is a `.yaml`, one is
    /// a `.md`) and no projection reads it, so a difference there is not a
    /// divergence this invariant cares about.
    ///
    /// This exists because the five-target projection comparison
    /// (`assert_projections_equal_across_targets`) is blind to a real class of
    /// divergence: grepping all five linkers for `tags`/`scope`/`stacks` finds
    /// zero hits, and `LinkAgent` does not carry `command`/`args` at all, so a
    /// bug in how those fields are merged would never show up in any
    /// projection. Both checks matter — this one for what the projection
    /// cannot see, the projection one for what actually ships to the CLIs.
    ///
    /// Uses a destructuring pattern rather than a single `assert_eq!` on the
    /// two `AgentMetadata` values because `AgentMetadata` does not derive
    /// `PartialEq` (nor do two of its optional fields, `TriggerConfig` and
    /// `AgentRingConfig`), and adding that derive to a shared domain type just
    /// to shorten this test was judged not worth the production-code touch.
    /// The destructuring gets the same guarantee a derive would have given —
    /// and then some: a field added to `AgentMetadata` later is a **compile
    /// error** here (the pattern stops being exhaustive) rather than a
    /// silently-skipped comparison, which forces this test to be updated
    /// instead of quietly missing the new field.
    fn assert_agents_equivalent(declared: &Agent, written: &Agent) {
        assert_eq!(declared.name, written.name, "agent name diverged");

        let AgentMetadata {
            provider: d_provider,
            model: d_model,
            command: d_command,
            args: d_args,
            temperature: d_temperature,
            max_tokens: d_max_tokens,
            timeout: d_timeout,
            tags: d_tags,
            stacks: d_stacks,
            scope: d_scope,
            model_fallback: d_model_fallback,
            cost_limit: d_cost_limit,
            rate_limit: d_rate_limit,
            context_window: d_context_window,
            mode: d_mode,
            orchestration: d_orchestration,
            triggers: d_triggers,
            ring_config: d_ring_config,
        } = declared.metadata.clone();
        let AgentMetadata {
            provider: w_provider,
            model: w_model,
            command: w_command,
            args: w_args,
            temperature: w_temperature,
            max_tokens: w_max_tokens,
            timeout: w_timeout,
            tags: w_tags,
            stacks: w_stacks,
            scope: w_scope,
            model_fallback: w_model_fallback,
            cost_limit: w_cost_limit,
            rate_limit: w_rate_limit,
            context_window: w_context_window,
            mode: w_mode,
            orchestration: w_orchestration,
            triggers: w_triggers,
            ring_config: w_ring_config,
        } = written.metadata.clone();

        assert_eq!(d_provider, w_provider, "metadata.provider diverged");
        assert_eq!(d_model, w_model, "metadata.model diverged");
        assert_eq!(d_command, w_command, "metadata.command diverged");
        assert_eq!(d_args, w_args, "metadata.args diverged");
        assert_eq!(
            d_temperature, w_temperature,
            "metadata.temperature diverged"
        );
        assert_eq!(d_max_tokens, w_max_tokens, "metadata.max_tokens diverged");
        assert_eq!(d_timeout, w_timeout, "metadata.timeout diverged");
        assert_eq!(d_tags, w_tags, "metadata.tags diverged");
        assert_eq!(d_stacks, w_stacks, "metadata.stacks diverged");
        assert_eq!(d_scope, w_scope, "metadata.scope diverged");
        assert_eq!(
            d_model_fallback, w_model_fallback,
            "metadata.model_fallback diverged"
        );
        // `cost_limit`, `rate_limit`, `context_window`, `mode`,
        // `orchestration`, `triggers` and `ring_config` are fields the
        // declarative format (`AgentDecl`/`AgentDefaults` in
        // `agent_decl.rs`) has no way to express at all — `to_agent` always
        // produces `None`/default for every one of them, regardless of what
        // a `.md` twin's `## Metadata` might otherwise be able to set. Both
        // sides of every fixture in this file are therefore `None` here by
        // construction, and these seven assertions are vacuous: they can
        // never fail while the format stays as it is. That is legitimate,
        // not a gap in this test — but if the declarative format ever grows
        // one of these seven, this comment is where the next reader learns
        // that its fixture also needs a non-default value added, the same
        // way `model_fallback` did after the mutation that found it unused.
        assert_eq!(d_cost_limit, w_cost_limit, "metadata.cost_limit diverged");
        assert_eq!(d_rate_limit, w_rate_limit, "metadata.rate_limit diverged");
        assert_eq!(
            d_context_window, w_context_window,
            "metadata.context_window diverged"
        );
        assert_eq!(d_mode, w_mode, "metadata.mode diverged");
        assert_eq!(
            d_orchestration, w_orchestration,
            "metadata.orchestration diverged"
        );
        // `triggers`/`ring_config` additionally lack `PartialEq` on their
        // types, so `Debug` equality is used instead — exact for what these
        // fixtures can ever produce (always `None`), without adding a
        // derive to production code for it.
        assert_eq!(
            format!("{d_triggers:?}"),
            format!("{w_triggers:?}"),
            "metadata.triggers diverged"
        );
        assert_eq!(
            format!("{d_ring_config:?}"),
            format!("{w_ring_config:?}"),
            "metadata.ring_config diverged"
        );

        assert_eq!(
            declared.system_prompt, written.system_prompt,
            "system_prompt diverged"
        );
        assert_eq!(
            declared.instructions, written.instructions,
            "instructions diverged"
        );
        assert_eq!(
            declared.output_format, written.output_format,
            "output_format diverged"
        );
        assert_eq!(declared.context, written.context, "context diverged");
    }

    /// Assert the two agents project identically on every link target. This
    /// is what actually catches a divergence in what ships to the CLIs —
    /// complementary to, not a substitute for, `assert_agents_equivalent`.
    ///
    /// Run against **every** target, not just claude: a divergence that only
    /// shows in the codex projection is still a divergence.
    fn assert_projections_equal_across_targets(declared: &Agent, written: &Agent) {
        for target in {
            use clap::ValueEnum;
            LinkTarget::value_variants().iter().map(|t| t.as_str())
        } {
            let linker = create_linker(target).unwrap();
            let a = linker.generate(&[LinkAgent::from(declared)], None, &[]);
            let b = linker.generate(&[LinkAgent::from(written)], None, &[]);

            // An empty projection on both sides would satisfy every assertion
            // below without proving anything.
            assert!(
                !a.is_empty(),
                "target {target} produced no output file — the comparison \
                 below would be vacuous"
            );
            assert_eq!(a.len(), b.len(), "target {target}: file count differs");
            for (x, y) in a.iter().zip(b.iter()) {
                assert_eq!(x.path, y.path, "target {target}: paths differ");
                assert_eq!(
                    x.content, y.content,
                    "target {target}: projection diverged — the declaration is \
                     not an alternative spelling but a second source of truth"
                );
            }
        }
    }

    /// Point the global agent library (`ARMADAI_CONFIG_DIR`) at an empty temp
    /// dir for the duration of `f`.
    ///
    /// The three parity tests below load their declared side through
    /// `load_agent_by_name`, the real production entry point — unlike the
    /// now-deleted `load_agent`, that function's file-backed lookup walks
    /// `library_dirs`, which always includes the REAL
    /// `~/.config/armadai/agents/`. On a machine that has ever run `armadai
    /// extract`/`init` from this very repo, that directory holds this
    /// project's own team agents (`core-specialist.md` included) — without
    /// this isolation, a parity test could silently resolve the wrong agent
    /// from a dev box's real global library instead of the fixture below,
    /// and pass or fail for a reason that has nothing to do with its own
    /// fixture. Mirrors `agent_source.rs`'s own
    /// `with_isolated_global_library`; serialised via `env_lock()` since it
    /// mutates a process-global env var.
    fn with_isolated_global_library<T>(f: impl FnOnce() -> T) -> T {
        let _guard = armadai_core::test_support::env_lock();
        let orig = std::env::var("ARMADAI_CONFIG_DIR").ok();
        let empty = tempfile::tempdir().unwrap();
        // SAFETY: serialised via `env_lock()` above.
        unsafe {
            std::env::set_var("ARMADAI_CONFIG_DIR", empty.path());
        }
        let result = f();
        // SAFETY: restoring original env state, still under the guard.
        unsafe {
            match &orig {
                Some(v) => std::env::set_var("ARMADAI_CONFIG_DIR", v),
                None => std::env::remove_var("ARMADAI_CONFIG_DIR"),
            }
        }
        result
    }

    /// A declared agent and its hand-written `.md` twin must produce the same
    /// native projection. If they diverge, the declaration format is a second
    /// source of truth rather than an alternative spelling of the first —
    /// exactly what it exists to avoid.
    ///
    /// Both twins carry a model, a temperature, tags and a scope (beyond the
    /// bare minimum) so the comparison actually exercises the metadata path
    /// `LinkAgent::from(&Agent)` walks, not just the system prompt.
    #[test]
    fn a_declared_agent_and_its_md_twin_project_identically() {
        with_isolated_global_library(|| {
            let dir = tempfile::tempdir().unwrap();

            // The fragment both versions share.
            let fragments = vec![armadai_core::prompt::Prompt {
                name: "base".into(),
                description: None,
                apply_to: vec![],
                body: "You own the core domain.".into(),
                source: std::path::PathBuf::from("base.md"),
            }];

            // Declared version.
            std::fs::create_dir_all(dir.path().join(".armadai")).unwrap();
            std::fs::write(
                dir.path().join(".armadai/agents.yaml"),
                "defaults:\n  provider: claude\n  model: latest:pro\n  temperature: 0.3\n\
                 agents:\n  - name: core-specialist\n    description: Core domain\n    \
                 tags: [rust, domain]\n    scope: [src/core/**]\n    max_tokens: 8192\n    \
                 timeout: 45\n    model_fallback: [latest:fast]\n    prompt: [base]\n",
            )
            .unwrap();
            // Loaded through `load_agent_by_name`, the actual production
            // entry point for one declared agent — not the now-deleted
            // `load_agent`, which had no production caller of its own.
            let config = armadai_core::project::ProjectConfig::default();
            let (declared, warning) = armadai_core::agent_source::load_agent_by_name(
                "core-specialist",
                &config,
                dir.path(),
                &fragments,
            )
            .unwrap();
            assert!(
                warning.is_none(),
                "a clean project must not warn: {warning:?}"
            );

            // Hand-written twin, same values.
            let md = dir.path().join("core-specialist.md");
            std::fs::write(
                &md,
                "# core-specialist\n\n## Metadata\n\
                 - provider: claude\n- model: latest:pro\n- temperature: 0.3\n\
                 - tags: [rust, domain]\n- scope: [src/core/**]\n- max_tokens: 8192\n\
                 - timeout: 45\n- model_fallback: [latest:fast]\n\n## System Prompt\n\nYou own the core domain.\n",
            )
            .unwrap();
            let written = armadai_core::parser::parse_agent_file(&md).unwrap();

            assert_agents_equivalent(&declared, &written);
            assert_projections_equal_across_targets(&declared, &written);
        });
    }

    /// Same invariant as above, for a `provider: cli` agent carrying
    /// `command`/`args` — fields added to the declaration format specifically
    /// so this class of agent could be declared.
    ///
    /// Neither field reaches any linker's output (no target emits `command`
    /// or `args` into generated content), so the real coverage this case adds
    /// is at the `Agent`/`AgentMetadata` level via `assert_agents_equivalent`
    /// — it proves the declared-agent metadata merge is CLI-safe, not that a
    /// declared CLI agent's `command`/`args` show up anywhere a linked config
    /// file's content diverges. The five-target projection comparison is kept
    /// anyway for the fields it *does* cover (model, temperature, prompt).
    #[test]
    fn a_declared_cli_agent_and_its_md_twin_project_identically() {
        with_isolated_global_library(|| {
            let dir = tempfile::tempdir().unwrap();

            let fragments = vec![armadai_core::prompt::Prompt {
                name: "base".into(),
                description: None,
                apply_to: vec![],
                body: "You wrap a CLI tool for scripted tasks.".into(),
                source: std::path::PathBuf::from("base.md"),
            }];

            // Declared version.
            std::fs::create_dir_all(dir.path().join(".armadai")).unwrap();
            std::fs::write(
                dir.path().join(".armadai/agents.yaml"),
                "defaults:\n  provider: cli\n  temperature: 0.5\n\
                 agents:\n  - name: shell-runner\n    description: Wraps a CLI tool\n    \
                 command: echo\n    args: [hello, world]\n    model: local-llm\n    \
                 tags: [ops, shell]\n    stacks: [devops]\n    scope: [scripts/**]\n    prompt: [base]\n",
            )
            .unwrap();
            let config = armadai_core::project::ProjectConfig::default();
            let (declared, warning) = armadai_core::agent_source::load_agent_by_name(
                "shell-runner",
                &config,
                dir.path(),
                &fragments,
            )
            .unwrap();
            assert!(
                warning.is_none(),
                "a clean project must not warn: {warning:?}"
            );

            // Hand-written twin, same values.
            let md = dir.path().join("shell-runner.md");
            std::fs::write(
                &md,
                "# shell-runner\n\n## Metadata\n\
                 - provider: cli\n- command: echo\n- args: [hello, world]\n\
                 - model: local-llm\n- temperature: 0.5\n- tags: [ops, shell]\n\
                 - stacks: [devops]\n- scope: [scripts/**]\n\n## System Prompt\n\nYou wrap a CLI tool for scripted tasks.\n",
            )
            .unwrap();
            let written = armadai_core::parser::parse_agent_file(&md).unwrap();

            assert_agents_equivalent(&declared, &written);
            assert_projections_equal_across_targets(&declared, &written);
        });
    }

    /// Same invariant again, for a declared agent composed from **two**
    /// prompt fragments rather than one. `compose_prompt` joins fragment
    /// bodies with a blank line — a single-fragment fixture (the two tests
    /// above) never exercises that join at all, which would make it dead code
    /// as far as this file's coverage goes.
    #[test]
    fn a_declared_agent_composed_from_two_fragments_projects_like_its_md_twin() {
        with_isolated_global_library(|| {
            let dir = tempfile::tempdir().unwrap();

            let fragments = vec![
                armadai_core::prompt::Prompt {
                    name: "role".into(),
                    description: None,
                    apply_to: vec![],
                    body: "You own the core domain.".into(),
                    source: std::path::PathBuf::from("role.md"),
                },
                armadai_core::prompt::Prompt {
                    name: "constraints".into(),
                    description: None,
                    apply_to: vec![],
                    body: "Never touch generated code by hand.".into(),
                    source: std::path::PathBuf::from("constraints.md"),
                },
            ];

            // Declared version: two prompt steps.
            std::fs::create_dir_all(dir.path().join(".armadai")).unwrap();
            std::fs::write(
                dir.path().join(".armadai/agents.yaml"),
                "defaults:\n  provider: claude\n  model: latest:pro\n  temperature: 0.3\n\
                 agents:\n  - name: core-specialist\n    description: Core domain\n    \
                 tags: [rust, domain]\n    scope: [src/core/**]\n    prompt: [role, constraints]\n",
            )
            .unwrap();
            let config = armadai_core::project::ProjectConfig::default();
            let (declared, warning) = armadai_core::agent_source::load_agent_by_name(
                "core-specialist",
                &config,
                dir.path(),
                &fragments,
            )
            .unwrap();
            assert!(
                warning.is_none(),
                "a clean project must not warn: {warning:?}"
            );

            // Hand-written twin: the same two bodies, joined by a blank line —
            // the equivalent of what `compose_prompt` produces.
            let md = dir.path().join("core-specialist.md");
            std::fs::write(
                &md,
                "# core-specialist\n\n## Metadata\n\
                 - provider: claude\n- model: latest:pro\n- temperature: 0.3\n\
                 - tags: [rust, domain]\n- scope: [src/core/**]\n\n## System Prompt\n\n\
                 You own the core domain.\n\nNever touch generated code by hand.\n",
            )
            .unwrap();
            let written = armadai_core::parser::parse_agent_file(&md).unwrap();

            assert_agents_equivalent(&declared, &written);
            assert_projections_equal_across_targets(&declared, &written);
        });
    }
}
