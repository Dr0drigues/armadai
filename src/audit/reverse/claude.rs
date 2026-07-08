use std::path::Path;

use serde::Deserialize;

use super::{
    ImportedAgent, ImportedConfig, ImportedInstructions, ImportedSkill, ParseIssue,
    PartialMetadata, ReverseLinker,
};
use crate::parser::frontmatter::extract_frontmatter;

/// Reads native Claude Code configuration surfaces.
pub struct ClaudeReverseLinker;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ClaudeAgentFrontmatter {
    name: Option<String>,
    description: Option<String>,
    model: Option<String>,
    tools: Option<ToolsField>,
}

/// Claude Code accepts `tools: Read, Grep` (CSV) or a YAML list.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ToolsField {
    List(Vec<String>),
    Csv(String),
}

impl ToolsField {
    fn into_vec(self) -> Vec<String> {
        match self {
            ToolsField::List(v) => v,
            ToolsField::Csv(s) => s
                .split(',')
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .collect(),
        }
    }
}

impl ReverseLinker for ClaudeReverseLinker {
    fn name(&self) -> &'static str {
        "claude"
    }

    fn detect(&self, root: &Path) -> bool {
        root.join(".claude/agents").is_dir()
            || root.join(".claude/skills").is_dir()
            || root.join("CLAUDE.md").is_file()
    }

    fn parse(&self, root: &Path) -> ImportedConfig {
        ImportedConfig {
            agents: parse_agents(&root.join(".claude/agents")),
            skills: parse_skills(&root.join(".claude/skills")),
            instructions: parse_instructions(&root.join("CLAUDE.md")),
        }
    }
}

/// Claude Code discovers agents anywhere under `.claude/agents/`, including
/// nested subdirectories (e.g. `.claude/agents/backend/dev.md`).
const MAX_AGENT_SCAN_DEPTH: u32 = 3;

fn parse_agents(dir: &Path) -> Vec<ImportedAgent> {
    let mut files = Vec::new();
    collect_agent_files(dir, MAX_AGENT_SCAN_DEPTH, &mut files);
    let mut agents: Vec<ImportedAgent> = files.iter().map(|p| parse_agent_file(p)).collect();
    agents.sort_by(|a, b| a.name.cmp(&b.name));
    agents
}

/// Recursively collects `*.md` files under `dir`, up to `depth` levels of
/// nesting. Directories named `foo.md` are skipped: only real files count.
fn collect_agent_files(dir: &Path, depth: u32, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_file() {
            if p.extension().is_some_and(|ext| ext == "md") {
                out.push(p);
            }
        } else if p.is_dir() && depth > 0 {
            collect_agent_files(&p, depth - 1, out);
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct SkillFm {
    name: Option<String>,
    description: Option<String>,
}

fn parse_skills(dir: &Path) -> Vec<ImportedSkill> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut skills: Vec<ImportedSkill> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .map(|p| parse_skill_dir(&p))
        .collect();
    skills.sort_by(|a, b| a.name.cmp(&b.name));
    skills
}

fn parse_skill_dir(dir: &Path) -> ImportedSkill {
    let dir_name = dir
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let skill_md = dir.join("SKILL.md");
    let Ok(content) = std::fs::read_to_string(&skill_md) else {
        return ImportedSkill {
            name: dir_name,
            source_path: dir.to_path_buf(),
            description: None,
            has_skill_md: false,
            frontmatter_ok: false,
        };
    };
    let (fm_raw, _body) = extract_frontmatter(&content);
    let (fm, frontmatter_ok) = match fm_raw.map(serde_yaml_ng::from_str::<SkillFm>) {
        Some(Ok(fm)) => (fm, true),
        Some(Err(_)) | None => (SkillFm::default(), false),
    };
    ImportedSkill {
        name: fm.name.unwrap_or(dir_name),
        source_path: skill_md,
        description: fm.description,
        has_skill_md: true,
        frontmatter_ok,
    }
}

fn parse_instructions(path: &Path) -> Option<ImportedInstructions> {
    let content = std::fs::read_to_string(path).ok()?;
    Some(ImportedInstructions {
        source_path: path.to_path_buf(),
        content,
    })
}

fn parse_agent_file(path: &Path) -> ImportedAgent {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            return ImportedAgent {
                name: stem,
                source_path: path.to_path_buf(),
                metadata: PartialMetadata::default(),
                system_prompt: String::new(),
                issues: vec![issue(path, format!("unreadable file: {e}"))],
            };
        }
    };
    let mut issues = Vec::new();
    let (fm_raw, body) = extract_frontmatter(&content);
    let fm: ClaudeAgentFrontmatter = match fm_raw {
        Some(raw) => serde_yaml_ng::from_str(raw).unwrap_or_else(|e| {
            issues.push(issue(path, format!("invalid YAML frontmatter: {e}")));
            ClaudeAgentFrontmatter::default()
        }),
        None => {
            issues.push(issue(path, "missing YAML frontmatter".to_string()));
            ClaudeAgentFrontmatter::default()
        }
    };
    ImportedAgent {
        name: fm.name.unwrap_or(stem),
        source_path: path.to_path_buf(),
        metadata: PartialMetadata {
            description: fm.description,
            model: fm.model,
            tools: fm.tools.map(ToolsField::into_vec),
        },
        system_prompt: body.trim().to_string(),
        issues,
    }
}

fn issue(path: &Path, message: String) -> ParseIssue {
    ParseIssue {
        file: path.to_path_buf(),
        message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, rel: &str, content: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, content).unwrap();
    }

    #[test]
    fn detect_requires_a_claude_surface() {
        let dir = tempfile::tempdir().unwrap();
        let linker = ClaudeReverseLinker;
        assert!(!linker.detect(dir.path()));
        write(
            dir.path(),
            ".claude/agents/reviewer.md",
            "---\nname: reviewer\n---\nBody",
        );
        assert!(linker.detect(dir.path()));
    }

    #[test]
    fn parse_agent_with_csv_tools() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            ".claude/agents/reviewer.md",
            "---\nname: reviewer\ndescription: Reviews code\nmodel: claude-sonnet-5\ntools: Read, Grep\n---\nYou review code.",
        );
        let config = ClaudeReverseLinker.parse(dir.path());
        assert_eq!(config.agents.len(), 1);
        let a = &config.agents[0];
        assert_eq!(a.name, "reviewer");
        assert_eq!(a.metadata.description.as_deref(), Some("Reviews code"));
        assert_eq!(a.metadata.model.as_deref(), Some("claude-sonnet-5"));
        assert_eq!(
            a.metadata.tools.as_deref(),
            Some(&["Read".to_string(), "Grep".to_string()][..])
        );
        assert_eq!(a.system_prompt, "You review code.");
        assert!(a.issues.is_empty());
    }

    #[test]
    fn parse_agent_with_list_tools_and_name_fallback() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            ".claude/agents/helper.md",
            "---\ndescription: Helps\ntools:\n  - Read\n  - Write\n---\nHelp.",
        );
        let config = ClaudeReverseLinker.parse(dir.path());
        let a = &config.agents[0];
        assert_eq!(a.name, "helper"); // fallback = file stem
        assert_eq!(
            a.metadata.tools.as_deref(),
            Some(&["Read".to_string(), "Write".to_string()][..])
        );
    }

    #[test]
    fn broken_yaml_becomes_issue_not_error() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            ".claude/agents/broken.md",
            "---\nname: [unclosed\n---\nBody",
        );
        let config = ClaudeReverseLinker.parse(dir.path());
        assert_eq!(config.agents.len(), 1);
        assert_eq!(config.agents[0].name, "broken");
        assert_eq!(config.agents[0].issues.len(), 1);
    }

    #[test]
    fn unreadable_file_yields_a_single_issue() {
        // A directory named `*.md` cannot be read as a file: this exercises
        // the early-return path without relying on filesystem permissions.
        let dir = tempfile::tempdir().unwrap();
        let bad_path = dir.path().join("agent-dir.md");
        std::fs::create_dir_all(&bad_path).unwrap();
        let agent = parse_agent_file(&bad_path);
        assert_eq!(agent.issues.len(), 1);
        assert!(agent.issues[0].message.contains("unreadable file"));
        assert!(agent.system_prompt.is_empty());
    }

    #[test]
    fn parse_agents_recurses_into_subdirectories() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            ".claude/agents/team/nested.md",
            "---\nname: nested\n---\nBody",
        );
        let config = ClaudeReverseLinker.parse(dir.path());
        assert_eq!(config.agents.len(), 1);
        assert_eq!(config.agents[0].name, "nested");
    }

    #[test]
    fn parse_agents_skips_directories_named_like_md_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".claude/agents/weird.md")).unwrap();
        let config = ClaudeReverseLinker.parse(dir.path());
        assert!(config.agents.is_empty());
    }

    #[test]
    fn parse_skills_and_instructions() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "CLAUDE.md",
            "# Project\nUse @reviewer for reviews.",
        );
        write(
            dir.path(),
            ".claude/skills/deploy/SKILL.md",
            "---\nname: deploy\ndescription: Deploys the app\n---\nSteps.",
        );
        std::fs::create_dir_all(dir.path().join(".claude/skills/empty-skill")).unwrap();
        let config = ClaudeReverseLinker.parse(dir.path());
        assert_eq!(config.skills.len(), 2);
        let deploy = config.skills.iter().find(|s| s.name == "deploy").unwrap();
        assert!(deploy.has_skill_md && deploy.frontmatter_ok);
        assert_eq!(deploy.description.as_deref(), Some("Deploys the app"));
        let empty = config
            .skills
            .iter()
            .find(|s| s.name == "empty-skill")
            .unwrap();
        assert!(!empty.has_skill_md);
        assert!(config.instructions.unwrap().content.contains("@reviewer"));
    }

    #[test]
    fn skill_with_invalid_yaml_falls_back_to_dir_name() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            ".claude/skills/broken-skill/SKILL.md",
            "---\nname: [unclosed\n---\nBody",
        );
        let config = ClaudeReverseLinker.parse(dir.path());
        assert_eq!(config.skills.len(), 1);
        let skill = &config.skills[0];
        assert!(skill.has_skill_md);
        assert!(!skill.frontmatter_ok);
        assert_eq!(skill.name, "broken-skill");
    }
}
