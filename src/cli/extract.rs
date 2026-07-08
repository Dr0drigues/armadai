//! `armadai extract` — extract agents, prompts, and skills with automatic
//! prompt-dependency resolution.
//!
//! Two modes:
//! - **Non-interactive**: full flag-driven (`--from`, `--agents`, ...).
//! - **Interactive wizard**: when no `--from` is given or `-i` is passed,
//!   walks the user through source selection, type picking, fuzzy item
//!   selection, dependency confirmation, and output path input via
//!   `dialoguer`.

use std::fs;
use std::path::{Path, PathBuf};

use clap::Args;
use dialoguer::{Confirm, FuzzySelect, Input, MultiSelect, theme::ColorfulTheme};

use crate::core::agent::Agent;
use crate::core::config;
use crate::core::dependency_resolver::resolve_dependencies;
use crate::core::prompt::{Prompt, load_all_prompts};
use crate::core::skill::{Skill, load_all_skills};
use crate::core::starter::{StarterPack, find_pack_dir, load_all_packs};

// ---------------------------------------------------------------------------
// CLI args
// ---------------------------------------------------------------------------

#[derive(Args, Debug, Default, Clone)]
pub struct ExtractArgs {
    /// Source: a starter pack name (e.g. `armadai-authoring`), `user`, or `project`.
    #[arg(long)]
    pub from: Option<String>,
    /// Agent names to extract.
    #[arg(long, num_args = 1..)]
    pub agents: Vec<String>,
    /// Prompt names to extract.
    #[arg(long, num_args = 1..)]
    pub prompts: Vec<String>,
    /// Skill names to extract.
    #[arg(long, num_args = 1..)]
    pub skills: Vec<String>,
    /// Auto-include prompts matching the selected agents via `apply_to`.
    #[arg(long)]
    pub with_deps: bool,
    /// Output directory (created if missing).
    #[arg(long, short = 'o', default_value = "./extracted")]
    pub out: PathBuf,
    /// Generate a `pack.yaml` manifest in the output directory.
    #[arg(long)]
    pub as_pack: bool,
    /// Force interactive wizard even if flags are provided.
    #[arg(long, short = 'i')]
    pub interactive: bool,
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub async fn execute(args: ExtractArgs) -> anyhow::Result<()> {
    let args = if args.interactive || args.from.is_none() {
        run_wizard(args)?
    } else {
        args
    };

    let from = args
        .from
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("--from is required (pack name, `user`, or `project`)"))?;

    let source = load_source_pool(from)?;
    let selected = select_resources(&args, &source);

    if selected.agents.is_empty() && selected.prompts.is_empty() && selected.skills.is_empty() {
        anyhow::bail!(
            "Nothing to extract: pass --agents/--prompts/--skills or use the interactive wizard."
        );
    }

    write_extracted(&args.out, &selected)?;

    if args.as_pack {
        let pack_name = args
            .out
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("extracted")
            .to_string();
        write_pack_yaml(&args.out, &pack_name, from, &selected)?;
    }

    print_summary(&args.out, &selected, args.as_pack);
    Ok(())
}

// ---------------------------------------------------------------------------
// Source loading
// ---------------------------------------------------------------------------

/// A loaded source pool of all candidate resources.
pub struct SourcePool {
    pub agents: Vec<Agent>,
    pub prompts: Vec<Prompt>,
    pub skills: Vec<Skill>,
}

pub fn load_source_pool(from: &str) -> anyhow::Result<SourcePool> {
    match from {
        "user" => Ok(load_user_library()),
        "project" => load_project_pool(),
        pack_name => load_pack_pool(pack_name),
    }
}

fn load_user_library() -> SourcePool {
    let agents = Agent::load_all(&config::user_agents_dir()).unwrap_or_default();
    let prompts = load_all_prompts(&config::user_prompts_dir());
    let skills = load_all_skills(&config::user_skills_dir());
    SourcePool {
        agents,
        prompts,
        skills,
    }
}

fn load_project_pool() -> anyhow::Result<SourcePool> {
    let cwd = std::env::current_dir()?;
    // Look for `.armadai/` then legacy root layout
    let project_dir = cwd.join(".armadai");
    let (agents_dir, prompts_dir, skills_dir) = if project_dir.is_dir() {
        (
            project_dir.join("agents"),
            project_dir.join("prompts"),
            project_dir.join("skills"),
        )
    } else {
        (cwd.join("agents"), cwd.join("prompts"), cwd.join("skills"))
    };

    Ok(SourcePool {
        agents: Agent::load_all(&agents_dir).unwrap_or_default(),
        prompts: load_all_prompts(&prompts_dir),
        skills: load_all_skills(&skills_dir),
    })
}

fn load_pack_pool(pack_name: &str) -> anyhow::Result<SourcePool> {
    let pack_dir =
        find_pack_dir(pack_name).ok_or_else(|| anyhow::anyhow!("Pack '{pack_name}' not found"))?;
    Ok(SourcePool {
        agents: Agent::load_all(&pack_dir.join("agents")).unwrap_or_default(),
        prompts: load_all_prompts(&pack_dir.join("prompts")),
        skills: load_all_skills(&pack_dir.join("skills")),
    })
}

// ---------------------------------------------------------------------------
// Selection
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct Selection {
    pub agents: Vec<Agent>,
    pub prompts: Vec<Prompt>,
    pub skills: Vec<Skill>,
}

/// Return the file-stem (kebab-case) identifier of a resource — this is what
/// `pack.yaml`, `armadai run`, and other CLI commands use to refer to it.
fn file_stem(p: &Path) -> String {
    p.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string()
}

/// Match a CLI-supplied identifier against a resource: file-stem first
/// (canonical), then display name fallback (so `--agents "Dev Lead"` also
/// works for users who copy the H1 instead of the filename).
fn matches_id(arg_list: &[String], file: &Path, display_name: &str) -> bool {
    let stem = file_stem(file);
    arg_list.iter().any(|a| a == &stem || a == display_name)
}

pub fn select_resources(args: &ExtractArgs, pool: &SourcePool) -> Selection {
    let agents: Vec<Agent> = pool
        .agents
        .iter()
        .filter(|a| matches_id(&args.agents, &a.source, &a.name))
        .cloned()
        .collect();

    let mut prompts: Vec<Prompt> = pool
        .prompts
        .iter()
        .filter(|p| matches_id(&args.prompts, &p.source, &p.name))
        .cloned()
        .collect();

    if args.with_deps && !agents.is_empty() {
        let deps = resolve_dependencies(&agents, &pool.prompts);
        for dep in deps.prompts {
            if !prompts.iter().any(|p| p.source == dep.source) {
                prompts.push(dep);
            }
        }
    }

    let skills: Vec<Skill> = pool
        .skills
        .iter()
        .filter(|s| matches_id(&args.skills, &s.source, &s.name))
        .cloned()
        .collect();

    Selection {
        agents,
        prompts,
        skills,
    }
}

// ---------------------------------------------------------------------------
// Write to disk
// ---------------------------------------------------------------------------

pub fn write_extracted(out: &Path, selected: &Selection) -> anyhow::Result<()> {
    fs::create_dir_all(out)?;

    if !selected.agents.is_empty() {
        let dst = out.join("agents");
        fs::create_dir_all(&dst)?;
        for agent in &selected.agents {
            copy_file(&agent.source, &dst)?;
        }
    }

    if !selected.prompts.is_empty() {
        let dst = out.join("prompts");
        fs::create_dir_all(&dst)?;
        for prompt in &selected.prompts {
            copy_file(&prompt.source, &dst)?;
        }
    }

    if !selected.skills.is_empty() {
        let dst = out.join("skills");
        fs::create_dir_all(&dst)?;
        for skill in &selected.skills {
            let skill_dst = dst.join(&skill.name);
            copy_dir_recursive(&skill.source, &skill_dst)?;
        }
    }

    Ok(())
}

fn copy_file(src: &Path, dst_dir: &Path) -> anyhow::Result<()> {
    let filename = src
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("Invalid source path: {}", src.display()))?;
    fs::copy(src, dst_dir.join(filename))?;
    Ok(())
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let path = entry.path();
        let target = dst.join(entry.file_name());
        if path.is_dir() {
            copy_dir_recursive(&path, &target)?;
        } else {
            fs::copy(&path, &target)?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// pack.yaml generation
// ---------------------------------------------------------------------------

fn write_pack_yaml(
    out: &Path,
    pack_name: &str,
    source_label: &str,
    selected: &Selection,
) -> anyhow::Result<()> {
    // pack.yaml references resources by their file-stem (canonical id), not
    // the H1 display name.
    let agent_ids: Vec<String> = selected
        .agents
        .iter()
        .map(|a| file_stem(&a.source))
        .collect();
    let prompt_ids: Vec<String> = selected
        .prompts
        .iter()
        .map(|p| file_stem(&p.source))
        .collect();
    let skill_ids: Vec<String> = selected
        .skills
        .iter()
        .map(|s| file_stem(&s.source))
        .collect();

    let agents_yaml = list_yaml("agents", agent_ids.iter().map(String::as_str));
    let prompts_yaml = list_yaml("prompts", prompt_ids.iter().map(String::as_str));
    let skills_yaml = list_yaml("skills", skill_ids.iter().map(String::as_str));

    let manifest = format!(
        "name: {pack_name}\ndescription: \"Extracted from {source_label}\"\n{agents_yaml}{prompts_yaml}{skills_yaml}"
    );
    fs::write(out.join("pack.yaml"), manifest)?;
    Ok(())
}

fn list_yaml<'a, I: IntoIterator<Item = &'a str>>(key: &str, items: I) -> String {
    let mut iter = items.into_iter().peekable();
    if iter.peek().is_none() {
        return String::new();
    }
    let mut out = format!("{key}:\n");
    for name in iter {
        out.push_str(&format!("  - {name}\n"));
    }
    out
}

// ---------------------------------------------------------------------------
// Interactive wizard
// ---------------------------------------------------------------------------

fn run_wizard(mut args: ExtractArgs) -> anyhow::Result<ExtractArgs> {
    let theme = ColorfulTheme::default();

    // 1. Source selection
    if args.from.is_none() {
        let mut source_labels: Vec<String> = vec!["user".into(), "project".into()];
        let packs: Vec<StarterPack> = load_all_packs();
        for p in &packs {
            source_labels.push(format!("pack:{}", p.name));
        }
        let idx = FuzzySelect::with_theme(&theme)
            .with_prompt("Source")
            .items(&source_labels)
            .default(0)
            .interact()?;
        let label = &source_labels[idx];
        args.from = Some(label.strip_prefix("pack:").unwrap_or(label).to_string());
    }

    // 2. Type selection
    let type_labels = vec!["agents", "prompts", "skills"];
    let pool = load_source_pool(args.from.as_ref().unwrap())?;
    let defaults = [
        !pool.agents.is_empty(),
        !pool.prompts.is_empty(),
        !pool.skills.is_empty(),
    ];
    let type_picks = MultiSelect::with_theme(&theme)
        .with_prompt("Types to extract (space to toggle, enter to confirm)")
        .items(&type_labels)
        .defaults(&defaults)
        .interact()?;
    let want_agents = type_picks.contains(&0);
    let want_prompts = type_picks.contains(&1);
    let want_skills = type_picks.contains(&2);

    if want_agents && args.agents.is_empty() && !pool.agents.is_empty() {
        let names: Vec<&str> = pool.agents.iter().map(|a| a.name.as_str()).collect();
        let picks = MultiSelect::with_theme(&theme)
            .with_prompt("Select agents")
            .items(&names)
            .interact()?;
        args.agents = picks.into_iter().map(|i| names[i].to_string()).collect();
    }

    if want_prompts && args.prompts.is_empty() && !pool.prompts.is_empty() {
        let names: Vec<&str> = pool.prompts.iter().map(|p| p.name.as_str()).collect();
        let picks = MultiSelect::with_theme(&theme)
            .with_prompt("Select prompts")
            .items(&names)
            .interact()?;
        args.prompts = picks.into_iter().map(|i| names[i].to_string()).collect();
    }

    if want_skills && args.skills.is_empty() && !pool.skills.is_empty() {
        let names: Vec<&str> = pool.skills.iter().map(|s| s.name.as_str()).collect();
        let picks = MultiSelect::with_theme(&theme)
            .with_prompt("Select skills")
            .items(&names)
            .interact()?;
        args.skills = picks.into_iter().map(|i| names[i].to_string()).collect();
    }

    // 3. Deps
    if !args.agents.is_empty() && !args.with_deps {
        args.with_deps = Confirm::with_theme(&theme)
            .with_prompt("Auto-include matching prompts via apply_to?")
            .default(true)
            .interact()?;
    }

    // 4. Output path
    let default_out = args.out.to_string_lossy().to_string();
    let out: String = Input::with_theme(&theme)
        .with_prompt("Output directory")
        .default(default_out)
        .interact_text()?;
    args.out = PathBuf::from(out);

    // 5. as-pack
    if !args.as_pack {
        args.as_pack = Confirm::with_theme(&theme)
            .with_prompt("Generate pack.yaml?")
            .default(false)
            .interact()?;
    }

    Ok(args)
}

// ---------------------------------------------------------------------------
// Summary
// ---------------------------------------------------------------------------

fn print_summary(out: &Path, selected: &Selection, as_pack: bool) {
    println!(
        "\nExtracted {} agent(s), {} prompt(s), {} skill(s) -> {}",
        selected.agents.len(),
        selected.prompts.len(),
        selected.skills.len(),
        out.display()
    );
    if as_pack {
        println!("Generated pack.yaml at {}", out.join("pack.yaml").display());
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn write_agent(dir: &Path, name: &str) {
        write_agent_with_h1(dir, name, name);
    }

    /// Create an agent file `<file_stem>.md` whose H1 (the parser-reported name)
    /// is `display_name`. Lets us exercise the file-stem-vs-display-name
    /// matching path that `armadai extract` relies on.
    fn write_agent_with_h1(dir: &Path, file_stem: &str, display_name: &str) {
        fs::create_dir_all(dir).unwrap();
        let content = format!(
            "# {display_name}\n\n## Metadata\n- provider: claude\n- model: latest:pro\n\n## System Prompt\n\nYou are {display_name}.\n"
        );
        fs::write(dir.join(format!("{file_stem}.md")), content).unwrap();
    }

    fn write_prompt(dir: &Path, name: &str, apply_to: &[&str]) {
        fs::create_dir_all(dir).unwrap();
        // YAML: quote every target — `*` is otherwise parsed as an anchor reference.
        let targets = apply_to
            .iter()
            .map(|s| format!("  - \"{s}\""))
            .collect::<Vec<_>>()
            .join("\n");
        let content = format!(
            "---\nname: {name}\ndescription: test prompt\napply_to:\n{targets}\n---\nBody.\n"
        );
        fs::write(dir.join(format!("{name}.md")), content).unwrap();
    }

    fn write_skill(dir: &Path, name: &str) {
        let skill_dir = dir.join(name);
        fs::create_dir_all(&skill_dir).unwrap();
        let content = format!(
            "---\nname: {name}\ndescription: test skill\nversion: \"1.0\"\n---\n\n# {name}\n\nBody.\n"
        );
        fs::write(skill_dir.join("SKILL.md"), content).unwrap();
        let refs = skill_dir.join("references");
        fs::create_dir_all(&refs).unwrap();
        fs::write(refs.join("guide.md"), "# Reference\nContent.").unwrap();
    }

    fn build_source(tmp: &Path) -> SourcePool {
        let agents_dir = tmp.join("agents");
        let prompts_dir = tmp.join("prompts");
        let skills_dir = tmp.join("skills");
        write_agent(&agents_dir, "reviewer");
        write_agent(&agents_dir, "writer");
        write_prompt(&prompts_dir, "rust-conv", &["reviewer"]);
        write_prompt(&prompts_dir, "global", &["*"]);
        write_skill(&skills_dir, "code-quality");

        SourcePool {
            agents: Agent::load_all(&agents_dir).unwrap(),
            prompts: load_all_prompts(&prompts_dir),
            skills: load_all_skills(&skills_dir),
        }
    }

    #[test]
    fn selects_named_agents_only() {
        let tmp = tempfile::tempdir().unwrap();
        let pool = build_source(tmp.path());
        let args = ExtractArgs {
            agents: vec!["reviewer".into()],
            ..Default::default()
        };

        let selected = select_resources(&args, &pool);

        assert_eq!(selected.agents.len(), 1);
        assert_eq!(selected.agents[0].name, "reviewer");
        assert!(selected.prompts.is_empty());
    }

    #[test]
    fn with_deps_pulls_matching_prompts() {
        let tmp = tempfile::tempdir().unwrap();
        let pool = build_source(tmp.path());
        let args = ExtractArgs {
            agents: vec!["reviewer".into()],
            with_deps: true,
            ..Default::default()
        };

        let selected = select_resources(&args, &pool);

        let prompt_names: Vec<&str> = selected.prompts.iter().map(|p| p.name.as_str()).collect();
        assert!(prompt_names.contains(&"rust-conv"));
        assert!(prompt_names.contains(&"global"));
    }

    #[test]
    fn without_deps_no_auto_prompts() {
        let tmp = tempfile::tempdir().unwrap();
        let pool = build_source(tmp.path());
        let args = ExtractArgs {
            agents: vec!["reviewer".into()],
            with_deps: false,
            ..Default::default()
        };

        let selected = select_resources(&args, &pool);

        assert!(selected.prompts.is_empty());
    }

    #[test]
    fn deduplicates_explicit_and_dep_prompts() {
        let tmp = tempfile::tempdir().unwrap();
        let pool = build_source(tmp.path());
        let args = ExtractArgs {
            agents: vec!["reviewer".into()],
            prompts: vec!["rust-conv".into()],
            with_deps: true,
            ..Default::default()
        };

        let selected = select_resources(&args, &pool);

        let rust_conv_count = selected
            .prompts
            .iter()
            .filter(|p| p.name == "rust-conv")
            .count();
        assert_eq!(rust_conv_count, 1);
    }

    #[test]
    fn matches_file_stem_when_h1_differs() {
        // Mirrors the real-world case where dev-lead.md has H1 "Dev Lead":
        // users pass the file-stem (the canonical id used in pack.yaml/apply_to),
        // not the display name.
        let tmp = tempfile::tempdir().unwrap();
        let agents_dir = tmp.path().join("agents");
        let prompts_dir = tmp.path().join("prompts");
        write_agent_with_h1(&agents_dir, "dev-lead", "Dev Lead");
        write_prompt(&prompts_dir, "rust-conv", &["dev-lead"]);

        let pool = SourcePool {
            agents: Agent::load_all(&agents_dir).unwrap(),
            prompts: load_all_prompts(&prompts_dir),
            skills: vec![],
        };
        let args = ExtractArgs {
            agents: vec!["dev-lead".into()],
            with_deps: true,
            ..Default::default()
        };

        let selected = select_resources(&args, &pool);

        assert_eq!(selected.agents.len(), 1);
        assert_eq!(selected.agents[0].name, "Dev Lead");
        assert_eq!(selected.prompts.len(), 1, "deps must follow apply_to id");
        assert_eq!(selected.prompts[0].name, "rust-conv");
    }

    #[test]
    fn skills_are_explicit_only() {
        let tmp = tempfile::tempdir().unwrap();
        let pool = build_source(tmp.path());
        let args = ExtractArgs {
            agents: vec!["reviewer".into()],
            with_deps: true,
            ..Default::default()
        };

        let selected = select_resources(&args, &pool);

        assert!(selected.skills.is_empty(), "skills must not auto-resolve");
    }

    #[test]
    fn write_creates_expected_layout() {
        let tmp = tempfile::tempdir().unwrap();
        let pool = build_source(tmp.path());
        let selection = Selection {
            agents: pool.agents.clone(),
            prompts: pool.prompts.clone(),
            skills: pool.skills.clone(),
        };

        let out = tmp.path().join("out");
        write_extracted(&out, &selection).unwrap();

        assert!(out.join("agents/reviewer.md").is_file());
        assert!(out.join("agents/writer.md").is_file());
        assert!(out.join("prompts/rust-conv.md").is_file());
        assert!(out.join("prompts/global.md").is_file());
        assert!(out.join("skills/code-quality/SKILL.md").is_file());
        assert!(
            out.join("skills/code-quality/references/guide.md")
                .is_file()
        );
    }

    #[test]
    fn write_skips_empty_categories() {
        let tmp = tempfile::tempdir().unwrap();
        let pool = build_source(tmp.path());
        let selection = Selection {
            agents: vec![pool.agents[0].clone()],
            prompts: vec![],
            skills: vec![],
        };

        let out = tmp.path().join("out2");
        write_extracted(&out, &selection).unwrap();

        assert!(out.join("agents").is_dir());
        assert!(!out.join("prompts").exists());
        assert!(!out.join("skills").exists());
    }

    #[test]
    fn pack_yaml_lists_selected_resources() {
        let tmp = tempfile::tempdir().unwrap();
        let pool = build_source(tmp.path());
        let selection = Selection {
            agents: vec![pool.agents[0].clone()],
            prompts: vec![pool.prompts[0].clone()],
            skills: vec![],
        };

        let out = tmp.path().join("out3");
        write_extracted(&out, &selection).unwrap();
        write_pack_yaml(&out, "my-extract", "user", &selection).unwrap();

        let yaml = fs::read_to_string(out.join("pack.yaml")).unwrap();
        assert!(yaml.contains("name: my-extract"));
        assert!(yaml.contains("Extracted from user"));
        assert!(yaml.contains("- reviewer") || yaml.contains("- writer"));
        // No skills section when empty
        assert!(!yaml.contains("skills:"));
    }
}
