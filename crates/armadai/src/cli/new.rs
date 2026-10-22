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
pub fn template_names_from(templates_dir: &Path) -> Vec<String> {
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

/// `armadai new <kind>`.
#[derive(clap::Subcommand)]
pub enum NewCommand {
    /// Create an agent from a template (.armadai/agents/<id>.md)
    #[command(after_help = "Examples:\n  \
        armadai new agent reviewer --template dev-review --stack rust\n  \
        armadai new agent scanner --template security-review -d \"audit OWASP top 10\" --tier max\n  \
        armadai new agent guardian --from checker --var subject=\"the frame\"")]
    Agent {
        /// Agent id, its file name: lowercase letters, digits and hyphens (required, unless -i)
        id: Option<String>,
        /// Template to start from
        #[arg(long, short, default_value = "basic", value_parser = template_value_parser())]
        template: String,
        /// Description (replaces the template's; also fills {{description}})
        #[arg(long, short)]
        description: Option<String>,
        /// Tech stack (fills {{stack}})
        #[arg(long, short)]
        stack: Option<String>,
        /// Model tier: fast, pro or max
        #[arg(long, value_parser = parse_tier)]
        tier: Option<armadai_model::Tier>,
        /// Skills of the project the agent uses (comma-separated)
        #[arg(long, value_delimiter = ',')]
        skills: Vec<String>,
        /// A template of the project (.armadai/templates/<id>.md): the agent stays linked to it
        #[arg(long, conflicts_with_all = ["template", "description", "stack", "tier", "skills", "interactive"])]
        from: Option<String>,
        /// A value for the project template, NAME=VALUE (repeatable)
        #[arg(long = "var", value_name = "NAME=VALUE", value_parser = parse_var, requires = "from")]
        vars: Vec<(String, String)>,
        /// Replace an existing agent file
        #[arg(long)]
        force: bool,
        /// Ask for every value in the terminal
        #[arg(long, short = 'i')]
        interactive: bool,
    },
    /// Create a flow from a template over agents of the project (.armadai/flows/<id>.yaml)
    #[command(
        long_about = "Create a flow from a template over agents of the project \
            (.armadai/flows/<id>.yaml); step ids are the agent ids.\n\n\
            Templates (agents in the order given):\n  \
            pipeline           a -> b -> c ... (2 agents or more)\n  \
            review-loop        author -> reviewer; `rejected` goes back to the author, \
            `approved` on to the optional third agent (2 or 3 agents)\n  \
            plan-build-verify  planner -> builder -> verifier; a failed verification goes back \
            to the builder (3 agents)\n  \
            fan-out            lead -> each other agent: a lead handing work to its \
            specialists (2 agents or more)",
        after_help = "Examples:\n  \
            armadai new flow ship --agents planner,dev,qa\n  \
            armadai new flow review --template review-loop --agents dev,reviewer\n  \
            armadai graph review"
    )]
    Flow {
        /// Flow id, its file name (required, unless -i)
        id: Option<String>,
        /// Flow template: pipeline, review-loop, plan-build-verify or fan-out
        #[arg(long, short, default_value = "pipeline", value_parser = parse_flow_template)]
        template: armadai_model::FlowTemplate,
        /// Agents of the project, in order, comma-separated (required, unless -i)
        #[arg(long, value_delimiter = ',')]
        agents: Vec<String>,
        /// Description of the flow
        #[arg(long, short)]
        description: Option<String>,
        /// Replace an existing flow file
        #[arg(long)]
        force: bool,
        /// Ask for every value in the terminal
        #[arg(long, short = 'i')]
        interactive: bool,
    },
    /// Create an automation: a command run on an event (.armadai/automations/<id>.yaml)
    #[command(after_help = "Examples:\n  \
        armadai new automation fmt --event after-tool --run \"cargo fmt\"")]
    Automation {
        /// Automation id, its file name (required, unless -i)
        id: Option<String>,
        /// session-start, session-end, before-tool, after-tool, on-prompt or on-stop (required, unless -i)
        #[arg(long, value_parser = parse_event)]
        event: Option<armadai_model::Event>,
        /// Shell command to run (required, unless -i)
        #[arg(long)]
        run: Option<String>,
        /// Description of the automation
        #[arg(long, short)]
        description: Option<String>,
        /// Replace an existing automation file
        #[arg(long)]
        force: bool,
        /// Ask for every value in the terminal
        #[arg(long, short = 'i')]
        interactive: bool,
    },
    /// Create a skill skeleton (.armadai/skills/<id>/SKILL.md)
    #[command(after_help = "Examples:\n  \
        armadai new skill release-notes --description \"Write release notes from the git log\"")]
    Skill {
        /// Skill id, its folder name (required, unless -i)
        id: Option<String>,
        /// What the skill does and when to use it (required, unless -i)
        #[arg(long, short)]
        description: Option<String>,
        /// Replace an existing SKILL.md
        #[arg(long)]
        force: bool,
        /// Ask for every value in the terminal
        #[arg(long, short = 'i')]
        interactive: bool,
    },
    /// The former `armadai new <name>`
    #[command(external_subcommand)]
    Former(Vec<String>),
}

fn parse_tier(s: &str) -> Result<armadai_model::Tier, String> {
    armadai_model::yaml::from_str(s).map_err(|_| format!("`{s}` is not a tier (fast, pro, max)"))
}

fn parse_var(s: &str) -> Result<(String, String), String> {
    match s.split_once('=') {
        Some((k, v)) if !k.is_empty() => Ok((k.to_string(), v.to_string())),
        _ => Err(format!(
            "`{s}` is not NAME=VALUE (e.g. --var subject=\"the frame\")"
        )),
    }
}

fn parse_flow_template(s: &str) -> Result<armadai_model::FlowTemplate, String> {
    s.parse()
        .map_err(|e: armadai_model::TemplateError| e.to_string())
}

fn parse_event(s: &str) -> Result<armadai_model::Event, String> {
    armadai_model::yaml::from_str(s).map_err(|_| {
        format!(
            "`{s}` is not an event (session-start, session-end, before-tool, after-tool, \
             on-prompt, on-stop)"
        )
    })
}

/// `v`, or the error `message` (which says how to give it).
fn required<T>(v: Option<T>, message: &str) -> anyhow::Result<T> {
    v.ok_or_else(|| anyhow::anyhow!("{message}"))
}

const AUTOMATION: &str =
    "{} is required: armadai new automation <id> --event <event> --run <command>";

pub fn execute(command: Option<NewCommand>) -> anyhow::Result<()> {
    use crate::cli::new_target::Kind;
    use crate::cli::new_wizard::{self, WizardKind};
    let Some(command) = command else {
        return new_wizard::run(None);
    };
    match command {
        NewCommand::Agent {
            interactive: true, ..
        } => new_wizard::run(Some(WizardKind::Agent)),
        NewCommand::Flow {
            interactive: true, ..
        } => new_wizard::run(Some(WizardKind::Flow)),
        NewCommand::Automation {
            interactive: true, ..
        } => new_wizard::run(Some(WizardKind::Automation)),
        NewCommand::Skill {
            interactive: true, ..
        } => new_wizard::run(Some(WizardKind::Skill)),
        NewCommand::Agent {
            id,
            from: Some(from),
            vars,
            force,
            ..
        } => create_instance(
            &required(
                id,
                "an agent id is required: armadai new agent <id> --from <template> --var name=value",
            )?,
            &from,
            var_map(vars)?,
            force,
        ),
        NewCommand::Agent {
            id,
            template,
            description,
            stack,
            tier,
            skills,
            force,
            ..
        } => {
            let opts = AgentOptions {
                description,
                stack,
                tier,
                skills,
                ..AgentOptions::default()
            };
            create_agent(
                &required(
                    id,
                    "an agent id is required: armadai new agent <id> [--template <template>]",
                )?,
                &template,
                &opts,
                force,
            )
        }
        NewCommand::Flow {
            id,
            template,
            agents,
            description,
            force,
            ..
        } => {
            let id = required(
                id,
                "a flow id is required: armadai new flow <id> --agents a,b",
            )?;
            crate::cli::new_target::check_id(&id)?;
            if agents.is_empty() {
                anyhow::bail!(
                    "--agents is required: armadai new flow <id> --agents a,b (agents of the \
                     project, in order)"
                );
            }
            // an agent the project lacks is the validator's `unknown-agent`: undone (T8)
            let flow = armadai_model::flow_from_template(template, &agents, description)?;
            create_flow(&id, &flow, force)
        }
        NewCommand::Automation {
            id,
            event,
            run,
            description,
            force,
            ..
        } => {
            let id = required(id, AUTOMATION.replace("{}", "an automation id").as_str())?;
            let a = armadai_model::Automation {
                event: required(event, AUTOMATION.replace("{}", "--event").as_str())?,
                run: required(run, AUTOMATION.replace("{}", "--run").as_str())?,
                description,
                native: Default::default(),
            };
            create(
                Kind::Automation,
                &id,
                force,
                &armadai_model::render_automation(&a),
            )
        }
        NewCommand::Skill {
            id,
            description,
            force,
            ..
        } => {
            let id = required(
                id,
                "a skill id is required: armadai new skill <id> --description <text>",
            )?;
            let skill = skill_skeleton(
                &id,
                required(
                    description,
                    "--description is required: armadai new skill <id> --description <text>",
                )?,
            );
            create(
                Kind::Skill,
                &id,
                force,
                &armadai_model::render_skill(&skill),
            )
        }
        NewCommand::Former(args) => {
            let name = args.first().map(String::as_str).unwrap_or("<id>");
            anyhow::bail!(
                "`armadai new <name>` became `armadai new agent <name>`: run \
                 `armadai new agent {name}` (or `armadai new --help` for flows, \
                 automations and skills)"
            )
        }
    }
}

/// The agent `id` from `template` with `opts`, written and validated.
pub fn create_agent(
    id: &str,
    template: &str,
    opts: &AgentOptions,
    force: bool,
) -> anyhow::Result<()> {
    crate::cli::new_target::check_id(id)?;
    let paths = AppPaths::resolve();
    let source = load_template_from(&paths.templates_dir, template)
        .ok_or_else(|| anyhow::anyhow!("template `{template}` not found"))?;
    let (agent, remaining) = agent_from_template(id, template, &source, opts)?;
    create(
        crate::cli::new_target::Kind::Agent,
        id,
        force,
        &armadai_model::render_agent(id, &agent),
    )?;
    warn_placeholders(&remaining);
    Ok(())
}

/// `--var` values by name; a name given twice is refused (which value was meant?).
fn var_map(
    vars: Vec<(String, String)>,
) -> anyhow::Result<std::collections::BTreeMap<String, String>> {
    let mut map = std::collections::BTreeMap::new();
    for (k, v) in vars {
        if map.contains_key(&k) {
            anyhow::bail!("--var `{k}` given twice; keep one value");
        }
        map.insert(k, v);
    }
    Ok(map)
}

/// The agent `id` as an instance of the project template `from`, written and
/// validated: an unknown template or a missing variable is the validator's, undone.
pub fn create_instance(
    id: &str,
    from: &str,
    vars: std::collections::BTreeMap<String, String>,
    force: bool,
) -> anyhow::Result<()> {
    let agent = instance_agent(from, vars);
    create(
        crate::cli::new_target::Kind::Agent,
        id,
        force,
        &armadai_model::render_agent(id, &agent),
    )
}

/// An agent holding only its instance form (`from` + `vars`); the writer ignores
/// `front` and `body` for it.
pub fn instance_agent(
    from: &str,
    vars: std::collections::BTreeMap<String, String>,
) -> armadai_model::Agent {
    armadai_model::Agent {
        front: armadai_model::AgentFront::default(),
        body: String::new(),
        instance: Some(armadai_model::Instance {
            from: from.to_string(),
            vars,
            front: Default::default(),
            body: String::new(),
        }),
    }
}

pub fn create_flow(id: &str, flow: &armadai_model::Flow, force: bool) -> anyhow::Result<()> {
    create(
        crate::cli::new_target::Kind::Flow,
        id,
        force,
        &armadai_model::render_flow(flow),
    )?;
    let root = crate::compile::find_project(&std::env::current_dir()?)?;
    let manifest = armadai_model::load_project(&root)
        .project
        .map(|p| p.manifest);
    if manifest.is_some_and(|m| {
        m.coordinator.is_none() && m.policy.delegation != armadai_model::Delegation::Strict
    }) {
        let w = crate::cli::style::warn();
        anstream::eprintln!(
            "{w}warning: this flow is guidance only: link renders it nowhere until you set a \
             coordinator or policy strict (.armadai/project.yaml){w:#}"
        );
    }
    let m = crate::cli::style::muted();
    anstream::println!("{m}  See it: armadai graph {id}{m:#}");
    Ok(())
}

pub fn skill_skeleton(id: &str, description: String) -> armadai_model::Skill {
    armadai_model::Skill {
        front: armadai_model::SkillFront {
            name: Some(id.to_string()),
            description: Some(description),
            ..Default::default()
        },
        body: format!(
            "# {}\n\nWhen to use this skill, then the steps to follow.\n",
            slug_to_title(id)
        ),
    }
}

/// Write one file through the guard, then validate the project: a creation that
/// introduces an error (a `(code, file)` absent before) is undone (T8).
pub fn create(
    kind: crate::cli::new_target::Kind,
    id: &str,
    force: bool,
    content: &str,
) -> anyhow::Result<()> {
    let mut target = crate::cli::new_target::NewTarget::resolve(kind, id, force)?;
    let key = |d: &armadai_model::Diagnostic| (d.code, d.file.clone());
    let before: std::collections::BTreeSet<_> = (armadai_model::validate(&target.root).iter())
        .map(key)
        .collect();
    target.write(content)?;
    let introduced: Vec<armadai_model::Diagnostic> = armadai_model::validate(&target.root)
        .into_iter()
        .filter(|d| !before.contains(&key(d)))
        .collect();
    let w = crate::cli::style::warn();
    let display = target.display.clone();
    let rolled_back = armadai_model::has_errors(&introduced);
    for d in &introduced {
        // the file a rolled-back diagnostic points at is gone: say so
        let tag = if rolled_back { "(rolled back) " } else { "" };
        anstream::eprintln!("{w}{tag}{}{w:#}", d.render());
    }
    if rolled_back {
        target.undo()?;
        anyhow::bail!(
            "{} would make the project invalid; nothing was written",
            display.display()
        );
    }
    target.commit()?;
    let o = crate::cli::style::ok();
    anstream::println!("{o}Created{o:#} {}", display.display());
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

/// What the user sets on top of a template; `None` keeps the template's value.
#[derive(Debug, Default, Clone)]
pub struct AgentOptions {
    pub description: Option<String>,
    pub stack: Option<String>,
    pub tier: Option<armadai_model::Tier>,
    pub skills: Vec<String>,
    pub capabilities: armadai_model::Capabilities,
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
    let mut front: armadai_model::AgentFront = armadai_model::yaml::from_str(yaml)
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
    if opts.capabilities != armadai_model::Capabilities::default() {
        front.capabilities = opts.capabilities.clone();
    }
    let mut vars = std::collections::BTreeMap::new();
    vars.insert("name".to_string(), slug_to_title(id));
    vars.insert("description".to_string(), front.description.clone());
    let stack = opts.stack.as_deref().unwrap_or(DEFAULT_STACK);
    vars.insert("stack".to_string(), stack.to_string());
    let (body, remaining) = armadai_core::template::render_lenient(body, &vars);
    Ok((
        armadai_model::Agent {
            front,
            body,
            instance: None,
        },
        remaining,
    ))
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
            ..AgentOptions::default()
        };
        let (agent, _) =
            agent_from_template("a", "basic", embedded_template("basic").unwrap(), &opts).unwrap();
        assert_eq!(agent.front.description, hostile);
        assert_eq!(agent.front.tier, Some(armadai_model::Tier::Max));
        assert_eq!(agent.front.skills, ["notes"]);
        assert!(agent.body.contains(hostile), "{}", agent.body);
        let text = armadai_model::render_agent("a", &agent);
        let (yaml, _, _) = armadai_model::split_frontmatter(&text).unwrap();
        let back: armadai_model::AgentFront = armadai_model::yaml::from_str(yaml).unwrap();
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
