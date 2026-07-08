use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::{
    ImportedAgent, ImportedConfig, ImportedInstructions, ImportedSkill, ParseIssue,
    PartialMetadata, ReverseLinker,
};
use crate::linker::LinkTarget;
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
            skills: Vec::new(), // Task 3
            instructions: None, // Task 3
        }
    }
}

fn parse_agents(dir: &Path) -> Vec<ImportedAgent> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut agents: Vec<ImportedAgent> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "md"))
        .map(|p| parse_agent_file(&p))
        .collect();
    agents.sort_by(|a, b| a.name.cmp(&b.name));
    agents
}

fn parse_agent_file(path: &Path) -> ImportedAgent {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let mut issues = Vec::new();
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            issues.push(issue(path, format!("unreadable file: {e}")));
            String::new()
        }
    };
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
        source_format: LinkTarget::Claude,
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
}
