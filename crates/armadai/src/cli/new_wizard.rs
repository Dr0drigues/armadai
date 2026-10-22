//! `armadai new` (no kind) and `armadai new <kind> -i`: questions in the terminal.
//!
//! Every decision lives in the pure `plan_*` functions (answers → what is written),
//! tested without a terminal; the `dialoguer` layer only collects the answers. The
//! write itself is the non-interactive path (guard, validation, undo).

use std::io::IsTerminal;

use armadai_model::{Automation, Cap, Capabilities, Event, Flow, FlowTemplate, Tier};

use crate::cli::new::AgentOptions;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WizardKind {
    Agent,
    Flow,
    Automation,
    Skill,
    Command,
}

/// The guided menu of `armadai new`, in the order shown.
pub const MENU: [(WizardKind, &str); 5] = [
    (WizardKind::Agent, "an agent"),
    (WizardKind::Flow, "a flow"),
    (WizardKind::Automation, "an automation"),
    (WizardKind::Skill, "a skill"),
    (WizardKind::Command, "a command"),
];

/// The five capabilities, in the order they are asked.
pub const CAPABILITIES: [&str; 5] = ["read", "write", "shell", "web", "delegate"];

#[derive(Debug, Clone, Default)]
pub struct AgentAnswers {
    pub id: String,
    pub template: String,
    /// Empty: the template's description.
    pub description: String,
    /// Empty: no stack.
    pub stack: String,
    /// `None`: the template's tier.
    pub tier: Option<Tier>,
    pub skills: Vec<String>,
    /// In [`CAPABILITIES`] order; `None` leaves the target's default.
    pub capabilities: [Option<Cap>; 5],
}

/// `(id, template, options)` for `new::create_agent`.
pub fn plan_agent(a: AgentAnswers) -> (String, String, AgentOptions) {
    let text = |s: String| Some(s.trim().to_string()).filter(|s| !s.is_empty());
    let [read, write, shell, web, delegate] = a.capabilities;
    let opts = AgentOptions {
        description: text(a.description),
        stack: text(a.stack),
        tier: a.tier,
        skills: a.skills,
        capabilities: Capabilities {
            read,
            write,
            shell,
            web,
            delegate,
        },
    };
    (a.id, a.template, opts)
}

/// Where a new agent starts: a template of the project is offered only when the
/// project has one.
pub fn agent_sources(project_templates: usize) -> &'static [&'static str] {
    if project_templates == 0 {
        &["a built-in template"]
    } else {
        &["a built-in template", "a template of the project"]
    }
}

/// Every `{{var}}` of a project template, its frontmatter values and its body.
pub fn template_variables(t: &armadai_model::Template) -> std::collections::BTreeSet<String> {
    let mut names = armadai_model::agent_templates::variables(&t.body);
    let front = serde_json::to_value(&t.front).unwrap_or_default();
    armadai_model::agent_templates::value_variables(&front, &mut names);
    names
}

#[derive(Debug, Clone)]
pub struct InstanceAnswers {
    pub id: String,
    pub from: String,
    /// `(variable, value)`, values as typed.
    pub vars: Vec<(String, String)>,
}

/// `(id, from, vars)` for `new::create_instance`.
pub fn plan_instance(
    a: InstanceAnswers,
) -> (String, String, std::collections::BTreeMap<String, String>) {
    let vars = (a.vars.into_iter())
        .map(|(k, v)| (k, v.trim().to_string()))
        .collect();
    (a.id, a.from, vars)
}

#[derive(Debug, Clone)]
pub struct FlowAnswers {
    pub id: String,
    pub template: FlowTemplate,
    /// In the order the flow runs them.
    pub agents: Vec<String>,
    /// Empty: none.
    pub description: String,
}

/// `(id, flow)`; the Mermaid preview is `flow_to_mermaid` of it.
pub fn plan_flow(a: FlowAnswers) -> Result<(String, Flow), armadai_model::TemplateError> {
    let description = Some(a.description.trim().to_string()).filter(|s| !s.is_empty());
    let flow = armadai_model::flow_from_template(a.template, &a.agents, description)?;
    Ok((a.id, flow))
}

#[derive(Debug, Clone)]
pub struct AutomationAnswers {
    pub id: String,
    pub event: Event,
    pub run: String,
    pub description: String,
}

pub fn plan_automation(a: AutomationAnswers) -> (String, Automation) {
    let description = Some(a.description.trim().to_string()).filter(|s| !s.is_empty());
    let automation = Automation {
        event: a.event,
        run: a.run.trim().to_string(),
        description,
        native: Default::default(),
    };
    (a.id, automation)
}

#[derive(Debug, Clone)]
pub struct CommandAnswers {
    pub id: String,
    pub description: String,
}

pub fn plan_command(a: CommandAnswers) -> (String, armadai_model::Command) {
    let command = crate::cli::new::command_skeleton(a.description.trim().to_string());
    (a.id, command)
}

/// The valid id an invalid answer most likely meant (`Mon Agent` → `mon-agent`),
/// to propose, never to apply silently.
pub fn suggest_id(input: &str) -> Option<String> {
    let mut out = String::new();
    for c in input.trim().chars().flat_map(char::to_lowercase) {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let out: String = out.trim_matches('-').chars().take(64).collect();
    let out = out.trim_end_matches('-').to_string();
    (out != input && crate::cli::new_target::check_id(&out).is_ok()).then_some(out)
}

fn require_terminal() -> anyhow::Result<()> {
    if !(std::io::stdin().is_terminal() && std::io::stderr().is_terminal()) {
        anyhow::bail!(
            "interactive mode needs a terminal; pass the options instead \
             (armadai new <agent|flow|automation|skill|command> --help)"
        );
    }
    Ok(())
}

pub fn run(kind: Option<WizardKind>) -> anyhow::Result<()> {
    require_terminal()?;
    let kind = match kind {
        Some(k) => k,
        None => {
            let i = dialoguer::Select::new()
                .with_prompt("Create")
                .items(MENU.map(|(_, label)| label))
                .default(0)
                .interact()?;
            MENU[i].0
        }
    };
    let root = crate::compile::find_project(&std::env::current_dir()?)?;
    let project = armadai_model::load_project(&root).project;
    match kind {
        WizardKind::Agent => {
            let templates = project.as_ref().map(|p| &p.templates);
            let sources = agent_sources(templates.map_or(0, |t| t.len()));
            if let Some(templates) = templates
                && sources.len() > 1
                && dialoguer::Select::new()
                    .with_prompt("Start from")
                    .items(sources)
                    .default(0)
                    .interact()?
                    == 1
            {
                let (id, from, vars) = plan_instance(ask_instance(templates)?);
                return crate::cli::new::create_instance(&id, &from, vars, false);
            }
            let skills: Vec<String> = (project.iter())
                .flat_map(|p| p.skills.keys().cloned())
                .collect();
            let (id, template, opts) = plan_agent(ask_agent(&skills)?);
            crate::cli::new::create_agent(&id, &template, &opts, false)
        }
        WizardKind::Flow => {
            let agents: Vec<String> = (project.iter())
                .flat_map(|p| p.agents.keys().cloned())
                .collect();
            if agents.len() < 2 {
                anyhow::bail!(
                    "a flow needs at least 2 agents; create them first (armadai new agent)"
                );
            }
            let (id, flow) = plan_flow(ask_flow(&agents)?)?;
            eprintln!("\n{}", armadai_model::flow_to_mermaid(&id, &flow));
            if !dialoguer::Confirm::new()
                .with_prompt("Write this flow?")
                .default(true)
                .interact()?
            {
                anyhow::bail!("nothing was written");
            }
            crate::cli::new::create_flow(&id, &flow, false)
        }
        WizardKind::Automation => {
            let (id, a) = plan_automation(ask_automation()?);
            crate::cli::new::create(
                crate::cli::new_target::Kind::Automation,
                &id,
                false,
                &armadai_model::render_automation(&a),
            )
        }
        WizardKind::Skill => {
            let id = ask_id("Skill id")?;
            let description: String = dialoguer::Input::new()
                .with_prompt("What it does and when to use it")
                .interact_text()?;
            let skill = crate::cli::new::skill_skeleton(&id, description);
            crate::cli::new::create(
                crate::cli::new_target::Kind::Skill,
                &id,
                false,
                &armadai_model::render_skill(&skill),
            )
        }
        WizardKind::Command => {
            let id = ask_id("Command id, typed as /<id> (e.g. brief)")?;
            let description: String = dialoguer::Input::new()
                .with_prompt("What it does (shown in the / menu)")
                .interact_text()?;
            let (id, command) = plan_command(CommandAnswers { id, description });
            crate::cli::new::create(
                crate::cli::new_target::Kind::Command,
                &id,
                false,
                &armadai_model::render_command(&command),
            )
        }
    }
}

fn ask_id(prompt: &str) -> anyhow::Result<String> {
    Ok(dialoguer::Input::<String>::new()
        .with_prompt(prompt)
        .validate_with(|s: &String| -> Result<(), String> {
            crate::cli::new_target::check_id(s).map_err(|e| match suggest_id(s) {
                Some(ok) => format!("{e}; try `{ok}`"),
                None => e.to_string(),
            })
        })
        .interact_text()?)
}

fn ask_text(prompt: &str) -> anyhow::Result<String> {
    Ok(dialoguer::Input::<String>::new()
        .with_prompt(prompt)
        .allow_empty(true)
        .interact_text()?)
}

fn ask_agent(skills: &[String]) -> anyhow::Result<AgentAnswers> {
    let id = ask_id("Agent id (e.g. code-reviewer)")?;
    let templates = crate::cli::new::template_names_from(
        &armadai_core::config::AppPaths::resolve().templates_dir,
    );
    let start = templates.iter().position(|t| t == "basic").unwrap_or(0);
    let t = dialoguer::Select::new()
        .with_prompt("Template")
        .items(&templates)
        .default(start)
        .interact()?;
    let description = ask_text("Description (empty: the template's)")?;
    let stack = ask_text("Tech stack (empty: none)")?;
    let tiers = [None, Some(Tier::Fast), Some(Tier::Pro), Some(Tier::Max)];
    let tier = tiers[dialoguer::Select::new()
        .with_prompt("Model tier")
        .items(["the template's", "fast", "pro", "max"])
        .default(0)
        .interact()?];
    let skills = if skills.is_empty() {
        Vec::new()
    } else {
        dialoguer::MultiSelect::new()
            .with_prompt("Skills (space to pick, enter to go on)")
            .items(skills)
            .interact()?
            .into_iter()
            .map(|i| skills[i].clone())
            .collect()
    };
    let mut capabilities = [None; 5];
    if dialoguer::Confirm::new()
        .with_prompt("Set capabilities (read, write, shell, web, delegate)?")
        .default(false)
        .interact()?
    {
        let caps = [None, Some(Cap::Allow), Some(Cap::Ask), Some(Cap::Deny)];
        for (slot, name) in capabilities.iter_mut().zip(CAPABILITIES) {
            *slot = caps[dialoguer::Select::new()
                .with_prompt(name)
                .items(["leave the default", "allow", "ask", "deny"])
                .default(0)
                .interact()?];
        }
    }
    Ok(AgentAnswers {
        id,
        template: templates[t].clone(),
        description,
        stack,
        tier,
        skills,
        capabilities,
    })
}

fn ask_instance(
    templates: &std::collections::BTreeMap<String, armadai_model::Template>,
) -> anyhow::Result<InstanceAnswers> {
    let id = ask_id("Agent id (e.g. guardian)")?;
    let ids: Vec<&String> = templates.keys().collect();
    let from = ids[dialoguer::Select::new()
        .with_prompt("Template of the project")
        .items(&ids)
        .default(0)
        .interact()?]
    .clone();
    let mut vars = Vec::new();
    for name in template_variables(&templates[&from]) {
        let value: String = dialoguer::Input::new()
            .with_prompt(format!("{{{{{name}}}}}"))
            .interact_text()?;
        vars.push((name, value));
    }
    Ok(InstanceAnswers { id, from, vars })
}

fn ask_flow(agents: &[String]) -> anyhow::Result<FlowAnswers> {
    let id = ask_id("Flow id (e.g. review)")?;
    let template = armadai_model::FLOW_TEMPLATES[dialoguer::Select::new()
        .with_prompt("Template")
        .items([
            "pipeline: a -> b -> c ... (2 agents or more)",
            "review-loop: author -> reviewer, rejected back, approved on (2 or 3 agents)",
            "plan-build-verify: planner -> builder -> verifier, failure back (3 agents)",
            "fan-out: a lead -> each other agent (2 agents or more)",
        ])
        .default(0)
        .interact()?];
    let picked: Vec<String> = dialoguer::MultiSelect::new()
        .with_prompt("Agents (space to pick, enter to go on)")
        .items(agents)
        .interact()?
        .into_iter()
        .map(|i| agents[i].clone())
        .collect();
    let agents = if picked.len() > 1 {
        dialoguer::Sort::new()
            .with_prompt("Order them (space to grab, arrows to move, enter to go on)")
            .items(&picked)
            .interact()?
            .into_iter()
            .map(|i| picked[i].clone())
            .collect()
    } else {
        picked
    };
    let description = ask_text("Description (empty: none)")?;
    Ok(FlowAnswers {
        id,
        template,
        agents,
        description,
    })
}

fn ask_automation() -> anyhow::Result<AutomationAnswers> {
    let id = ask_id("Automation id (e.g. fmt)")?;
    let events = [
        Event::SessionStart,
        Event::SessionEnd,
        Event::BeforeTool,
        Event::AfterTool,
        Event::OnPrompt,
        Event::OnStop,
    ];
    let event = events[dialoguer::Select::new()
        .with_prompt("Event")
        .items([
            "session-start",
            "session-end",
            "before-tool",
            "after-tool",
            "on-prompt",
            "on-stop",
        ])
        .default(3)
        .interact()?];
    let run: String = dialoguer::Input::new()
        .with_prompt("Command to run")
        .interact_text()?;
    let description = ask_text("Description (empty: none)")?;
    Ok(AutomationAnswers {
        id,
        event,
        run,
        description,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_answers_keep_the_template_defaults() {
        let (id, template, opts) = plan_agent(AgentAnswers {
            id: "rev".into(),
            template: "dev-review".into(),
            description: "  ".into(),
            ..AgentAnswers::default()
        });
        assert_eq!((id.as_str(), template.as_str()), ("rev", "dev-review"));
        assert_eq!(opts.description, None);
        assert_eq!(opts.stack, None);
        assert_eq!(opts.tier, None);
        assert_eq!(opts.capabilities, Capabilities::default());
    }

    /// Answers become the agent file; capabilities left to the default stay absent.
    #[test]
    fn agent_answers_become_the_file() {
        let (id, template, opts) = plan_agent(AgentAnswers {
            id: "rev".into(),
            template: "dev-review".into(),
            description: " Reviews Rust ".into(),
            stack: "rust".into(),
            tier: Some(Tier::Max),
            skills: vec!["notes".into()],
            capabilities: [
                Some(Cap::Allow),
                None,
                Some(Cap::Deny),
                None,
                Some(Cap::Ask),
            ],
        });
        let source = include_str!("../../templates/dev-review.md");
        let (agent, _) =
            crate::cli::new::agent_from_template(&id, &template, source, &opts).unwrap();
        let text = armadai_model::render_agent(&id, &agent);
        let front = text.split("---\n").nth(1).unwrap();
        assert_eq!(
            front,
            "description: Reviews Rust\ntier: max\ntemperature: 0.3\ntags:\n- dev\n- review\n\
             - quality\nskills:\n- notes\ncapabilities:\n  read: allow\n  shell: deny\n  \
             delegate: ask\n"
        );
        assert!(text.contains("working on rust"), "{text}");
    }

    #[test]
    fn a_project_template_is_offered_only_when_there_is_one() {
        assert_eq!(agent_sources(0), ["a built-in template"]);
        assert_eq!(
            agent_sources(2),
            ["a built-in template", "a template of the project"]
        );
    }

    #[test]
    fn instance_answers_become_the_instance_file() {
        let t: armadai_model::Template = armadai_model::Template {
            front: armadai_model::yaml::from_str(
                "description: \"Checks {{subject}}\"\nnative: { claude: { x: \"{{deep}}\" } }\n",
            )
            .unwrap(),
            body: "You check {{subject}} for {{ who }}, not \\{{literal}}.\n".into(),
        };
        let names: Vec<String> = template_variables(&t).into_iter().collect();
        assert_eq!(names, ["deep", "subject", "who"]);
        let (id, from, vars) = plan_instance(InstanceAnswers {
            id: "guardian".into(),
            from: "checker".into(),
            vars: vec![
                ("subject".into(), " the frame ".into()),
                ("who".into(), "dev".into()),
            ],
        });
        assert_eq!(id, "guardian");
        let agent = crate::cli::new::instance_agent(&from, vars);
        assert_eq!(
            armadai_model::render_agent(&id, &agent),
            "---\nfrom: checker\nvars:\n  subject: the frame\n  who: dev\n---\n"
        );
    }

    #[test]
    fn flow_answers_keep_the_agent_order() {
        let (id, flow) = plan_flow(FlowAnswers {
            id: "ship".into(),
            template: FlowTemplate::Pipeline,
            agents: vec!["qa".into(), "dev".into(), "lead".into()],
            description: "".into(),
        })
        .unwrap();
        assert_eq!(id, "ship");
        assert_eq!(flow.description, None);
        let order: Vec<(&str, &str)> = (flow.links.iter())
            .map(|l| (l.from.as_str(), l.to.as_str()))
            .collect();
        assert_eq!(order, [("qa", "dev"), ("dev", "lead")]);
        let err = plan_flow(FlowAnswers {
            id: "x".into(),
            template: FlowTemplate::PlanBuildVerify,
            agents: vec!["a".into(), "b".into()],
            description: "d".into(),
        })
        .unwrap_err();
        assert!(err.to_string().contains("at least 3"), "{err}");
    }

    #[test]
    fn automation_answers() {
        let (id, a) = plan_automation(AutomationAnswers {
            id: "fmt".into(),
            event: Event::AfterTool,
            run: " cargo fmt ".into(),
            description: "".into(),
        });
        assert_eq!(id, "fmt");
        assert_eq!(a.run, "cargo fmt");
        assert_eq!(a.description, None);
        assert_eq!(
            armadai_model::render_automation(&a),
            "event: after-tool\nrun: cargo fmt\n"
        );
    }

    #[test]
    fn command_answers_become_the_command_file() {
        let (id, c) = plan_command(CommandAnswers {
            id: "brief".into(),
            description: " Refine an idea ".into(),
        });
        assert_eq!(id, "brief");
        assert_eq!(
            armadai_model::render_command(&c),
            "---\ndescription: Refine an idea\n---\n$ARGUMENTS\n"
        );
    }

    #[test]
    fn the_guided_menu_offers_every_kind() {
        assert_eq!(
            MENU,
            [
                (WizardKind::Agent, "an agent"),
                (WizardKind::Flow, "a flow"),
                (WizardKind::Automation, "an automation"),
                (WizardKind::Skill, "a skill"),
                (WizardKind::Command, "a command"),
            ]
        );
    }

    #[test]
    fn an_invalid_id_gets_a_proposal() {
        assert_eq!(suggest_id("Mon Agent").as_deref(), Some("mon-agent"));
        assert_eq!(
            suggest_id("  Code_Reviewer!! ").as_deref(),
            Some("code-reviewer")
        );
        assert_eq!(suggest_id("café bot").as_deref(), Some("caf-bot"));
        assert_eq!(suggest_id("../x").as_deref(), Some("x"));
        assert_eq!(suggest_id("ok"), None, "already valid");
        assert_eq!(suggest_id("!!!"), None);
        assert_eq!(suggest_id("CON"), None, "reserved");
        assert_eq!(suggest_id(&"A".repeat(80)).map(|s| s.len()), Some(64));
    }
}
