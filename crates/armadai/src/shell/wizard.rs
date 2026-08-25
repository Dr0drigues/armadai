//! Project setup wizard for the shell.
//!
//! Guides the user through project initialization and linking before entering the shell.

use anyhow::Result;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use armadai_core::project;
use armadai_core::starter::{find_pack_dir, list_available_packs};

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

/// Check project readiness and run wizard if needed.
/// Returns the provider configuration to use, or an error if setup was cancelled.
pub fn ensure_project_ready() -> Result<WizardResult> {
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

    // Step 4: Prompt for link if needed
    let provider = prompt_link()?;

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

fn prompt_link() -> Result<String> {
    println!("\nNo link found. Which AI assistant do you use?");
    println!("  1) Gemini CLI");
    println!("  2) Claude Code");
    println!("  3) GitHub Copilot");
    println!("  4) Codex");
    println!("  5) Skip");

    let choice = read_choice(1, 5)?;

    if choice == 5 {
        return Err(anyhow::anyhow!("Link setup skipped by user"));
    }

    let target = match choice {
        1 => "gemini",
        2 => "claude",
        3 => "copilot",
        4 => "codex",
        _ => unreachable!(),
    };

    // Run link
    run_link(target)?;

    Ok(target.to_string())
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

fn run_link(target: &str) -> Result<()> {
    let (root, config) = project::find_project_config()
        .ok_or_else(|| anyhow::anyhow!("No project config found after initialization"))?;
    run_link_at(&root, &config, target)
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
/// Deliberately narrower than `link`'s own CLI surface: no coordinator, no
/// skills/prompts, no `--agents` filter, no `--model`/interactive model
/// prompt, no `--force` — the wizard flow never had any of those, and none
/// of them are part of what issue #347 asks to fix.
fn run_link_at(
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

    let linker = crate::linker::create_linker(target)?;

    let sources = &config.sources;
    let files = linker.generate(&link_agents, None, sources);

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
                // No coordinator in the wizard flow — same convention
                // `link` uses for a target that still emits a
                // team-roster document with none configured.
                crate::linker::manifest::ProducedBy::coordinator(target.to_string())
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
    use armadai_core::config::ENV_MUTEX;

    /// Points `ARMADAI_CONFIG_DIR` at a fresh, empty temp dir for the
    /// guard's lifetime, restoring it on drop — serialised on `ENV_MUTEX`
    /// (shared with the rest of the workspace's env-mutating tests, see
    /// `armadai_core::config`'s own tests and `web::api`'s
    /// `load_agents_declarative_tests`). Without this, `load_all_agents`'s
    /// shadowing check would scan whatever `~/.config/armadai/agents/`
    /// happens to hold on the machine running the test.
    struct IsolatedGlobalConfig {
        _lock: std::sync::MutexGuard<'static, ()>,
        orig_config_dir: Option<String>,
        _config_tmp: tempfile::TempDir,
    }

    impl IsolatedGlobalConfig {
        fn enter() -> Self {
            let lock = ENV_MUTEX.lock().unwrap();
            let orig_config_dir = std::env::var("ARMADAI_CONFIG_DIR").ok();
            let config_tmp = tempfile::tempdir().unwrap();
            // SAFETY: serialised via ENV_MUTEX above.
            unsafe {
                std::env::set_var("ARMADAI_CONFIG_DIR", config_tmp.path());
            }
            Self {
                _lock: lock,
                orig_config_dir,
                _config_tmp: config_tmp,
            }
        }
    }

    impl Drop for IsolatedGlobalConfig {
        fn drop(&mut self) {
            // SAFETY: still under the guard held by `self._lock`.
            unsafe {
                match &self.orig_config_dir {
                    Some(v) => std::env::set_var("ARMADAI_CONFIG_DIR", v),
                    None => std::env::remove_var("ARMADAI_CONFIG_DIR"),
                }
            }
        }
    }

    /// Extends [`IsolatedGlobalConfig`] with a process cwd change, for the
    /// one test below that must exercise `cli::unlink::execute` — which,
    /// unlike [`run_link_at`], has no root-taking variant and resolves its
    /// project via `project::find_project_config()`'s cwd read. Restores
    /// the original cwd on drop, still under the same env-mutex guard.
    struct IsolatedProjectDir {
        _config: IsolatedGlobalConfig,
        orig_cwd: std::path::PathBuf,
    }

    impl IsolatedProjectDir {
        fn enter(root: &Path) -> Self {
            let config = IsolatedGlobalConfig::enter();
            let orig_cwd = std::env::current_dir().unwrap();
            std::env::set_current_dir(root).unwrap();
            Self {
                _config: config,
                orig_cwd,
            }
        }
    }

    impl Drop for IsolatedProjectDir {
        fn drop(&mut self) {
            let _ = std::env::set_current_dir(&self.orig_cwd);
        }
    }

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
        run_link_at(&found_root, &config, "claude").unwrap();

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
    #[test]
    fn wizard_link_refuses_to_overwrite_a_hand_written_file() {
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
        run_link_at(&found_root, &config, "claude").unwrap();

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
    #[test]
    fn wizard_link_sees_agents_declared_only_in_agents_yaml() {
        let _isolated = IsolatedGlobalConfig::enter();

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
        run_link_at(&found_root, &config, "claude").unwrap();

        assert!(
            root.join(".claude/agents/declared-only.md").is_file(),
            "an agent declared only in .armadai/agents.yaml must still be linked by the wizard"
        );
    }
}
