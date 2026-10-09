mod config;
mod extract;
pub mod init;
mod inspect;
mod link;
mod list;
mod models;
pub(crate) mod new;
mod prompts;
mod registry;
pub(crate) mod setup;
mod skills;
pub(crate) mod style;
pub(crate) mod unlink;
mod update;
mod validate;

use clap::{CommandFactory, Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "armadai",
    about = "AI agent orchestrator",
    long_about = "AI agent orchestrator — define and manage specialized agents from Markdown files.\n\n\
        Each agent is a .md file in agents/ with metadata, system prompt, and optional instructions.\n\
        Supports any LLM provider (Claude, GPT, Gemini) via CLI tools or API.",
    version,
    arg_required_else_help = true,
    after_help = "Examples:\n  \
        armadai new my-agent --template dev-review --stack rust\n  \
        armadai list --tags dev --stack rust\n  \
        armadai link --target claude\n\n\
        Documentation: https://github.com/Dr0drigues/swarm-festai"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Create a new agent from a template
    #[command(
        long_about = "Create a new agent from a template.\n\n\
            Available templates: basic, dev-review, dev-test, cli-generic, planning, \
            security-review, debug, tech-debt, tdd-red, tdd-green, tdd-refactor, tech-writer.\n\
            The new agent is created at agents/<name>.md.\n\n\
            Use --interactive (-i) for a guided step-by-step creation wizard.",
        after_help = "Examples:\n  \
            armadai new my-assistant\n  \
            armadai new reviewer --template dev-review --stack rust\n  \
            armadai new scanner --template security-review -d \"audit OWASP top 10\"\n  \
            armadai new -i"
    )]
    New {
        /// Agent name (optional in interactive mode)
        name: Option<String>,
        /// Template to use
        #[arg(long, short, default_value = "basic", value_parser = crate::cli::new::template_value_parser())]
        template: String,
        /// Tech stack (replaces {{stack}} placeholder)
        #[arg(long, short)]
        stack: Option<String>,
        /// Agent description (replaces {{description}} placeholder)
        #[arg(long, short)]
        description: Option<String>,
        /// Interactive creation wizard
        #[arg(long, short = 'i')]
        interactive: bool,
    },
    /// List available agents
    #[command(after_help = "Examples:\n  \
        armadai list\n  \
        armadai list --tags dev review\n  \
        armadai list --stack rust")]
    List {
        /// Filter by tags
        #[arg(long)]
        tags: Option<Vec<String>>,
        /// Filter by stack
        #[arg(long)]
        stack: Option<String>,
        /// Show agents from the global library (~/.config/armadai/) only
        #[arg(long)]
        global: bool,
    },
    /// Inspect an agent's parsed configuration
    #[command(long_about = "Inspect an agent's parsed configuration.\n\n\
            Displays the fully parsed agent definition: metadata, system prompt, \
            instructions, output format, and pipeline configuration.")]
    Inspect {
        /// Agent name
        agent: String,
    },
    /// Validate starter pack or project config
    #[command(
        long_about = "Validate starter pack or project config.\n\n\
            Auto-detects whether the target is a starter pack (pack.yaml) or project \
            (armadai.yaml / .armadai/config.yaml) and runs the appropriate validation. \
            Checks agent/prompt/skill references, orchestration config, and trigger sections.",
        after_help = "Examples:\n  \
            armadai validate\n  \
            armadai validate starters/armadai-authoring\n  \
            armadai validate /path/to/project"
    )]
    Validate {
        /// Path to pack or project directory (default: current directory)
        path: Option<std::path::PathBuf>,
    },
    /// Extract agents, prompts, and skills with dependency resolution
    #[command(
        long_about = "Extract agents, prompts, and skills with dependency resolution.\n\n\
            Pulls resources from a starter pack, the user library, or the current project, \
            optionally including prompts that target the selected agents via `apply_to`. \
            When called with no flags, walks you through an interactive wizard.",
        after_help = "Examples:\n  \
            armadai extract\n  \
            armadai extract -i\n  \
            armadai extract --from armadai-authoring --agents authoring-lead --with-deps\n  \
            armadai extract --from user --agents dev-lead --out ./snapshot --as-pack"
    )]
    Extract(extract::ExtractArgs),
    /// Manage providers and secrets
    #[command(
        long_about = "Manage providers and secrets.\n\n\
            Configure API keys, view provider status, and manage SOPS + age encryption.",
        after_help = "Examples:\n  \
            armadai config providers\n  \
            armadai config secrets init\n  \
            armadai config secrets rotate"
    )]
    Config {
        #[command(subcommand)]
        action: config::ConfigAction,
    },
    /// Manage model deprecations and project registry
    #[command(
        subcommand,
        long_about = "Manage model deprecations and project registry.\n\n\
            Check for deprecated models in agent files and update them in-place. \
            Projects are auto-registered when you run `armadai link`.",
        after_help = "Examples:\n  \
            armadai models check\n  \
            armadai models check --all --prune\n  \
            armadai models update\n  \
            armadai models update --all\n  \
            armadai models list"
    )]
    Models(models::ModelsAction),
    /// Generate native config files for AI assistants
    #[command(
        long_about = "Generate native config files for AI assistants.\n\n\
            Reads project config (.armadai/config.yaml or armadai.yaml) and generates \
            target-specific configuration files (e.g. .claude/agents/*.md for Claude Code, \
            .github/agents/*.agent.md for GitHub Copilot). One source format, any target.",
        after_help = "Examples:\n  \
            armadai link --target claude\n  \
            armadai link --target copilot --dry-run\n  \
            armadai link --target claude --agents code-reviewer test-writer\n  \
            armadai link --target claude --output .claude/agents --force"
    )]
    Link {
        /// Target AI assistant
        #[arg(long, short, value_enum)]
        target: Option<crate::linker::LinkTarget>,
        /// Model to use in generated configs (for orchestrator targets like copilot/opencode)
        #[arg(long, short = 'm')]
        model: Option<String>,
        /// Coordinator agent whose prompt becomes the main context file
        #[arg(long, short = 'C')]
        coordinator: Option<String>,
        /// Preview generated files without writing
        #[arg(long)]
        dry_run: bool,
        /// Overwrite existing files without confirmation
        #[arg(long)]
        force: bool,
        /// Output directory (overrides config and defaults)
        #[arg(long, short)]
        output: Option<std::path::PathBuf>,
        /// Only link specific agents (by name)
        #[arg(long, num_args = 1..)]
        agents: Option<Vec<String>>,
    },
    /// Remove generated config files for AI assistants (reverse of link)
    #[command(
        long_about = "Remove generated config files for AI assistants.\n\n\
            Reverses the effect of `armadai link` by deleting the generated files. \
            Uses the same resolution logic as link to determine which files to remove.",
        after_help = "Examples:\n  \
            armadai unlink --target claude\n  \
            armadai unlink --target copilot --dry-run\n  \
            armadai unlink --target claude --with-config\n  \
            armadai unlink --target claude --agents code-reviewer test-writer"
    )]
    Unlink {
        /// Target AI assistant
        #[arg(long, short, value_enum)]
        target: Option<crate::linker::LinkTarget>,
        /// Coordinator agent whose prompt becomes the main context file
        #[arg(long, short = 'C')]
        coordinator: Option<String>,
        /// Preview files that would be removed without deleting
        #[arg(long)]
        dry_run: bool,
        /// Also remove the project config file (.armadai/config.yaml or armadai.yaml)
        #[arg(long)]
        with_config: bool,
        /// Output directory (must match the one used during link)
        #[arg(long, short)]
        output: Option<std::path::PathBuf>,
        /// Only unlink specific agents (by name)
        #[arg(long, num_args = 1..)]
        agents: Option<Vec<String>>,
    },
    /// Initialize ArmadAI configuration
    #[command(
        long_about = "Initialize ArmadAI configuration.\n\n\
            Creates ~/.config/armadai/ with default config.yaml, providers.yaml, \
            and subdirectories (agents/, prompts/, skills/, registry/).\n\n\
            Use --project to create a .armadai/ directory with config.yaml and \
            subdirectories (agents/, prompts/, skills/, starters/).",
        after_help = "Examples:\n  \
            armadai init\n  \
            armadai init --force\n  \
            armadai init --project\n  \
            armadai init --pack rust-dev\n  \
            armadai init --pack fullstack --force"
    )]
    Init {
        /// Overwrite existing config files
        #[arg(long)]
        force: bool,
        /// Create a project-local .armadai/ directory with config.yaml
        #[arg(long)]
        project: bool,
        /// Starter pack name, or path to a directory containing pack.yaml
        #[arg(long)]
        pack: Option<String>,
    },
    /// Browse and import agents from the community registry
    #[command(
        subcommand,
        long_about = "Browse and import agents from the community registry.\n\n\
            Integrates with awesome-copilot as a discovery and distribution mechanism. \
            Agents are converted from Copilot format to ArmadAI Markdown on import.",
        after_help = "Examples:\n  \
            armadai registry sync\n  \
            armadai registry search \"security review\"\n  \
            armadai registry list --category official\n  \
            armadai registry add official/security\n  \
            armadai registry info official/security"
    )]
    Registry(registry::RegistryAction),
    /// Manage composable prompts
    #[command(
        subcommand,
        long_about = "Manage composable prompt fragments.\n\n\
            Prompts are reusable Markdown files with optional YAML frontmatter \
            (name, description, apply_to). They compose with agents via the \
            apply_to field or explicit project config references.",
        after_help = "Examples:\n  \
            armadai prompts list\n  \
            armadai prompts show rust-conventions"
    )]
    Prompts(prompts::PromptsAction),
    /// Manage composable skills
    #[command(
        subcommand,
        long_about = "Manage composable skills.\n\n\
            Skills follow the SKILL.md open standard — structured knowledge with \
            scripts, references and assets. Each skill lives in a directory \
            containing a SKILL.md file.\n\n\
            Discover and install skills from GitHub repos with sync/search/add.",
        after_help = "Examples:\n  \
            armadai skills list\n  \
            armadai skills show docker-compose\n  \
            armadai skills sync\n  \
            armadai skills search \"testing\"\n  \
            armadai skills add anthropics/skills/webapp-testing\n  \
            armadai skills info webapp-testing"
    )]
    Skills(skills::SkillsAction),
    /// Self-update to the latest release
    #[command(long_about = "Self-update to the latest release.\n\n\
            Downloads the latest binary from GitHub Releases and replaces the current one.")]
    Update,
    /// Generate shell completion scripts
    #[command(after_help = "Examples:\n  \
        armadai completion bash > ~/.local/share/bash-completion/completions/armadai\n  \
        armadai completion zsh > ~/.zfunc/_armadai\n  \
        armadai completion fish > ~/.config/fish/completions/armadai.fish")]
    Completion {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
    /// Internal: called by the Claude Code `PreToolUse` hook on the Agent
    /// tool. Reads the hook JSON from stdin and enforces the declared
    /// delegation topology. Hidden from help.
    #[command(hide = true, name = "__claude-policy-gate")]
    ClaudePolicyGate,
}

/// Names of the subcommands marked `hide = true`, taken from clap itself
/// rather than from a naming convention.
fn hidden_subcommand_names() -> Vec<String> {
    Cli::command()
        .get_subcommands()
        .filter(|s| s.is_hide_set())
        .map(|s| s.get_name().to_string())
        .collect()
}

/// Remove hidden subcommands from a generated completion script.
///
/// `hide = true` keeps a subcommand out of `--help`, but clap_complete 4.6.8
/// still emits it into the generated script, so `armadai <TAB>` offers
/// internal commands like `__claude-policy-gate` — described, ironically, as
/// "Hidden from help".
///
/// Only the *offered candidates* are stripped. The per-command handling blocks
/// further down are left alone: they are unreachable unless someone types the
/// full internal name, and removing them would break the surrounding
/// case/switch syntax.
///
/// Matching is on exact names, never on a `__` prefix — fish's own helper
/// functions are called `__fish_armadai_*`, and a prefix filter would gut the
/// script.
fn strip_hidden_subcommands(script: &str, hidden: &[String]) -> String {
    if hidden.is_empty() {
        return script.to_string();
    }
    let mut out = String::with_capacity(script.len());
    'lines: for line in script.lines() {
        let trimmed = line.trim_start();
        for name in hidden {
            // zsh: `'<name>:description' \`
            if trimmed.starts_with(&format!("'{name}:")) {
                continue 'lines;
            }
            // fish: `... -f -a "<name>" -d '...'`
            if line.contains(&format!("-a \"{name}\"")) {
                continue 'lines;
            }
            // elvish: `cand <name> 'description'`
            if trimmed.starts_with(&format!("cand {name} ")) {
                continue 'lines;
            }
            // powershell: `[CompletionResult]::new('<name>', ...)`
            if line.contains(&format!("[CompletionResult]::new('{name}'")) {
                continue 'lines;
            }
        }
        // bash: `opts="new ... <name> ... help"`, and fish's
        // `not __fish_seen_subcommand_from ... <name> ...` guard lists.
        let mut kept = line.to_string();
        if kept.contains("opts=\"") || kept.contains("__fish_seen_subcommand_from") {
            for name in hidden {
                kept = kept
                    .replace(&format!(" {name} "), " ")
                    .replace(&format!(" {name}\""), "\"");
            }
        }
        out.push_str(&kept);
        out.push('\n');
    }
    out
}

pub async fn handle(cli: Cli) -> anyhow::Result<()> {
    match cli.command {
        Command::New {
            name,
            template,
            stack,
            description,
            interactive,
        } => new::execute(name, template, stack, description, interactive).await,
        Command::List {
            tags,
            stack,
            global,
        } => {
            armadai_core::config::set_force_global(global);
            list::execute(tags, stack).await
        }
        Command::Inspect { agent } => inspect::execute(agent).await,
        Command::Validate { path } => validate::execute(path).await,
        Command::Config { action } => config::execute(action).await,
        Command::Models(action) => models::execute(action).await,
        Command::Extract(args) => extract::execute(args).await,
        Command::Registry(action) => registry::execute(action).await,
        Command::Prompts(action) => prompts::execute(action).await,
        Command::Skills(action) => skills::execute(action).await,
        Command::Link {
            target,
            model,
            coordinator,
            dry_run,
            force,
            output,
            agents,
        } => link::execute(target, model, coordinator, dry_run, force, output, agents).await,
        Command::Unlink {
            target,
            coordinator,
            dry_run,
            with_config,
            output,
            agents,
        } => unlink::execute(target, coordinator, dry_run, with_config, output, agents).await,
        Command::Init {
            force,
            project,
            pack,
        } => init::execute(force, project, pack).await,
        Command::Update => update::execute().await,
        Command::Completion { shell } => {
            let mut buf = Vec::new();
            clap_complete::generate(shell, &mut Cli::command(), "armadai", &mut buf);
            let script = String::from_utf8_lossy(&buf);
            print!(
                "{}",
                strip_hidden_subcommands(&script, &hidden_subcommand_names())
            );
            Ok(())
        }
        Command::ClaudePolicyGate => crate::claude_adapter::policy_gate::gate_from_stdin(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Generate a completion script the way the `completion` handler does.
    fn completion(shell: clap_complete::Shell) -> String {
        let mut buf = Vec::new();
        clap_complete::generate(shell, &mut Cli::command(), "armadai", &mut buf);
        strip_hidden_subcommands(&String::from_utf8_lossy(&buf), &hidden_subcommand_names())
    }

    #[test]
    fn there_are_hidden_subcommands_to_strip() {
        // If this ever empties, the filter below stops proving anything.
        let hidden = hidden_subcommand_names();
        assert!(
            hidden.iter().any(|n| n == "__claude-policy-gate"),
            "expected the policy gate among hidden subcommands: {hidden:?}"
        );
    }

    /// `hide = true` keeps a subcommand out of `--help`, but clap_complete
    /// 4.6.8 still emits it into the generated script — so `armadai <TAB>`
    /// offered `__claude-policy-gate`, described as "Hidden from help".
    #[test]
    fn completion_scripts_do_not_offer_hidden_subcommands() {
        use clap::ValueEnum;
        // Every shell clap_complete supports, not a hand-picked three: the
        // filter shipped covering bash/zsh/fish while elvish and powershell
        // still offered the internal commands, and a hard-coded list could
        // not see it.
        for shell in clap_complete::Shell::value_variants() {
            let mut raw = Vec::new();
            clap_complete::generate(*shell, &mut Cli::command(), "armadai", &mut raw);
            let raw = String::from_utf8_lossy(&raw).to_string();
            let filtered = strip_hidden_subcommands(&raw, &hidden_subcommand_names());

            // The filter must actually remove something. Without this, a
            // clap_complete syntax change would make the filter a no-op AND
            // the assertions below vacuously true — the name would stay in
            // the script and nothing would notice.
            // Not "fewer lines": for bash the filter edits the `opts=` line
            // in place rather than dropping it. What must hold is that it
            // changed something.
            assert_ne!(
                filtered, raw,
                "{shell}: the filter changed nothing, so it has stopped working"
            );

            for name in hidden_subcommand_names() {
                for offered in [
                    format!("'{name}:"),                         // zsh
                    format!("-a \"{name}\""),                    // fish
                    format!("cand {name} "),                     // elvish
                    format!("[CompletionResult]::new('{name}'"), // powershell
                ] {
                    assert!(
                        !filtered.contains(&offered),
                        "{shell} still offers {name} via {offered:?}"
                    );
                }
                // bash/fish word lists.
                for line in filtered
                    .lines()
                    .filter(|l| l.contains("opts=\"") || l.contains("__fish_seen_subcommand_from"))
                {
                    assert!(
                        !line.split_whitespace().any(|w| w.trim_matches('"') == name),
                        "{shell} word list still contains {name}"
                    );
                }
            }
        }
    }

    #[test]
    fn completion_scripts_keep_the_real_commands() {
        // The filter must not be a blunt instrument.
        let zsh = completion(clap_complete::Shell::Zsh);
        for cmd in ["extract", "link", "unlink", "completion"] {
            assert!(
                zsh.contains(&format!("'{cmd}:")),
                "zsh completion lost the {cmd} command"
            );
        }
    }

    #[test]
    fn fish_helper_functions_survive_the_filter() {
        // fish names its own helpers `__fish_armadai_*`. Filtering on a `__`
        // prefix rather than on exact subcommand names would gut the script.
        let fish = completion(clap_complete::Shell::Fish);
        assert!(
            fish.matches("__fish_armadai").count() > 50,
            "the fish helpers were stripped along with the hidden commands"
        );
    }
}
