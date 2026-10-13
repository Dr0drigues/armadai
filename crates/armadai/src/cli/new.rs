use std::path::Path;

use include_dir::{Dir, include_dir};

use armadai_core::config::AppPaths;

/// Agent templates shipped inside the binary, so `armadai new` works on a
/// fresh machine. A same-named file in the templates directory overrides them.
static EMBEDDED_TEMPLATES: Dir = include_dir!("$CARGO_MANIFEST_DIR/templates");

/// Content of an embedded template, if there is one with this name.
fn embedded_template(name: &str) -> Option<&'static str> {
    EMBEDDED_TEMPLATES
        .get_file(format!("{name}.md"))
        .and_then(|f| f.contents_utf8())
}

/// Resolve a template: `templates_dir/<name>.md` first (project or user
/// directory), then the embedded set.
fn load_template_from(templates_dir: &Path, name: &str) -> Option<String> {
    std::fs::read_to_string(templates_dir.join(format!("{name}.md")))
        .ok()
        .or_else(|| embedded_template(name).map(str::to_string))
}

/// Sorted, de-duplicated names of the templates in `templates_dir` plus the
/// embedded ones.
fn template_names_from(templates_dir: &Path) -> Vec<String> {
    let mut names = collect_template_names(templates_dir);
    names.extend(
        EMBEDDED_TEMPLATES
            .files()
            .filter(|f| f.path().extension().is_some_and(|ext| ext == "md"))
            .filter_map(|f| f.path().file_stem())
            .map(|s| s.to_string_lossy().to_string()),
    );
    names.sort();
    names.dedup();
    names
}

pub async fn execute(
    name: Option<String>,
    template: String,
    stack: Option<String>,
    description: Option<String>,
    interactive: bool,
) -> anyhow::Result<()> {
    if interactive {
        anyhow::bail!("interactive mode is not available yet; pass the options");
    }
    let name =
        name.ok_or_else(|| anyhow::anyhow!("Agent name is required (or use --interactive)"))?;
    let paths = AppPaths::resolve();
    let output_path = agent_destination(&name)?;
    let Some(source) = load_template_from(&paths.templates_dir, &template) else {
        let e = crate::cli::style::err();
        anstream::eprintln!("{e}Template '{template}' not found.{e:#}\n");
        list_templates(&paths.templates_dir)?;
        anyhow::bail!("Use --template <name> with one of the templates above");
    };
    let opts = AgentOptions {
        description,
        stack,
        ..AgentOptions::default()
    };
    let (agent, remaining) = agent_from_template(&name, &template, &source, &opts)?;
    if let Some(dir) = output_path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&output_path, armadai_model::render_agent(&name, &agent))?;

    let o = crate::cli::style::ok();
    let m = crate::cli::style::muted();
    anstream::println!("{o}Agent created:{o:#} {m}{}{m:#}", output_path.display());
    warn_placeholders(&remaining);
    Ok(())
}

fn warn_placeholders(remaining: &[String]) {
    if remaining.is_empty() {
        return;
    }
    let w = crate::cli::style::warn();
    anstream::eprintln!("{w}  Note: these placeholders are left for you to fill in:{w:#}");
    for p in remaining {
        anstream::eprintln!("{w}    - {{{{{p}}}}}{w:#}");
    }
}

/// `.armadai/agents/<name>.md` of the project at or above the current directory;
/// refused for an invalid id or an agent that already exists.
fn agent_destination(name: &str) -> anyhow::Result<std::path::PathBuf> {
    if !armadai_model::is_valid_id(name) {
        anyhow::bail!("`{name}` is not a valid agent id ([a-z0-9][a-z0-9-]{{0,63}})");
    }
    let root = crate::compile::find_project(&std::env::current_dir()?)
        .map_err(|e| anyhow::anyhow!("{e}\nCreate a project first: `armadai init --project`"))?;
    let path = root.join(".armadai/agents").join(format!("{name}.md"));
    if path.exists() {
        anyhow::bail!("Agent '{name}' already exists at {}", path.display());
    }
    Ok(path)
}

/// What the user sets on top of a template; `None` keeps the template's value.
#[derive(Debug, Default, Clone)]
pub struct AgentOptions {
    pub description: Option<String>,
    pub stack: Option<String>,
    pub tier: Option<armadai_model::Tier>,
    pub skills: Vec<String>,
}

/// What `{{stack}}` reads when no stack is given.
const DEFAULT_STACK: &str = "this project's stack";

/// The agent a canonical template gives for `id`, plus the placeholders left in
/// its body. `description`, `tier` and `skills` are set on the parsed frontmatter
/// (never by substitution in the YAML); `{{name}}`, `{{description}}` and
/// `{{stack}}` are rendered in the body only.
pub fn agent_from_template(
    id: &str,
    template: &str,
    source: &str,
    opts: &AgentOptions,
) -> anyhow::Result<(armadai_model::Agent, Vec<String>)> {
    let Some((yaml, body, _)) = armadai_model::split_frontmatter(source) else {
        anyhow::bail!(
            "template {template} uses the legacy format; rewrite it with a YAML frontmatter \
             (see the built-in templates) or run armadai migrate on a project"
        );
    };
    let mut front: armadai_model::AgentFront = serde_yaml_ng::from_str(yaml)
        .map_err(|e| anyhow::anyhow!("template {template} has an invalid frontmatter: {e}"))?;
    match &opts.description {
        Some(d) => front.description = d.clone(),
        None if front.description.contains("{{") || front.description.trim().is_empty() => {
            anyhow::bail!("template {template} has no default description: pass --description")
        }
        None => {}
    }
    if opts.tier.is_some() {
        front.tier = opts.tier;
    }
    if !opts.skills.is_empty() {
        front.skills = opts.skills.clone();
    }
    let mut vars = std::collections::BTreeMap::new();
    vars.insert("name".to_string(), slug_to_title(id));
    vars.insert("description".to_string(), front.description.clone());
    let stack = opts.stack.as_deref().unwrap_or(DEFAULT_STACK);
    vars.insert("stack".to_string(), stack.to_string());
    let (body, remaining) = armadai_core::template::render_lenient(body, &vars);
    Ok((armadai_model::Agent { front, body }, remaining))
}

/// Convert a slug like "my-agent" to title case "My Agent".
pub fn slug_to_title(slug: &str) -> String {
    slug.split('-')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(c) => format!("{}{}", c.to_uppercase(), chars.as_str()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Clap value parser that provides completion for available template names.
/// The embedded templates are always offered, so the list is never empty.
pub fn template_value_parser() -> clap::builder::ValueParser {
    let paths = armadai_core::config::AppPaths::resolve();
    let names = template_names_from(&paths.templates_dir);
    // Leak strings to get 'static references needed by clap's PossibleValuesParser.
    // This is called once at startup, so the leak is negligible.
    let leaked: Vec<&'static str> = names
        .into_iter()
        .map(|s| &*Box::leak(s.into_boxed_str()))
        .collect();
    clap::builder::PossibleValuesParser::new(leaked).into()
}

/// Collect template names (stems) from the templates directory.
fn collect_template_names(templates_dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(templates_dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "md"))
        .filter_map(|e| {
            e.path()
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
        })
        .collect();
    names.sort();
    names
}

fn list_templates(templates_dir: &Path) -> anyhow::Result<()> {
    let h = crate::cli::style::header();
    let m = crate::cli::style::muted();
    anstream::println!("{h}Available templates:{h:#}");
    for name in template_names_from(templates_dir) {
        let a = crate::cli::style::accent();
        anstream::println!("  - {a}{name}{a:#}");
    }
    anstream::println!(
        "{m}  Built-in templates are overridden by a same-named file in {}{m:#}",
        templates_dir.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn load_template_falls_back_to_embedded_when_dir_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("no-such-dir");
        let body = load_template_from(&missing, "basic").expect("embedded basic");
        assert_eq!(body, embedded_template("basic").unwrap());
        assert!(load_template_from(&missing, "no-such-template").is_none());
    }

    #[test]
    fn load_template_prefers_the_filesystem_over_embedded() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("basic.md"), "FS_WINS").unwrap();
        assert_eq!(load_template_from(dir.path(), "basic").unwrap(), "FS_WINS");
        // other names still come from the embedded set
        assert_eq!(
            load_template_from(dir.path(), "debug").unwrap(),
            embedded_template("debug").unwrap()
        );
    }

    #[test]
    fn template_names_merge_embedded_and_filesystem_sorted_unique() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("basic.md"), "x").unwrap(); // shadows an embedded name
        fs::write(dir.path().join("zzz-custom.md"), "x").unwrap();
        fs::write(dir.path().join("notes.txt"), "x").unwrap(); // not a template
        let names = template_names_from(dir.path());
        assert_eq!(names.iter().filter(|n| *n == "basic").count(), 1);
        assert!(names.contains(&"zzz-custom".to_string()));
        assert!(names.contains(&"tdd-refactor".to_string()));
        assert!(!names.contains(&"notes".to_string()));
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted);
        assert_eq!(names.len(), 12);
    }

    #[test]
    fn embedded_set_matches_the_templates_directory() {
        let on_disk =
            collect_template_names(Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/templates")));
        assert_eq!(template_names_from(Path::new("/nonexistent")), on_disk);
        assert_eq!(on_disk.len(), 11);
    }

    /// A project holding only `agent` as `.armadai/agents/<id>.md`: its diagnostics.
    fn diagnostics_of(id: &str, agent: &armadai_model::Agent) -> Vec<armadai_model::Diagnostic> {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".armadai/agents")).unwrap();
        fs::write(
            dir.path().join(".armadai/project.yaml"),
            "schema_version: 1\n",
        )
        .unwrap();
        fs::write(
            dir.path().join(format!(".armadai/agents/{id}.md")),
            armadai_model::render_agent(id, agent),
        )
        .unwrap();
        armadai_model::validate(dir.path())
    }

    #[test]
    fn every_embedded_template_gives_a_clean_agent_without_options() {
        for name in template_names_from(Path::new("/nonexistent")) {
            let source = embedded_template(&name).unwrap();
            let (agent, left) =
                agent_from_template("my-agent", &name, source, &AgentOptions::default()).unwrap();
            let text = armadai_model::render_agent("my-agent", &agent);
            assert!(!text.contains("{{"), "{name}: {text}");
            assert!(left.is_empty(), "{name}: {left:?}");
            assert!(!agent.front.description.trim().is_empty(), "{name}");
            assert!(!agent.body.contains("## Pipeline"), "{name}");
            assert!(agent.front.tier.is_some(), "{name}");
            let ds = diagnostics_of("my-agent", &agent);
            assert!(ds.is_empty(), "{name}: {ds:?}");
        }
    }

    #[test]
    fn options_are_set_on_the_frontmatter_not_substituted_into_it() {
        // YAML-hostile text: a substitution into the frontmatter would break it
        let hostile = "audits: \"payments\" # and {{stack}}\n- more";
        let opts = AgentOptions {
            description: Some(hostile.into()),
            stack: Some("rust".into()),
            tier: Some(armadai_model::Tier::Max),
            skills: vec!["notes".into()],
        };
        let (agent, _) =
            agent_from_template("a", "basic", embedded_template("basic").unwrap(), &opts).unwrap();
        assert_eq!(agent.front.description, hostile);
        assert_eq!(agent.front.tier, Some(armadai_model::Tier::Max));
        assert_eq!(agent.front.skills, ["notes"]);
        assert!(agent.body.contains(hostile), "{}", agent.body);
        let text = armadai_model::render_agent("a", &agent);
        let (yaml, _, _) = armadai_model::split_frontmatter(&text).unwrap();
        let back: armadai_model::AgentFront = serde_yaml_ng::from_str(yaml).unwrap();
        assert_eq!(back, agent.front);

        let (agent, _) = agent_from_template(
            "a",
            "dev-review",
            embedded_template("dev-review").unwrap(),
            &opts,
        )
        .unwrap();
        assert!(agent.body.contains("working on rust"), "{}", agent.body);
    }

    #[test]
    fn a_legacy_template_is_a_clear_error() {
        let legacy = "# {{name}}\n\n## Metadata\n- provider: anthropic\n\n## System Prompt\n\nHi\n";
        let e = agent_from_template("a", "mine", legacy, &AgentOptions::default()).unwrap_err();
        assert!(
            e.to_string()
                .contains("template mine uses the legacy format"),
            "{e}"
        );
    }

    #[test]
    fn a_template_without_a_default_description_needs_one() {
        let t = "---\ndescription: \"{{description}}\"\n---\nBody {{description}}\n";
        let e = agent_from_template("a", "mine", t, &AgentOptions::default()).unwrap_err();
        assert!(e.to_string().contains("--description"), "{e}");
        let opts = AgentOptions {
            description: Some("Does things".into()),
            ..AgentOptions::default()
        };
        let (agent, _) = agent_from_template("a", "mine", t, &opts).unwrap();
        let text = armadai_model::render_agent("a", &agent);
        assert!(!text.contains("{{description}}"), "{text}");
        assert!(text.contains("Body Does things"), "{text}");
    }

    #[test]
    fn test_slug_to_title() {
        assert_eq!(slug_to_title("my-agent"), "My Agent");
        assert_eq!(slug_to_title("code-reviewer"), "Code Reviewer");
        assert_eq!(slug_to_title("simple"), "Simple");
    }
}
