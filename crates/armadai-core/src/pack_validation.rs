//! Pack and project config validation.
//!
//! This module provides linting for:
//! - Starter pack `pack.yaml` files
//! - Project config files (`armadai.yaml` / `.armadai/config.yaml`)

use std::collections::HashSet;
use std::path::Path;

use super::orchestration::OrchestrationConfig;
use super::project::ProjectConfig;
use super::prompt::Prompt;
use super::starter::StarterPack;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Severity level of a validation issue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

/// A validation issue found during linting.
#[derive(Debug, Clone)]
pub struct ValidationIssue {
    pub severity: Severity,
    pub location: String,
    pub message: String,
}

impl ValidationIssue {
    fn error(location: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            location: location.into(),
            message: message.into(),
        }
    }

    fn warning(location: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            location: location.into(),
            message: message.into(),
        }
    }
}

/// Validate a starter pack directory (pack.yaml + bundled agents/prompts/skills).
///
/// # Arguments
/// * `pack_root` - Path to the starter pack directory containing `pack.yaml`
///
/// # Returns
/// List of validation issues (empty = valid)
pub fn validate_pack(pack_root: &Path) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();

    // Load pack.yaml
    let pack = match StarterPack::load(pack_root) {
        Ok(p) => p,
        Err(e) => {
            issues.push(ValidationIssue::error(
                "pack.yaml",
                format!("Failed to load pack.yaml: {e}"),
            ));
            return issues;
        }
    };

    // Build a set of all agent names for cross-reference validation
    let agent_names: HashSet<String> = pack.agents.iter().cloned().collect();

    // R1: Validate agent references exist as files
    let agents_dir = pack_root.join("agents");
    for agent_name in &pack.agents {
        let filename = if agent_name.ends_with(".md") {
            agent_name.clone()
        } else {
            format!("{agent_name}.md")
        };
        let agent_path = agents_dir.join(&filename);
        if !agent_path.is_file() {
            issues.push(ValidationIssue::error(
                format!("pack.yaml:agents['{agent_name}']"),
                format!("Agent file not found: {}", agent_path.display()),
            ));
        }
    }

    // R3: Validate prompt references exist as files
    let prompts_dir = pack_root.join("prompts");
    for prompt_name in &pack.prompts {
        let filename = if prompt_name.ends_with(".md") {
            prompt_name.clone()
        } else {
            format!("{prompt_name}.md")
        };
        let prompt_path = prompts_dir.join(&filename);
        if !prompt_path.is_file() {
            issues.push(ValidationIssue::error(
                format!("pack.yaml:prompts['{prompt_name}']"),
                format!("Prompt file not found: {}", prompt_path.display()),
            ));
        }
    }

    // R4: Validate skill references exist as directories with SKILL.md
    let skills_dir = pack_root.join("skills");
    for skill_name in &pack.skills {
        let skill_dir = skills_dir.join(skill_name);
        let skill_md = skill_dir.join("SKILL.md");
        if !skill_md.is_file() {
            // Skills not bundled are OK (might be built-in)
            continue;
        }
    }

    // R5: Validate prompt apply_to targets exist in agents list
    if prompts_dir.is_dir() {
        for entry in std::fs::read_dir(&prompts_dir)
            .into_iter()
            .flatten()
            .flatten()
        {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "md") {
                match Prompt::load(&path) {
                    Ok(prompt) => {
                        for target in &prompt.apply_to {
                            if target == "*" {
                                continue;
                            }
                            // Strip .md suffix if present for comparison
                            let target_name = target.strip_suffix(".md").unwrap_or(target);
                            if !agent_names.contains(target_name) {
                                issues.push(ValidationIssue::error(
                                    format!(
                                        "prompts/{}:apply_to['{target}']",
                                        path.file_name().unwrap().to_string_lossy()
                                    ),
                                    format!(
                                        "Target agent '{target}' not found in pack.yaml agents list"
                                    ),
                                ));
                            }
                        }
                    }
                    Err(e) => {
                        issues.push(ValidationIssue::warning(
                            format!("prompts/{}", path.file_name().unwrap().to_string_lossy()),
                            format!("Failed to parse prompt: {e}"),
                        ));
                    }
                }
            }
        }
    }

    // R6: Validate agent ## Triggers sections
    if agents_dir.is_dir() {
        for entry in std::fs::read_dir(&agents_dir)
            .into_iter()
            .flatten()
            .flatten()
        {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "md") {
                match crate::parser::parse_agent_file(&path) {
                    Ok(agent) => {
                        if let Some(triggers) = &agent.metadata.triggers {
                            let loc = format!(
                                "agents/{}:## Triggers",
                                path.file_name().unwrap().to_string_lossy()
                            );
                            validate_trigger_config(triggers, &loc, &mut issues);
                        }
                    }
                    Err(e) => {
                        issues.push(ValidationIssue::warning(
                            format!("agents/{}", path.file_name().unwrap().to_string_lossy()),
                            format!("Failed to parse agent: {e}"),
                        ));
                    }
                }
            }
        }
    }

    issues
}

/// Validate a project config (armadai.yaml or .armadai/config.yaml).
///
/// # Arguments
/// * `project_root` - Path to the project root directory
///
/// # Returns
/// List of validation issues (empty = valid)
#[allow(dead_code)]
pub fn validate_project_config(project_root: &Path) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();

    // Detect which config file is present
    let config_path = if project_root.join(".armadai/config.yaml").is_file() {
        project_root.join(".armadai/config.yaml")
    } else if project_root.join("armadai.yaml").is_file() {
        project_root.join("armadai.yaml")
    } else if project_root.join("armadai.yml").is_file() {
        project_root.join("armadai.yml")
    } else {
        issues.push(ValidationIssue::error(
            "project",
            "No config file found (expected .armadai/config.yaml or armadai.yaml)",
        ));
        return issues;
    };

    // Load project config
    let config = match ProjectConfig::load(&config_path) {
        Ok(c) => c,
        Err(e) => {
            issues.push(ValidationIssue::error(
                config_path.file_name().unwrap().to_string_lossy(),
                format!("Failed to load config: {e}"),
            ));
            return issues;
        }
    };

    // Build a set of all agent names for cross-reference validation
    let mut agent_names = HashSet::new();
    for agent_ref in &config.agents {
        match agent_ref {
            super::project::AgentRef::Named { name } => {
                agent_names.insert(name.clone());
            }
            super::project::AgentRef::Path { path } => {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    agent_names.insert(stem.to_string());
                }
            }
            super::project::AgentRef::Registry { registry } => {
                agent_names.insert(registry.clone());
            }
            super::project::AgentRef::Declared { declared } => {
                agent_names.insert(declared.clone());
            }
        }
    }

    // Every agent in `.armadai/agents.yaml` is included automatically — it
    // does not need to be relisted in `agents:` (the `Declared` arm above
    // only fires for that redundant `- declared: x` spelling) — so an
    // orchestration `coordinator`/`teams[].lead`/`teams[].agents` naming a
    // declared-but-not-relisted agent must count as known here too, or this
    // reports a project `list`, `link`, `inspect` and `run --orchestrate`
    // all resolve fine as invalid. Guarded the same way
    // `model_updater::check_project` guards its own declarations scan.
    let decls_path = super::agent_source::declarations_path(project_root);
    if decls_path.is_file()
        && let Ok(decls) = super::agent_decl::load(&decls_path)
    {
        for decl in &decls.agents {
            agent_names.insert(decl.name.clone());
        }
    }

    // R1 + R2: Validate orchestration config (teams + coordinator)
    if let Some(orch) = &config.orchestration {
        validate_orchestration_config(orch, &agent_names, &config_path, &mut issues);
    }

    // R3: Validate prompt refs (file existence)
    for (idx, prompt_ref) in config.prompts.iter().enumerate() {
        match super::project::resolve_prompt(prompt_ref, project_root) {
            Ok(_) => {}
            Err(e) => {
                issues.push(ValidationIssue::error(
                    format!("prompts[{idx}]"),
                    format!("{e}"),
                ));
            }
        }
    }

    // R4: Validate skill refs (directory existence with SKILL.md)
    for (idx, skill_ref) in config.skills.iter().enumerate() {
        match super::project::resolve_skill(skill_ref, project_root) {
            Ok(skill_dir) => {
                let skill_md = skill_dir.join("SKILL.md");
                if !skill_md.is_file() {
                    issues.push(ValidationIssue::error(
                        format!("skills[{idx}]"),
                        format!("SKILL.md not found in {}", skill_dir.display()),
                    ));
                }
            }
            Err(e) => {
                issues.push(ValidationIssue::error(
                    format!("skills[{idx}]"),
                    format!("{e}"),
                ));
            }
        }
    }

    // R5: Validate prompt apply_to targets (for resolved prompts)
    for prompt_ref in &config.prompts {
        if let Ok(prompt_path) = super::project::resolve_prompt(prompt_ref, project_root) {
            match Prompt::load(&prompt_path) {
                Ok(prompt) => {
                    for target in &prompt.apply_to {
                        if target == "*" {
                            continue;
                        }
                        let target_name = target.strip_suffix(".md").unwrap_or(target);
                        if !agent_names.contains(target_name) {
                            issues.push(ValidationIssue::error(
                                format!(
                                    "{}:apply_to['{target}']",
                                    prompt_path.file_name().unwrap().to_string_lossy()
                                ),
                                format!("Target agent '{target}' not found in project agents list"),
                            ));
                        }
                    }
                }
                Err(e) => {
                    issues.push(ValidationIssue::warning(
                        format!(
                            "prompts/{}",
                            prompt_path.file_name().unwrap().to_string_lossy()
                        ),
                        format!("Failed to parse prompt: {e}"),
                    ));
                }
            }
        }
    }

    // R6: Validate agent Triggers sections
    for agent_ref in &config.agents {
        if let Ok(agent_path) = super::project::resolve_agent(agent_ref, project_root) {
            match crate::parser::parse_agent_file(&agent_path) {
                Ok(agent) => {
                    if let Some(triggers) = &agent.metadata.triggers {
                        let loc = format!(
                            "agents/{}:## Triggers",
                            agent_path.file_name().unwrap().to_string_lossy()
                        );
                        validate_trigger_config(triggers, &loc, &mut issues);
                    }
                }
                Err(e) => {
                    issues.push(ValidationIssue::warning(
                        format!(
                            "agents/{}",
                            agent_path.file_name().unwrap().to_string_lossy()
                        ),
                        format!("Failed to parse agent: {e}"),
                    ));
                }
            }
        }
    }

    issues
}

// ---------------------------------------------------------------------------
// Internal validation helpers
// ---------------------------------------------------------------------------

/// Validate orchestration config (R1 + R2).
fn validate_orchestration_config(
    orch: &OrchestrationConfig,
    agent_names: &HashSet<String>,
    config_path: &Path,
    issues: &mut Vec<ValidationIssue>,
) {
    // R2: Validate coordinator is in agents list
    if let Some(ref coordinator) = orch.coordinator
        && !agent_names.contains(coordinator)
    {
        issues.push(ValidationIssue::error(
            format!(
                "{}:orchestration.coordinator",
                config_path.file_name().unwrap().to_string_lossy()
            ),
            format!("Coordinator '{coordinator}' not found in agents list"),
        ));
    }

    // R1: Validate team members and leads are in agents list
    for (team_idx, team) in orch.teams.iter().enumerate() {
        if let Some(ref lead) = team.lead
            && !agent_names.contains(lead)
        {
            issues.push(ValidationIssue::error(
                format!(
                    "{}:orchestration.teams[{team_idx}].lead",
                    config_path.file_name().unwrap().to_string_lossy()
                ),
                format!("Team lead '{lead}' not found in agents list"),
            ));
        }
        for (agent_idx, agent) in team.agents.iter().enumerate() {
            if !agent_names.contains(agent) {
                issues.push(ValidationIssue::error(
                    format!(
                        "{}:orchestration.teams[{team_idx}].agents[{agent_idx}]",
                        config_path.file_name().unwrap().to_string_lossy()
                    ),
                    format!("Team member '{agent}' not found in agents list"),
                ));
            }
        }
    }
}

/// Validate a TriggerConfig (R6).
fn validate_trigger_config(
    triggers: &super::orchestration::TriggerConfig,
    location: &str,
    issues: &mut Vec<ValidationIssue>,
) {
    // Valid entry kinds (closed enum)
    const VALID_KINDS: &[&str] = &[
        "finding",
        "challenge",
        "confirmation",
        "synthesis",
        "question",
        "answer",
    ];

    // Validate requires
    for kind in &triggers.requires {
        let kind_lower = kind.to_lowercase();
        if !VALID_KINDS.contains(&kind_lower.as_str()) {
            issues.push(ValidationIssue::error(
                format!("{location}:requires"),
                format!(
                    "Invalid kind '{kind}' — must be one of: {}",
                    VALID_KINDS.join(", ")
                ),
            ));
        }
    }

    // Validate excludes
    for kind in &triggers.excludes {
        let kind_lower = kind.to_lowercase();
        if !VALID_KINDS.contains(&kind_lower.as_str()) {
            issues.push(ValidationIssue::error(
                format!("{location}:excludes"),
                format!(
                    "Invalid kind '{kind}' — must be one of: {}",
                    VALID_KINDS.join(", ")
                ),
            ));
        }
    }

    // Validate priority (0-100)
    if triggers.priority > 100 {
        issues.push(ValidationIssue::warning(
            format!("{location}:priority"),
            format!("Priority {} is out of range 0-100", triggers.priority),
        ));
    }

    // Validate min_round >= 0 (always true for u32, but for clarity)
    // No explicit check needed — u32 is always >= 0

    // Validate max_round >= min_round if set
    if let Some(max) = triggers.max_round
        && max < triggers.min_round
    {
        issues.push(ValidationIssue::warning(
            format!("{location}:max_round"),
            format!(
                "max_round ({max}) is less than min_round ({})",
                triggers.min_round
            ),
        ));
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    // ── Pack validation tests ──────────────────────────────────────

    #[test]
    fn test_validate_pack_happy_path() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("pack.yaml"),
            "name: test\ndescription: Test pack\n",
        )
        .unwrap();

        let issues = validate_pack(dir.path());
        assert_eq!(issues.len(), 0, "Expected 0 issues, got {issues:?}");
    }

    #[test]
    fn test_validate_pack_r1_agent_not_found() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("pack.yaml"),
            "name: test\ndescription: Test\nagents:\n  - missing-agent\n",
        )
        .unwrap();

        let issues = validate_pack(dir.path());
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].severity, Severity::Error);
        assert!(issues[0].message.contains("not found"));
        assert!(issues[0].location.contains("missing-agent"));
    }

    #[test]
    fn test_validate_pack_r3_prompt_not_found() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("pack.yaml"),
            "name: test\ndescription: Test\nprompts:\n  - missing-prompt\n",
        )
        .unwrap();

        let issues = validate_pack(dir.path());
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].severity, Severity::Error);
        assert!(issues[0].message.contains("not found"));
        assert!(issues[0].location.contains("missing-prompt"));
    }

    #[test]
    fn test_validate_pack_r5_apply_to_invalid_target() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("pack.yaml"),
            "name: test\ndescription: Test\nagents:\n  - real-agent\nprompts:\n  - my-prompt\n",
        )
        .unwrap();

        let agents_dir = dir.path().join("agents");
        fs::create_dir_all(&agents_dir).unwrap();
        fs::write(
            agents_dir.join("real-agent.md"),
            "# Real Agent\n\n## Metadata\n- provider: claude\n- model: latest:pro\n\n## System Prompt\nTest\n",
        )
        .unwrap();

        let prompts_dir = dir.path().join("prompts");
        fs::create_dir_all(&prompts_dir).unwrap();
        fs::write(
            prompts_dir.join("my-prompt.md"),
            "---\napply_to:\n  - nonexistent-agent\n---\nBody",
        )
        .unwrap();

        let issues = validate_pack(dir.path());
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].severity, Severity::Error);
        assert!(issues[0].message.contains("not found"));
        assert!(issues[0].location.contains("apply_to"));
    }

    #[test]
    fn test_validate_pack_r6_triggers_invalid_kind() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("pack.yaml"),
            "name: test\ndescription: Test\nagents:\n  - test-agent\n",
        )
        .unwrap();

        let agents_dir = dir.path().join("agents");
        fs::create_dir_all(&agents_dir).unwrap();
        fs::write(
            agents_dir.join("test-agent.md"),
            "# Test Agent\n\n## Metadata\n- provider: claude\n- model: latest:pro\n\n## System Prompt\nTest\n\n## Triggers\n- requires: [invalid-kind]\n- priority: 50\n",
        )
        .unwrap();

        let issues = validate_pack(dir.path());
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].severity, Severity::Error);
        assert!(issues[0].message.contains("Invalid kind"));
        assert!(issues[0].location.contains("Triggers"));
    }

    #[test]
    fn test_validate_pack_r6_triggers_priority_out_of_range() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("pack.yaml"),
            "name: test\ndescription: Test\nagents:\n  - test-agent\n",
        )
        .unwrap();

        let agents_dir = dir.path().join("agents");
        fs::create_dir_all(&agents_dir).unwrap();
        fs::write(
            agents_dir.join("test-agent.md"),
            "# Test Agent\n\n## Metadata\n- provider: claude\n- model: latest:pro\n\n## System Prompt\nTest\n\n## Triggers\n- priority: 150\n",
        )
        .unwrap();

        let issues = validate_pack(dir.path());
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].severity, Severity::Warning);
        assert!(issues[0].message.contains("out of range"));
    }

    #[test]
    fn test_validate_pack_r6_triggers_valid_kinds() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("pack.yaml"),
            "name: test\ndescription: Test\nagents:\n  - test-agent\n",
        )
        .unwrap();

        let agents_dir = dir.path().join("agents");
        fs::create_dir_all(&agents_dir).unwrap();
        fs::write(
            agents_dir.join("test-agent.md"),
            "# Test Agent\n\n## Metadata\n- provider: claude\n- model: latest:pro\n\n## System Prompt\nTest\n\n## Triggers\n- requires: [finding, challenge]\n- excludes: [synthesis]\n- priority: 80\n",
        )
        .unwrap();

        let issues = validate_pack(dir.path());
        assert_eq!(issues.len(), 0, "Expected 0 issues, got {issues:?}");
    }

    // ── Project config validation tests ────────────────────────────

    #[test]
    fn test_validate_project_config_r1_team_member_not_in_agents() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("armadai.yaml"),
            r#"
agents:
  - name: coordinator
  - name: real-agent
orchestration:
  coordinator: coordinator
  teams:
    - agents:
        - real-agent
        - missing-agent
"#,
        )
        .unwrap();

        let issues = validate_project_config(dir.path());
        assert!(issues.iter().any(|i| i.severity == Severity::Error
            && i.message.contains("missing-agent")
            && i.message.contains("not found")));
    }

    #[test]
    fn test_validate_project_config_r2_coordinator_not_in_agents() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("armadai.yaml"),
            r#"
agents:
  - name: agent-a
orchestration:
  coordinator: missing-coordinator
  teams: []
"#,
        )
        .unwrap();

        let issues = validate_project_config(dir.path());
        assert!(issues.iter().any(|i| i.severity == Severity::Error
            && i.message.contains("missing-coordinator")
            && i.message.contains("not found")));
    }

    /// I1: an agent declared only in `.armadai/agents.yaml` — never relisted
    /// in `armadai.yaml`'s `agents:`, which is the whole point of the format
    /// — must count as known to `orchestration.coordinator`/`teams[].lead`/
    /// `teams[].agents`. Before this fix, `agent_names` was built from
    /// `config.agents` alone, so this reported three false ERRORs for a
    /// project `list`/`link`/`inspect`/`run --orchestrate` all resolve fine.
    #[test]
    fn test_validate_project_config_resolves_declared_only_orchestration_agents() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("armadai.yaml"),
            r#"
orchestration:
  enabled: true
  coordinator: zzz-lead
  teams:
    - lead: zzz-worker
      agents:
        - zzz-worker2
"#,
        )
        .unwrap();
        fs::create_dir_all(dir.path().join(".armadai")).unwrap();
        fs::write(
            dir.path().join(".armadai/agents.yaml"),
            "defaults:\n  provider: claude\nagents:\n  \
             - name: zzz-lead\n    prompt: []\n  \
             - name: zzz-worker\n    prompt: []\n  \
             - name: zzz-worker2\n    prompt: []\n",
        )
        .unwrap();

        let issues = validate_project_config(dir.path());
        assert!(
            issues.iter().all(|i| i.severity != Severity::Error),
            "every named agent is declared and must resolve: {issues:?}"
        );
    }

    #[test]
    fn test_validate_project_config_no_config_file() {
        let dir = tempfile::tempdir().unwrap();

        let issues = validate_project_config(dir.path());
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].severity, Severity::Error);
        assert!(issues[0].message.contains("No config file found"));
    }
}
