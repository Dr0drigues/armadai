mod config;
mod graph;
pub mod init;
mod inspect;
mod link;
mod list;
mod migrate;
mod models;
mod native_pack;
pub(crate) mod new;
mod new_target;
mod new_wizard;
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
    about = "Declarative agent teams, compiled for Claude Code, Codex, OpenCode and Copilot",
    long_about = "Declarative agent teams — describe them once, compile them for Claude Code, Codex, \
        OpenCode and Copilot.\n\n\
        A project is a `.armadai/` folder: project.yaml, one Markdown file per agent (YAML \
        frontmatter, prompt as the body), skills, flows and automations.\n\
        `armadai link` writes the native configuration files of each target assistant and a \
        conformance report; ArmadAI does not run agents itself. A legacy project is converted \
        once with `armadai migrate`.",
    version,
    arg_required_else_help = true,
    after_help = "Examples:\n  \
        armadai init --project\n  \
        armadai new agent my-agent --template dev-review --stack rust\n  \
        armadai init --pack rust-dev\n  \
        armadai new flow review --template review-loop --agents dev,reviewer\n  \
        armadai graph review\n  \
        armadai link --target claude\n\n\
        Documentation: https://github.com/Dr0drigues/armadai"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Create an agent, a flow, an automation or a skill
    #[command(
        long_about = "Create an agent, a flow, an automation or a skill in the project of the \
            current directory (armadai init --project first).\n\n\
            Each command writes one file under .armadai/, then validates the project: a \
            creation that would introduce an error is undone and nothing is left behind. \
            Ids are lowercase letters, digits and hyphens. Without a kind, a guided menu \
            opens (a terminal is needed); `-i` asks for every value of one kind.\n\n\
            Agent templates: basic, dev-review, dev-test, planning, security-review, debug, \
            tech-debt, tdd-red, tdd-green, tdd-refactor, tech-writer (built into the binary; a \
            same-named file in ./templates or ~/.config/armadai/templates overrides one, and \
            adds your own). Flow templates: pipeline, review-loop, plan-build-verify, fan-out.",
        after_help = "Examples:\n  \
            armadai new agent reviewer --template dev-review --stack rust\n  \
            armadai new flow review --template review-loop --agents dev,reviewer\n  \
            armadai new automation fmt --event after-tool --run \"cargo fmt\"\n  \
            armadai new skill release-notes --description \"Write release notes\""
    )]
    New {
        #[command(subcommand)]
        what: Option<new::NewCommand>,
    },
    /// List the project's agents, skills, flows and automations
    #[command(after_help = "Examples:\n  \
        armadai list\n  \
        armadai list --tags dev review")]
    List {
        /// Only agents carrying one of these tags
        #[arg(long, num_args = 1..)]
        tags: Option<Vec<String>>,
    },
    /// Show one agent of the project
    #[command(long_about = "Show one agent of the project.\n\n\
            Displays its frontmatter (description, tier, tags, skills, capabilities, \
            native targets), the flows it takes part in, and its prompt.")]
    Inspect {
        /// Agent id (its file name under .armadai/agents/)
        agent: String,
    },
    /// Print the graph of a flow (or of every flow) as Mermaid
    #[command(
        long_about = "Print the graph of a flow as Mermaid (flowchart TD) on stdout, and \
            nothing else: redirect it to a .mmd file or paste it into a Markdown ```mermaid \
            block.\n\n\
            Without a flow, every flow of the project is printed, one diagram each, opened by \
            a `%% flow: <id>` comment and separated by a blank line (Mermaid renders one \
            diagram per document: take them one at a time). Conditional links are labelled \
            with their state; failure paths (`when: failed`, `on_failure`) are dotted.",
        after_help = "Examples:\n  \
            armadai graph review > review.mmd\n  \
            armadai graph"
    )]
    Graph {
        /// Flow id (its file name under .armadai/flows/); every flow when omitted
        flow: Option<String>,
        /// Output format
        #[arg(long, value_enum, default_value_t = graph::GraphFormat::Mermaid)]
        format: graph::GraphFormat,
    },
    /// Validate the project (.armadai/) or a starter pack
    #[command(
        long_about = "Validate the project (.armadai/project.yaml and its folders) or a \
            starter pack (pack.yaml). A legacy project must be migrated first \
            (armadai migrate).",
        after_help = "Examples:\n  \
            armadai validate\n  \
            armadai validate starters/armadai-authoring\n  \
            armadai validate /path/to/project"
    )]
    Validate {
        /// Path to pack or project directory (default: current directory)
        path: Option<std::path::PathBuf>,
        /// Print the diagnostics of a new-format project as a JSON array
        #[arg(long)]
        json: bool,
    },
    /// Convert a legacy project to the new .armadai/project.yaml format
    #[command(
        long_about = "Convert a legacy project to the new .armadai/project.yaml format.\n\n\
            Reads the legacy config (.armadai/config.yaml, armadai.yaml or armadai.yml), its \
            agents, declared agents, prompts and skills, and writes the new format under \
            .armadai/. The legacy files are first copied to .armadai/.migrated/<timestamp>/, \
            then removed: .armadai/ legacy content and the config file that was read (a root \
            armadai.yaml/armadai.yml is removed when it is the one read). Other files are left \
            in place and reported. The report lists \
            the files written, every dropped field with its reason, and the warnings.\n\n\
            Then run 'armadai link' to compile the migrated project.",
        after_help = "Examples:\n  \
            armadai migrate --dry-run\n  \
            armadai migrate\n  \
            armadai migrate --path /path/to/project --force"
    )]
    Migrate {
        /// Project directory (default: current directory)
        #[arg(long)]
        path: Option<std::path::PathBuf>,
        /// Convert and validate without writing anything; print the report
        #[arg(long)]
        dry_run: bool,
        /// Migrate again although .armadai/project.yaml already exists; the legacy
        /// project must first be restored (e.g. from .armadai/.migrated/<timestamp>/)
        #[arg(long)]
        force: bool,
    },
    /// Manage the user configuration (custom starter pack directories)
    #[command(after_help = "Examples:\n  \
            armadai config starters-dir list\n  \
            armadai config starters-dir add ~/my-starters\n  \
            armadai config starters-dir remove ~/my-starters")]
    Config {
        #[command(subcommand)]
        action: config::ConfigAction,
    },
    /// Manage model deprecations and project registry
    #[command(
        subcommand,
        long_about = "Manage model deprecations and project registry.\n\n\
            Check the model ids agents pin in `native.<target>.model` for deprecated ones, \
            and replace them in place (agents otherwise carry a tier). \
            Projects are auto-registered when you run `armadai link`.",
        after_help = "Examples:\n  \
            armadai models check\n  \
            armadai models check --all --prune\n  \
            armadai models update\n  \
            armadai models update --all\n  \
            armadai models list"
    )]
    Models(models::ModelsAction),
    /// Compile the project (.armadai/) into native config files for AI assistants
    #[command(
        long_about = "Compile the project into native config files for AI assistants.\n\n\
            Reads .armadai/project.yaml and its folders (agents, skills, flows, \
            automations), validates them, and writes each target's files (e.g. \
            .claude/agents/*.md, CLAUDE.md and .claude/settings.json hooks for Claude Code). \
            Prints a conformance report: what each target compiled, approximated or \
            could not compile. A legacy project must be migrated first (armadai migrate).",
        after_help = "Examples:\n  \
            armadai link\n  \
            armadai link --target claude --dry-run\n  \
            armadai link --target claude codex\n  \
            armadai link --target claude --agents reviewer tester\n  \
            armadai link --json"
    )]
    Link {
        /// Targets (default: `targets` of project.yaml, else claude)
        #[arg(long, short, value_enum, num_args = 1..)]
        target: Vec<crate::linker::LinkTarget>,
        /// Preview generated files and the report without writing
        #[arg(long)]
        dry_run: bool,
        /// Overwrite existing files that differ from what link writes
        #[arg(long)]
        force: bool,
        /// Output directory (one target only; overrides `outputs` of project.yaml)
        #[arg(long, short)]
        output: Option<std::path::PathBuf>,
        /// Only write these agents' files (ids); CLAUDE.md and hooks are unaffected
        #[arg(long, num_args = 1..)]
        agents: Option<Vec<String>>,
        /// Print the conformance report as JSON on stdout, and nothing else
        #[arg(long)]
        json: bool,
    },
    /// Remove what `link` wrote (reverse of link)
    #[command(
        long_about = "Remove what `armadai link` wrote.\n\n\
            Reads the link manifest (.armadai/link-manifest.yaml): removes the files link \
            wrote that are still unchanged, and only the ArmadAI hooks and CLAUDE.md block \
            from files shared with you. Without a manifest, removes only files whose content \
            still equals what link would write now.",
        after_help = "Examples:\n  \
            armadai unlink\n  \
            armadai unlink --target copilot --dry-run\n  \
            armadai unlink --target claude --agents reviewer tester"
    )]
    Unlink {
        /// Targets (default: `targets` of project.yaml, else claude)
        #[arg(long, short, value_enum, num_args = 1..)]
        target: Vec<crate::linker::LinkTarget>,
        /// Preview what would be removed without deleting
        #[arg(long)]
        dry_run: bool,
        /// Output directory (must match the one used during link)
        #[arg(long, short)]
        output: Option<std::path::PathBuf>,
        /// Only unlink these agents' files (ids)
        #[arg(long, num_args = 1..)]
        agents: Option<Vec<String>>,
    },
    /// Initialize ArmadAI configuration
    #[command(
        long_about = "Initialize ArmadAI configuration.\n\n\
            Creates ~/.config/armadai/ with default config.yaml, \
            skills/ (the built-in authoring skills, and skills installed by \
            `armadai skills add`) and registry/ (the cache for remote starter packs).\n\n\
            Use --project to create an empty project: .armadai/project.yaml, agents/, \
            skills/, flows/, automations/ and .armadai/.gitignore.\n\n\
            Use --pack to create the project from a starter pack instead. A native pack \
            (`format: 1` in its pack.yaml) is validated, then copied: project.yaml, agents/, \
            skills/, flows/ and automations/; its automations are listed (they become \
            hooks). A pack without `format` is the legacy format, converted like \
            `armadai migrate`. With --force, a pack is merged into an existing project: an \
            existing file is kept, and so is the existing project.yaml.",
        after_help = "Examples:\n  \
            armadai init\n  \
            armadai init --project\n  \
            armadai init --pack rust-dev\n  \
            armadai init --pack ./my-pack\n  \
            armadai init --pack fullstack --force"
    )]
    Init {
        /// Overwrite existing config files
        #[arg(long)]
        force: bool,
        /// Create an empty project (.armadai/project.yaml and its folders) here
        #[arg(long)]
        project: bool,
        /// Create the project from this starter pack (name, or a directory holding pack.yaml)
        #[arg(long)]
        pack: Option<String>,
    },
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
        Command::New { what } => new::execute(what),
        Command::List { tags } => list::execute(tags).await,
        Command::Inspect { agent } => inspect::execute(agent).await,
        Command::Graph { flow, format } => graph::execute(flow, format),
        Command::Validate { path, json } => validate::execute(path, json).await,
        Command::Migrate {
            path,
            dry_run,
            force,
        } => migrate::execute(path, dry_run, force),
        Command::Config { action } => config::execute(action).await,
        Command::Models(action) => models::execute(action).await,
        Command::Skills(action) => skills::execute(action).await,
        Command::Link {
            target,
            dry_run,
            force,
            output,
            agents,
            json,
        } => link::execute(target, dry_run, force, output, agents, json).await,
        Command::Unlink {
            target,
            dry_run,
            output,
            agents,
        } => unlink::execute(target, dry_run, output, agents).await,
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
    fn every_kept_command_has_a_help_description() {
        use clap::CommandFactory;
        let root = Cli::command();
        let help = root.clone().render_help().to_string();
        for cmd in [
            "new",
            "list",
            "inspect",
            "graph",
            "validate",
            "config",
            "models",
            "skills",
            "link",
            "unlink",
            "init",
            "update",
            "completion",
        ] {
            let sub = root
                .find_subcommand(cmd)
                .unwrap_or_else(|| panic!("`{cmd}` missing from top-level help:\n{help}"));
            // Without an explicit short description clap silently falls back
            // to the first line of `long_about`, so check `about` itself.
            let about = sub.get_about().map(|a| a.to_string()).unwrap_or_default();
            assert!(
                !about.trim().is_empty(),
                "`{cmd}` has no short description (doc comment) in --help:\n{help}"
            );
        }
    }

    #[test]
    fn completion_scripts_keep_the_real_commands() {
        // The filter must not be a blunt instrument.
        let zsh = completion(clap_complete::Shell::Zsh);
        for cmd in ["migrate", "link", "unlink", "completion"] {
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
