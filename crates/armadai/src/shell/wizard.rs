//! Project setup wizard for the shell.
//!
//! Guides the user through project initialization and linking before entering the shell.

use anyhow::Result;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use armadai_core::project;
use armadai_core::starter::{find_pack_dir, list_available_packs};

use crate::linker::model_resolution::{self, TargetKind};

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct WizardResult {
    pub provider_command: String,
    pub provider_args: Vec<String>,
    pub model_name: String,
    pub project_name: String,
}

/// Fallback provider for a session where no assistant was ever chosen — the
/// user picked `5) Skip` in `prompt_link` (see its doc: a target the user
/// *did* choose is kept even if linking it failed, so this is `Skip`'s
/// fallback only). The shell still needs *some* command to relay to;
/// `claude` is the one this module already leans on as a default elsewhere
/// (`detect_model_name`, the `resolve_shell_model` examples in
/// `shell/config.rs`) when nothing more specific is known.
const DEFAULT_UNLINKED_PROVIDER: &str = "claude";

/// Check project readiness and run wizard if needed.
/// Returns the provider configuration to use, or an error if setup was cancelled.
pub async fn ensure_project_ready() -> Result<WizardResult> {
    // Step 1: Check project state
    let project_state = detect_project();

    // Step 2: Initialize project if needed
    match project_state {
        ProjectState::NoProject => {
            if !prompt_init()? {
                return Err(anyhow::anyhow!("Project setup cancelled by user"));
            }
        }
        ProjectState::GitRepoNoConfig => {
            if !prompt_init()? {
                return Err(anyhow::anyhow!("Project setup cancelled by user"));
            }
        }
        ProjectState::Configured => {
            // Project already configured, continue
        }
    }

    // Step 3: Check for existing link
    if let Some(linked) = detect_link() {
        // Link exists, use it — default to pro tier
        return build_wizard_result(&linked.name, "latest:pro");
    }

    // Step 4: Prompt for link if needed. Neither declining (`Skip`) nor a
    // link step that fails (e.g. `link`'s own `blocks_a_write` refusal on
    // a shadowing collision) may prevent the shell from opening — the
    // shell is an interactive environment a user needs precisely to fix
    // the kind of problem that would make linking fail, so refusing entry
    // over it would take away their own remedy. Only the *write* is ever
    // refused; `prompt_link` reports why and still returns `Ok(Some(_))`
    // for a chosen-but-unwritten target, `Ok(None)` only for `Skip` (see
    // its own doc) — never an `Err` that would abort here via `?`. A
    // genuine setup failure reading the choice itself still propagates as
    // an `Err` untouched.
    let provider = match prompt_link().await? {
        Some(target) => target,
        None => {
            eprintln!(
                "\nContinuing without a linked assistant — run `armadai link` from inside \
                 the shell once you're ready."
            );
            DEFAULT_UNLINKED_PROVIDER.to_string()
        }
    };

    // Check auth
    if !check_auth(&provider) {
        eprintln!("\nWarning: '{}' command not found in PATH.", provider);
        eprintln!("Make sure it is installed and available before using the shell.");
    }

    // Step 5: Choose performance tier
    let tier = prompt_model_tier()?;
    eprintln!();

    build_wizard_result(&provider, &tier)
}

// ---------------------------------------------------------------------------
// Project detection
// ---------------------------------------------------------------------------

enum ProjectState {
    Configured,
    GitRepoNoConfig,
    NoProject,
}

fn detect_project() -> ProjectState {
    if project::find_project_config().is_some() {
        return ProjectState::Configured;
    }

    if Path::new(".git").exists() {
        return ProjectState::GitRepoNoConfig;
    }

    ProjectState::NoProject
}

// ---------------------------------------------------------------------------
// Link detection
// ---------------------------------------------------------------------------

struct LinkedProvider {
    name: String,
    #[allow(dead_code)]
    path: String,
}

fn detect_link() -> Option<LinkedProvider> {
    let checks = [
        (".gemini", "gemini"),
        (".claude", "claude"),
        (".github/copilot-instructions.md", "copilot"),
        (".codex", "codex"),
    ];

    for (path, provider) in checks {
        if Path::new(path).exists() {
            return Some(LinkedProvider {
                name: provider.to_string(),
                path: path.to_string(),
            });
        }
    }

    None
}

// ---------------------------------------------------------------------------
// Auth check
// ---------------------------------------------------------------------------

fn check_auth(provider: &str) -> bool {
    is_command_available(provider)
}

fn is_command_available(command: &str) -> bool {
    #[cfg(unix)]
    {
        use std::process::Command;
        Command::new("which")
            .arg(command)
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    #[cfg(windows)]
    {
        use std::process::Command;
        Command::new("where")
            .arg(command)
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    #[cfg(not(any(unix, windows)))]
    {
        let _ = command;
        false
    }
}

// ---------------------------------------------------------------------------
// Interactive prompts
// ---------------------------------------------------------------------------

fn prompt_init() -> Result<bool> {
    println!("\nArmadAI Shell — Project Setup\n");
    println!("No ArmadAI config found in this directory.\n");
    println!("Would you like to initialize a project?");
    println!("  1) Quick setup with a starter pack");
    println!("  2) Skip (use system-wide agents only)");

    let choice = read_choice(1, 2)?;

    if choice == 2 {
        return Ok(false);
    }

    // Choice 1: starter pack
    let packs = list_available_packs();
    if packs.is_empty() {
        eprintln!("\nNo starter packs available.");
        return Ok(false);
    }

    println!("\nAvailable starter packs:");
    for (i, pack) in packs.iter().enumerate() {
        println!("  {}) {}", i + 1, pack);
    }

    let pack_choice = read_choice(1, packs.len())?;
    let pack_name = &packs[pack_choice - 1];

    // Run init with pack
    run_init_with_pack(pack_name)?;

    Ok(true)
}

/// Prompt for a link target and attempt to link it.
///
/// `Ok(None)` means "no assistant was chosen" — only `5) Skip`: that
/// choice has nothing to refuse, there being no write behind it, so it
/// always succeeds and the caller (`ensure_project_ready`) falls back to
/// [`DEFAULT_UNLINKED_PROVIDER`].
///
/// A target the user *did* choose is never downgraded to that same
/// fallback, even when linking it fails (most commonly `link`'s own
/// `blocks_a_write` refusal on a shadowing collision, but any other
/// link-step failure too) — only the *write* is refused, not the user's
/// stated choice of assistant, so `Ok(Some(target))` is still returned
/// after printing why the write didn't happen. Silently substituting
/// `claude` for a user who said `gemini` would be a worse surprise than
/// an assistant with no generated agent files yet.
///
/// A genuine setup failure reading the choice itself (`read_choice`'s own
/// `Err`, e.g. unreadable stdin or an out-of-range answer) is a different
/// class of problem and still propagates as `Err`.
async fn prompt_link() -> Result<Option<String>> {
    println!("\nNo link found. Which AI assistant do you use?");
    println!("  1) Gemini CLI");
    println!("  2) Claude Code");
    println!("  3) GitHub Copilot");
    println!("  4) Codex");
    println!("  5) Skip");

    let choice = read_choice(1, 5)?;

    if choice == 5 {
        return Ok(None);
    }

    let target = match choice {
        1 => "gemini",
        2 => "claude",
        3 => "copilot",
        4 => "codex",
        _ => unreachable!(),
    };

    Ok(Some(attempt_link(target).await))
}

/// Link `target`, and report — never propagate — a failure. See
/// `prompt_link`'s doc for why a link-step failure (most commonly `link`'s
/// own `blocks_a_write` refusal on a shadowing collision) must still leave
/// the user able to enter the shell with the assistant they chose: only
/// the *write* is refused, not their presence in the one tool that lets
/// them fix the underlying problem. Split out from `prompt_link` — which
/// also reads the choice interactively from stdin — so this half is
/// testable on its own.
async fn attempt_link(target: &str) -> String {
    if let Err(e) = run_link(target).await {
        eprintln!("\nWarning: linking '{target}' failed: {e}");
    }
    target.to_string()
}

fn read_choice(min: usize, max: usize) -> Result<usize> {
    print!("\nChoice [1]: ");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    let input = input.trim();
    if input.is_empty() {
        return Ok(1);
    }

    let choice: usize = input.parse().map_err(|_| {
        anyhow::anyhow!(
            "Invalid input: expected a number between {} and {}",
            min,
            max
        )
    })?;

    if choice < min || choice > max {
        return Err(anyhow::anyhow!(
            "Choice out of range: expected {} to {}",
            min,
            max
        ));
    }

    Ok(choice)
}

// ---------------------------------------------------------------------------
// Init/Link execution
// ---------------------------------------------------------------------------

fn run_init_with_pack(pack_name: &str) -> Result<()> {
    use armadai_core::config;
    use armadai_core::starter::StarterPack;

    // Init global config
    config::ensure_config_dirs()?;

    // Install pack
    let pack_dir = find_pack_dir(pack_name)
        .ok_or_else(|| anyhow::anyhow!("Starter pack '{}' not found", pack_name))?;

    let pack = StarterPack::load(&pack_dir)?;
    println!(
        "\nInstalling starter pack: {} — {}",
        pack.name, pack.description
    );

    let (agents, prompts, skills) = pack.install(&pack_dir, false)?;
    println!(
        "Pack '{}' installed: {} agent(s), {} prompt(s), {} skill(s)",
        pack.name, agents, prompts, skills
    );

    // Create project config
    let dotarmadai = Path::new(".armadai");
    let dotarmadai_config = dotarmadai.join("config.yaml");

    if dotarmadai_config.exists() {
        println!("\n.armadai/config.yaml already exists, skipping project init");
        return Ok(());
    }

    // Create directory structure
    for subdir in &["agents", "prompts", "skills", "starters"] {
        std::fs::create_dir_all(dotarmadai.join(subdir))?;
    }

    let content = crate::cli::init::generate_project_yaml(&pack, pack_name);
    std::fs::write(&dotarmadai_config, &content)?;
    println!(
        "\nCreated .armadai/config.yaml with pack '{}' agents",
        pack.name
    );

    Ok(())
}

async fn run_link(target: &str) -> Result<()> {
    let (root, config) = project::find_project_config()
        .ok_or_else(|| anyhow::anyhow!("No project config found after initialization"))?;
    run_link_at(&root, &config, target).await
}

/// The testable core of [`run_link`], taking an already-resolved project
/// root/config so tests can drive it against a tempdir without touching
/// the process's current directory (`project::find_project_config` reads
/// `std::env::current_dir()`, which a parallel test suite cannot safely
/// mutate).
///
/// This used to be a hand-rolled, independent re-implementation of
/// `cli::link::execute`'s project-detection gate, agent resolution and
/// write loop (issue #347, and #339's "fifth copy of the project-detection
/// gate" — `armadai shell` could not see declared agents at all). It now
/// goes through the exact same primitives `link` does for each of those:
/// `agent_source::project_declares_agents`/`load_all_agents` for detection
/// and resolution, `linker::manifest::write_files` for the write itself —
/// so the manifest write and the exists-guard come from the same place
/// `link` gets them, rather than a third copy that could drift from both.
///
/// Deliberately narrower than `link`'s own CLI *flags*: no `--coordinator`
/// override, no skills/prompts, no `--agents` filter, no
/// `--model`/interactive model prompt, no `--force` — the wizard flow has
/// no way to express any of those. What it does honour is the project
/// **config**: `link.coordinator` reaches `generate` here exactly as it
/// does in `link` (issue #375). It used to be hardcoded to `None`, so the
/// wizard wrote a per-agent file for every agent and never the target's
/// root instructions file — and a later manifest-less `unlink`, looking
/// for the coordinator where `link` would have put it, left that
/// per-agent file behind.
async fn run_link_at(
    root: &Path,
    config: &armadai_core::project::ProjectConfig,
    target: &str,
) -> Result<()> {
    println!("\nLinking to '{}'...", target);

    // Every agent in `.armadai/agents.yaml` is included automatically —
    // same widened gate `link`, `list`, `run` and `unlink` all share via
    // `agent_source::project_declares_agents` (issue #337/#339). The old
    // `config.agents.is_empty()` check here was the one copy that never
    // learned about declared agents at all: a declarations-only project
    // would fail here with a false "No agents declared", and even a
    // project mixing both formats would only ever resolve the file-backed
    // half below.
    if !armadai_core::agent_source::project_declares_agents(root, config) {
        return Err(anyhow::anyhow!("No agents declared in project config"));
    }

    let fragments = armadai_core::agent_source::project_fragments(root);
    let (agents, warnings) = armadai_core::agent_source::load_all_agents(config, root, &fragments);
    for w in &warnings {
        eprintln!("  warn: {}", w.message());
    }

    let mut link_agents: Vec<crate::linker::LinkAgent> =
        agents.iter().map(crate::linker::LinkAgent::from).collect();

    if link_agents.is_empty() {
        return Err(anyhow::anyhow!("No agents could be resolved"));
    }

    // Resolve deprecated models — same as `link`.
    for agent in &mut link_agents {
        armadai_core::model_aliases::resolve_model_deprecations(
            &mut agent.model,
            &mut agent.model_fallback,
        );
    }

    // `link` pairs `load_all_agents` with this refusal (issue #342/#349's
    // closed defect: never write a smaller fleet than declared) — the one
    // piece of that pairing this function's earlier fix left out. Without
    // it, a loss this chantier's declarative format is responsible for (a
    // dropped declaration, or a shadowing collision) would still only warn,
    // then have `write_files` below record the amputated fleet actually
    // written as the *authoritative* manifest for this target — worse than
    // the pre-manifest world, where at least nothing durable claimed to be
    // complete. The wizard has no `--agents` filter to scope this to, so —
    // like a plain `armadai link` with none either — any such loss refuses
    // the whole write.
    if armadai_core::agent_source::blocks_a_write(&warnings, None) {
        return Err(anyhow::anyhow!(
            "one or more agents could not be loaded (see warning(s) above) — refusing to \
             link a smaller fleet than declared. Fix the issue(s), or rerun once resolved."
        ));
    }

    // The configured coordinator, resolved against the roster through the
    // same `name_matches_reference` `link` and `unlink` both use (issue
    // #341/#370): `coordinator: dev-lead` designates the agent titled
    // `Dev Lead`. Removing it from `link_agents` is what makes it the
    // target's root instructions file instead of one more per-agent file
    // — `link`'s own step 3b, verbatim. The wizard has no
    // `--coordinator` flag to override the config with.
    //
    // The model-resolution step below runs on the coordinator too, the
    // same way `link` and `unlink` both do. Measured: no linker
    // serialises a coordinator's `model:` into its root instructions
    // document today (checked in all five of claude/codex/copilot/
    // gemini/opencode), so that arm makes no observable difference to
    // the bytes written — it is kept aligned so a linker that starts
    // emitting one is correct by construction rather than by accident,
    // and so this function stays a mirror of `link` rather than a fourth
    // variant of it.
    let (mut coordinator, coordinator_warning) = crate::linker::take_coordinator(
        &mut link_agents,
        None,
        config.link.as_ref().and_then(|l| l.coordinator.clone()),
    );
    if let Some(message) = coordinator_warning {
        eprintln!(
            "  warn: {}",
            crate::cli::style::indent_continuation(&message, "        ")
        );
    }

    // The same model resolution `link`'s own step 4b performs (issue I1 on
    // #347's review): without this, a `latest:*` placeholder reaches
    // `generate()` completely unresolved, so the wizard writes a literal
    // `model: latest:pro` where `link` writes the actual resolved model
    // (e.g. `claude-sonnet-4-5-20250929`) for the same agent and target.
    // That mismatch is why the #342 fallback can never reclaim a
    // wizard-written file on its own: regenerating "the `link` way" never
    // produces the same bytes the wizard wrote, so a project that lost its
    // manifest has no way back for these files at all. The wizard has no
    // `--model` flag and no interactive model prompt of its own (its
    // earlier performance-tier prompt picks the *shell's* conversation
    // model, a separate concern) — for an `Orchestrator` target this
    // mirrors `link`'s own non-interactive, no-`--model` default exactly.
    let target_kind = model_resolution::classify_target(target);
    match target_kind {
        TargetKind::LlmEditor { provider } => {
            #[cfg(feature = "providers-api")]
            {
                model_resolution::remap_models_for_llm_editor(&mut link_agents, provider).await;
                if let Some(ref mut coord) = coordinator {
                    model_resolution::remap_models_for_llm_editor(
                        std::slice::from_mut(coord),
                        provider,
                    )
                    .await;
                }
            }
            #[cfg(not(feature = "providers-api"))]
            {
                model_resolution::remap_models_for_llm_editor(&mut link_agents, provider);
                if let Some(ref mut coord) = coordinator {
                    model_resolution::remap_models_for_llm_editor(
                        std::slice::from_mut(coord),
                        provider,
                    );
                }
            }
        }
        TargetKind::Orchestrator => {
            model_resolution::resolve_latest_placeholders(&mut link_agents);
            if let Some(ref mut coord) = coordinator {
                model_resolution::resolve_latest_placeholders(std::slice::from_mut(coord));
            }
        }
    }

    let linker = crate::linker::create_linker(target)?;

    let sources = &config.sources;
    let files = linker.generate(&link_agents, coordinator.as_ref(), sources);

    if files.is_empty() {
        return Err(anyhow::anyhow!("No files to generate"));
    }

    // The same output-directory resolution `link` uses (there is no
    // wizard equivalent of `link`'s `--output` flag, but a project's own
    // `link.overrides` still applies) — needed so the manifest this
    // writes declares the same `root` a later plain `armadai unlink` would
    // independently compute. A mismatch there makes `unlink` refuse the
    // manifest wholesale (`root_confirmed`) and fall back to the #342
    // guard instead of consuming what was just written here.
    let output_dir = config
        .link
        .as_ref()
        .and_then(|l| l.overrides.get(target))
        .and_then(|o| o.output.as_ref())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(linker.default_output_dir()));
    let target_root = root.join(&output_dir);

    let default_dir = PathBuf::from(linker.default_output_dir());
    let agent_count = link_agents.len();
    let output_files: Vec<(PathBuf, String, crate::linker::manifest::ProducedBy)> = files
        .into_iter()
        .enumerate()
        .map(|(idx, f)| {
            let relative = f
                .path
                .strip_prefix(&default_dir)
                .unwrap_or(&f.path)
                .to_path_buf();
            let final_path = root.join(&output_dir).join(&relative);
            let produced_by = if idx < agent_count {
                crate::linker::manifest::ProducedBy::agent(link_agents[idx].name.clone())
            } else {
                // Past the per-agent prefix: the target's root
                // instructions document, attributed to the configured
                // coordinator — or, for the targets that emit a
                // team-roster document with none configured, to the
                // target itself. `link`'s step 8, verbatim.
                crate::linker::manifest::ProducedBy::coordinator(
                    coordinator
                        .as_ref()
                        .map(|c| c.name.clone())
                        .unwrap_or_else(|| target.to_string()),
                )
            };
            (final_path, f.content, produced_by)
        })
        .collect();

    // The actual write, through the exact same path `link` uses: an
    // exists-guard (the wizard has no `--force` of its own, so this is
    // always `force: false` — a hand-written file is never overwritten)
    // and a manifest entry for every decision, written at the point of
    // effect.
    let outcomes = crate::linker::manifest::write_files(
        root,
        target,
        &output_dir,
        &target_root,
        output_files,
        false,
    )?;

    let mut written = 0;
    let mut skipped = 0;
    let mut unchanged = 0;
    for outcome in &outcomes {
        match outcome {
            crate::linker::manifest::FileOutcome::Wrote(path) => {
                println!("  wrote {}", path.display());
                written += 1;
            }
            crate::linker::manifest::FileOutcome::UpToDate(path) => {
                println!("  up-to-date {}", path.display());
                unchanged += 1;
            }
            crate::linker::manifest::FileOutcome::SkippedExisting(path) => {
                eprintln!(
                    "  skip: {} already exists (use `armadai link --target {} --force` to overwrite)",
                    path.display(),
                    target
                );
                skipped += 1;
            }
        }
    }

    println!(
        "\nLinked {} agent(s) to '{}': {} written, {} skipped, {} unchanged.",
        link_agents.len(),
        target,
        written,
        skipped,
        unchanged
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Result builder
// ---------------------------------------------------------------------------

/// Prompt user to choose a performance tier.
fn prompt_model_tier() -> Result<String> {
    println!("\nChoose performance level:");
    println!("  1) Fast — cheapest, fastest (flash/haiku/mini)");
    println!("  2) Pro — balanced quality & cost (recommended)");
    println!("  3) Max — best quality, most expensive (opus/pro/o3)");
    print!("\nChoice [2]: ");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let choice = input.trim();

    match choice {
        "1" => Ok("latest:fast".to_string()),
        "3" => Ok("latest:max".to_string()),
        _ => Ok("latest:pro".to_string()), // default
    }
}

fn build_wizard_result(provider: &str, tier: &str) -> Result<WizardResult> {
    let (command, args) = match provider {
        "gemini" => ("gemini", vec!["-p".to_string()]),
        "claude" => ("claude", vec![]),
        "aider" => ("aider", vec!["--yes".to_string()]),
        "codex" => ("codex", vec![]),
        _ => (provider, vec![]),
    };

    // Resolve model from tier
    let model_name = crate::shell::config::resolve_shell_model(command, tier);
    let project_name = detect_project_name();

    Ok(WizardResult {
        provider_command: command.to_string(),
        provider_args: args,
        model_name,
        project_name,
    })
}

fn detect_model_name(command: &str) -> String {
    match command {
        "gemini" => {
            // Try to read model from .gemini/settings.json
            if let Ok(content) = std::fs::read_to_string(".gemini/settings.json")
                && let Ok(json) = serde_json::from_str::<serde_json::Value>(&content)
                && let Some(model) = json.get("model").and_then(|m| m.as_str())
            {
                return model.to_string();
            }
            "gemini-2.5-flash".to_string()
        }
        "claude" => "claude-sonnet-4-5".to_string(),
        "aider" => "gpt-4o".to_string(),
        "codex" => "codex".to_string(),
        _ => "unknown".to_string(),
    }
}

fn detect_project_name() -> String {
    std::env::current_dir()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
        .unwrap_or_else(|| "unknown".to_string())
}

#[cfg(test)]
mod run_link_tests {
    use super::*;
    use armadai_core::test_support::{IsolatedConfigDir, IsolatedProjectDir};

    /// A project with two file-backed agents, `keep` and `drop`, no
    /// declarative agents, targeting nothing in particular — `run_link_at`
    /// is always called with an explicit `target`.
    fn write_two_agent_project(root: &Path) {
        std::fs::create_dir_all(root.join("agents")).unwrap();
        std::fs::write(
            root.join("armadai.yaml"),
            "agents:\n  - name: keep\n  - name: drop\n",
        )
        .unwrap();
        std::fs::write(
            root.join("agents/keep.md"),
            "# keep\n\n## Metadata\n- provider: claude\n\n## System Prompt\n\nStay.\n",
        )
        .unwrap();
        std::fs::write(
            root.join("agents/drop.md"),
            "# drop\n\n## Metadata\n- provider: claude\n\n## System Prompt\n\nLeave.\n",
        )
        .unwrap();
    }

    /// A project whose roster's **second** agent is the configured
    /// `link.coordinator`, spelled as a slug (`dev-lead`) against an H1
    /// title that is not the slug (`Dev Lead`). Both details are load
    /// bearing:
    ///
    /// - **Not position 0.** A fixture with the coordinator first stays
    ///   green under an always-true match predicate, because the agent
    ///   picked out is the right one by accident — the exact fixture trap
    ///   measured on #370.
    /// - **Title ≠ reference.** `link.coordinator` is matched against the
    ///   agent's H1 title *or that title's slug*, never against the
    ///   `agents:` key (a separate namespace — see `docs/wiki/link.md`).
    ///   A `Dev Lead`/`dev-lead` pair fails under a plain `==` on names
    ///   and passes only through `name_matches_reference`.
    fn write_coordinator_project(root: &Path) {
        std::fs::create_dir_all(root.join("agents")).unwrap();
        std::fs::write(
            root.join("armadai.yaml"),
            "agents:\n  - name: worker\n  - name: dev-lead\nlink:\n  target: claude\n  \
             coordinator: dev-lead\n",
        )
        .unwrap();
        std::fs::write(
            root.join("agents/worker.md"),
            "# Worker\n\n## Metadata\n- provider: claude\n\n## System Prompt\n\nDo the work.\n",
        )
        .unwrap();
        std::fs::write(
            root.join("agents/dev-lead.md"),
            "# Dev Lead\n\n## Metadata\n- provider: claude\n\n## System Prompt\n\nLead.\n",
        )
        .unwrap();
    }

    /// Issue #375: the wizard must honour `link.coordinator` exactly as
    /// `link` does — the configured coordinator becomes the target's root
    /// instructions file (`.claude/CLAUDE.md`) and gets **no** per-agent
    /// file, because `link` removes it from the roster before generating.
    ///
    /// Before the fix, `run_link_at` passed a hardcoded `None` as
    /// `generate`'s coordinator argument, so the wizard wrote a per-agent
    /// file for every agent and never the root instructions file, whatever
    /// the config said.
    ///
    /// Mutation this catches: reverting the `generate` call to
    /// `linker.generate(&link_agents, None, sources)` (the pre-fix line)
    /// leaves no `.claude/CLAUDE.md` and writes `.claude/agents/dev-lead.md`
    /// instead — both assertions below fail.
    #[tokio::test]
    async fn wizard_link_honours_the_configured_link_coordinator() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        write_coordinator_project(&root);

        let (found_root, config) = project::find_project_config_from(&root).unwrap();
        run_link_at(&found_root, &config, "claude").await.unwrap();

        assert!(
            root.join(".claude/CLAUDE.md").is_file(),
            "the wizard must write the coordinator's root instructions file, the way \
             `link` does"
        );
        assert!(
            !root.join(".claude/agents/dev-lead.md").exists(),
            "the coordinator is removed from the roster before generating, so it must \
             get no per-agent file"
        );
        assert!(
            root.join(".claude/agents/worker.md").is_file(),
            "every non-coordinator agent still gets its per-agent file"
        );
    }

    /// Issue #375's measured symptom, end to end: what the wizard writes
    /// must be exactly what `unlink` reclaims — including on the
    /// **fallback** path, the one that recomputes what `link` would write
    /// rather than reading the manifest. `.armadai/` is removed between
    /// the two halves precisely to force that path; without that removal
    /// the manifest answers, and this test would prove nothing about the
    /// name-based matching it exists to pin (the trap already recorded in
    /// `tests/unlink_content_guard.rs`).
    ///
    /// Measured before the fix, on the real binary:
    ///
    /// ```text
    /// 1 deleted, 0 kept, 1 already absent   survivor: .claude/agents/dev-lead.md
    /// ```
    ///
    /// `unlink` looked for the coordinator where `link` would have put it
    /// (`.claude/CLAUDE.md`), did not find it, and left behind the
    /// per-agent file the wizard had written instead.
    ///
    /// Mutation this catches: the same `None` revert as the test above
    /// leaves `.claude/agents/dev-lead.md` on disk after the unlink.
    #[tokio::test]
    async fn wizard_link_then_unlink_without_a_manifest_leaves_nothing_behind() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        write_coordinator_project(&root);

        let _isolated = IsolatedProjectDir::enter(&root);

        let (found_root, config) = project::find_project_config_from(&root).unwrap();
        run_link_at(&found_root, &config, "claude").await.unwrap();

        // Force `unlink`'s fallback path: with the manifest present it
        // would reclaim by record, never exercising the name matching
        // this test is about.
        std::fs::remove_dir_all(root.join(".armadai")).unwrap();

        crate::cli::unlink::execute(
            Some(crate::linker::LinkTarget::Claude),
            None,
            false,
            false,
            None,
            None,
        )
        .await
        .unwrap();

        let survivors: Vec<PathBuf> = walk_files(&root.join(".claude"));
        assert!(
            survivors.is_empty(),
            "a manifest-less unlink must reclaim everything a wizard-driven link wrote; \
             survivors: {survivors:?}"
        );
    }

    /// Every file under `dir`, recursively — `.claude/` itself is left in
    /// place by `unlink` by design (issue #338 case 1), so the assertion
    /// above is about files, not the directory.
    fn walk_files(dir: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        let Ok(entries) = std::fs::read_dir(dir) else {
            return out;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                out.extend(walk_files(&path));
            } else {
                out.push(path);
            }
        }
        out.sort();
        out
    }

    /// A wizard-driven link must write the same `.armadai/link-manifest.yaml`
    /// `link` itself writes, so `unlink` can act on it afterwards instead
    /// of falling back to the #342 content-match guard — the exact gap
    /// issue #347 measured (the wizard's own write loop skipped the
    /// manifest entirely, so `unlink` never knew what it had produced).
    ///
    /// Proven the same way `link_manifest.rs`'s own `case2` does: drop an
    /// agent from the config *after* linking, then unlink. Only a
    /// manifest can reclaim that agent's file at all — the #342 fallback
    /// regenerates against the *current* config, which no longer names
    /// `drop`, so its file could never even be a candidate there. This
    /// guards against the exact trap #349 warned about: a version of this
    /// test that passed identically with the manifest deleted would only
    /// be proving the fallback, not the wizard's manifest write.
    ///
    /// Mutation this catches: reverting `run_link_at`'s write to a
    /// hand-rolled `fs::write` loop with no `linker::manifest::write_files`
    /// call (the pre-fix wizard) leaves no manifest behind at all, so
    /// `unlink` falls back and `drop.md` survives — this assertion fails.
    #[tokio::test]
    async fn wizard_link_writes_a_manifest_that_unlink_consumes_for_an_orphaned_agent() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        write_two_agent_project(&root);

        let _isolated = IsolatedProjectDir::enter(&root);

        let (found_root, config) = project::find_project_config_from(&root).unwrap();
        run_link_at(&found_root, &config, "claude").await.unwrap();

        let manifest_path = root.join(".armadai/link-manifest.yaml");
        assert!(
            manifest_path.is_file(),
            "a wizard-driven link must write a manifest, the same way `link` does"
        );

        let drop_file = root.join(".claude/agents/drop.md");
        assert!(drop_file.is_file(), "link must have generated drop's file");

        // Drop `drop` from the config entirely — the orphan case only a
        // manifest can still reclaim.
        std::fs::write(root.join("armadai.yaml"), "agents:\n  - name: keep\n").unwrap();

        crate::cli::unlink::execute(
            Some(crate::linker::LinkTarget::Claude),
            None,
            false,
            false,
            None,
            None,
        )
        .await
        .unwrap();

        assert!(
            !drop_file.exists(),
            "an orphaned agent's file must be reclaimed via the manifest the wizard-driven \
             link wrote"
        );
    }

    /// `run_link_at` refuses to overwrite a hand-written file, exactly as
    /// `link` does (`link.rs:295` before the shared write path existed) —
    /// the second gap issue #347 measured: the wizard's own write loop had
    /// no exists-guard at all and would have clobbered it.
    ///
    /// Mutation this catches: if the exists-guard were removed (or
    /// `run_link_at` passed `force: true` to `write_files`), the
    /// hand-written content would be overwritten and this test's content
    /// assertion would fail.
    #[tokio::test]
    async fn wizard_link_refuses_to_overwrite_a_hand_written_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        std::fs::create_dir_all(root.join("agents")).unwrap();
        std::fs::write(root.join("armadai.yaml"), "agents:\n  - name: solo\n").unwrap();
        std::fs::write(
            root.join("agents/solo.md"),
            "# solo\n\n## Metadata\n- provider: claude\n\n## System Prompt\n\nWork.\n",
        )
        .unwrap();

        let claude_agents = root.join(".claude/agents");
        std::fs::create_dir_all(&claude_agents).unwrap();
        let hand_written = "# written by a human before the wizard ever ran\n";
        std::fs::write(claude_agents.join("solo.md"), hand_written).unwrap();

        let (found_root, config) = project::find_project_config_from(&root).unwrap();
        run_link_at(&found_root, &config, "claude").await.unwrap();

        assert_eq!(
            std::fs::read_to_string(claude_agents.join("solo.md")).unwrap(),
            hand_written,
            "a hand-written file must never be overwritten by the wizard's link, \
             matching link's own --force-less behaviour"
        );
    }

    /// #339's other half: `run_link_at` must see agents declared purely
    /// via `.armadai/agents.yaml`, not just `armadai.yaml`'s `agents:`
    /// list — the "fifth copy of the project-detection gate" issue #347
    /// named alongside the manifest/exists-guard gap. Before this fix,
    /// `config.agents.is_empty()` was the whole check here, so a
    /// declarations-only project (a real, common shape: this chantier's
    /// whole point is not needing to relist every agent in `agents:`)
    /// would fail with a false "No agents declared in project config".
    ///
    /// Mutation this catches: reverting the gate to
    /// `config.agents.is_empty()` makes this test's `run_link_at` call
    /// return an error instead of `Ok`.
    #[tokio::test]
    async fn wizard_link_sees_agents_declared_only_in_agents_yaml() {
        let _isolated = IsolatedConfigDir::enter();

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        std::fs::create_dir_all(root.join(".armadai")).unwrap();
        std::fs::write(root.join(".armadai/config.yaml"), "agents: []\n").unwrap();
        std::fs::write(
            root.join(".armadai/agents.yaml"),
            "defaults:\n  provider: claude\nagents:\n  - name: declared-only\n",
        )
        .unwrap();

        let (found_root, config) = project::find_project_config_from(&root).unwrap();
        run_link_at(&found_root, &config, "claude").await.unwrap();

        assert!(
            root.join(".claude/agents/declared-only.md").is_file(),
            "an agent declared only in .armadai/agents.yaml must still be linked by the wizard"
        );
    }

    /// B1 (independent review of #347): `link` pairs `load_all_agents`
    /// with `blocks_a_write` — never write a smaller fleet than declared
    /// when the loss is this chantier's format's own responsibility (a
    /// dropped declaration, or a shadowing collision). `run_link_at`
    /// adopted the resolution half of that pairing but not the refusal
    /// half, so a declared agent shadowed by a same-named global `.md`
    /// would only warn, then have the *rest* of the fleet written and
    /// recorded in the manifest as if it were complete — worse than the
    /// pre-manifest world, since the manifest is now the authoritative
    /// record `unlink` trusts.
    ///
    /// Mutation this catches: removing the `blocks_a_write` call (or
    /// calling it with an argument that can never block, e.g. a filter
    /// that excludes the shadowed name) makes this test's `run_link_at`
    /// call return `Ok` and write `solo.md` — both assertions below fail.
    #[tokio::test]
    async fn wizard_link_refuses_a_write_when_a_declared_agent_is_shadowed() {
        let isolated = IsolatedConfigDir::enter();
        std::fs::create_dir_all(isolated.global_agents_dir()).unwrap();
        std::fs::write(
            isolated.global_agents_dir().join("collide.md"),
            "# collide\n\n## Metadata\n- provider: claude\n\n## System Prompt\n\nGlobal.\n",
        )
        .unwrap();

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        std::fs::create_dir_all(root.join(".armadai")).unwrap();
        std::fs::write(root.join(".armadai/config.yaml"), "agents: []\n").unwrap();
        std::fs::write(
            root.join(".armadai/agents.yaml"),
            "defaults:\n  provider: claude\nagents:\n  - name: solo\n  - name: collide\n",
        )
        .unwrap();

        let (found_root, config) = project::find_project_config_from(&root).unwrap();
        let result = run_link_at(&found_root, &config, "claude").await;

        assert!(
            result.is_err(),
            "a shadowing collision on one declared agent must refuse the whole write, \
             not silently link the rest as if the fleet were complete"
        );
        assert!(
            !root.join(".claude").exists(),
            "nothing must be written at all when the write is refused — not even the \
             unaffected agent's file"
        );
    }

    /// I1 (independent review of #347): the wizard must write the same
    /// bytes `link` would for the same agent and target, which means
    /// resolving a `latest:*` placeholder into a concrete model (`link`'s
    /// own step 4b) rather than leaving the literal placeholder string in
    /// the generated file. Without this, the #342 fallback can never
    /// reclaim a wizard-written file at all — regenerating "the `link`
    /// way" resolves the placeholder, so it never byte-matches what the
    /// wizard actually wrote, and an unrecoverable orphan is the result
    /// the moment the manifest is gone.
    ///
    /// An isolated, empty `ARMADAI_CONFIG_DIR` guarantees no
    /// `models-cache.json` is present, so `resolve_model_for_tier`
    /// deterministically falls back to `fallback_model_for_tier`
    /// ("claude-sonnet-4-5-20250929" for anthropic/Pro) — pinning this
    /// test to that fallback rather than to whatever a live models.dev
    /// cache happens to hold on the machine running it.
    ///
    /// Mutation this catches: removing the model-resolution step (or its
    /// `.await` under `providers-api`, which would fail to compile rather
    /// than silently no-op, but the sync-mode call is removable the same
    /// way) leaves `model: latest:pro` verbatim in the written file, and
    /// both assertions below fail.
    #[tokio::test]
    async fn wizard_link_resolves_latest_placeholders_the_way_link_does() {
        let _isolated = IsolatedConfigDir::enter();

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        std::fs::create_dir_all(root.join("agents")).unwrap();
        std::fs::write(root.join("armadai.yaml"), "agents:\n  - name: solo\n").unwrap();
        std::fs::write(
            root.join("agents/solo.md"),
            "# solo\n\n## Metadata\n- provider: claude\n- model: latest:pro\n\n\
             ## System Prompt\n\nWork.\n",
        )
        .unwrap();

        let (found_root, config) = project::find_project_config_from(&root).unwrap();
        run_link_at(&found_root, &config, "claude").await.unwrap();

        let written = std::fs::read_to_string(root.join(".claude/agents/solo.md")).unwrap();
        assert!(
            !written.contains("model: latest:pro"),
            "the wizard must resolve latest:* placeholders, not write them verbatim: \
             {written}"
        );
        assert!(
            written.contains("model: claude-sonnet-4-5-20250929"),
            "the wizard must resolve to the same fallback model `link` would, when no \
             models-cache is present: {written}"
        );
    }

    /// Review point 2 (fix round on `fix/wizard-link-write-path`): every
    /// pre-existing wizard test targets `"claude"`, an `LlmEditor` — so
    /// `run_link_at`'s `TargetKind::Orchestrator` match arm, the one I1
    /// was actually about (it is precisely where model resolution differs
    /// from `LlmEditor`'s force-to-target-provider rule), had zero
    /// coverage: gutting that whole arm left every wizard test green.
    ///
    /// Uses `opencode`, not `copilot` — the other `Orchestrator` target —
    /// because `CopilotLinker::generate` never serialises `model:` into
    /// its output at all (checked in `linker/copilot.rs`), so a byte-level
    /// assertion has nothing to check there; `OpencodeLinker` does write
    /// `model: <value>` frontmatter. A different tier (`latest:fast`) than
    /// the test above's `latest:pro` keeps the two tests' expected values
    /// visibly apart.
    ///
    /// Caveat this test does NOT cover, stated rather than silently
    /// assumed: this pins the non-interactive default only. On a real
    /// TTY, `link` itself prompts for a model and stamps that single
    /// answer onto every agent for `Orchestrator` targets (`cli::link`'s
    /// own interactive branch, gated on `std::io::stdin().is_terminal()`)
    /// — a wizard/`link` byte-equality claim for `Orchestrator` targets
    /// holds only against that same non-interactive path. Neither
    /// `run_link_at` nor this test attempts the interactive path at all;
    /// the wizard has no `--model` prompt of its own for the target's
    /// *linked* agents (only for the shell's own conversation model —
    /// see `run_link_at`'s doc).
    ///
    /// Mutation this catches: emptying the `TargetKind::Orchestrator`
    /// match arm (or swapping it for the `LlmEditor` arm's
    /// target-provider-forced resolution) either leaves `model:
    /// latest:fast` unresolved or resolves it to a different tier's
    /// fallback — either way this test's assertions fail, where all
    /// `"claude"`-targeted tests stay green regardless.
    #[tokio::test]
    async fn wizard_link_resolves_latest_placeholders_for_an_orchestrator_target() {
        let _isolated = IsolatedConfigDir::enter();

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        std::fs::create_dir_all(root.join("agents")).unwrap();
        std::fs::write(root.join("armadai.yaml"), "agents:\n  - name: solo\n").unwrap();
        std::fs::write(
            root.join("agents/solo.md"),
            "# solo\n\n## Metadata\n- provider: claude\n- model: latest:fast\n\n\
             ## System Prompt\n\nWork.\n",
        )
        .unwrap();

        let (found_root, config) = project::find_project_config_from(&root).unwrap();
        run_link_at(&found_root, &config, "opencode").await.unwrap();

        let written = std::fs::read_to_string(root.join(".opencode/agents/solo.md")).unwrap();
        assert!(
            !written.contains("model: latest:fast"),
            "the wizard must resolve latest:* placeholders for Orchestrator targets \
             too, not write them verbatim: {written}"
        );
        assert!(
            written.contains("model: claude-haiku-4-5-20251001"),
            "the wizard must resolve to the same fallback model link's non-interactive \
             path would, when no models-cache is present: {written}"
        );
    }

    /// Review point 1 (blocking, on `fix/wizard-link-write-path`): a link
    /// step that fails to *write* (here, `blocks_a_write` on a shadowing
    /// collision — the same setup as
    /// `wizard_link_refuses_a_write_when_a_declared_agent_is_shadowed`)
    /// must never take the shell itself away. `attempt_link` is
    /// `prompt_link`'s non-interactive half — see its doc — and must
    /// report the failure without propagating it, still returning the
    /// chosen target so `ensure_project_ready` can open the shell with
    /// it. `attempt_link`'s return type (`String`, not `Result<String>`)
    /// already makes the "propagate the error" mutant impossible to
    /// reintroduce without a compile error; what this test pins is the
    /// value that comes back on that path.
    ///
    /// Mutation this catches: if a failed link instead returned
    /// [`DEFAULT_UNLINKED_PROVIDER`] (silently substituting `claude` for
    /// whatever the user actually chose) rather than echoing the chosen
    /// target back, this test's `assert_eq!` fails.
    #[tokio::test]
    async fn attempt_link_keeps_the_chosen_target_when_the_write_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        std::fs::create_dir_all(root.join(".armadai")).unwrap();
        std::fs::write(root.join(".armadai/config.yaml"), "agents: []\n").unwrap();
        std::fs::write(
            root.join(".armadai/agents.yaml"),
            "defaults:\n  provider: claude\nagents:\n  - name: solo\n  - name: collide\n",
        )
        .unwrap();

        let isolated = IsolatedProjectDir::enter(&root);
        std::fs::create_dir_all(isolated.global_agents_dir()).unwrap();
        std::fs::write(
            isolated.global_agents_dir().join("collide.md"),
            "# collide\n\n## Metadata\n- provider: claude\n\n## System Prompt\n\nGlobal.\n",
        )
        .unwrap();

        // Deliberately NOT "claude": `DEFAULT_UNLINKED_PROVIDER` is also
        // "claude", so a mutant that silently substitutes it would pass
        // an assert_eq! against "claude" undetected — the exact
        // mutually-confusable-value trap this project has hit before.
        // "gemini" makes the two outcomes distinguishable.
        let result = attempt_link("gemini").await;

        assert_eq!(
            result, "gemini",
            "a link-step failure must still return the target the user chose, not an \
             error and not a silently substituted default"
        );
        assert!(
            !root.join(".gemini").exists(),
            "the write itself must still be refused — nothing gets written"
        );
    }
}
