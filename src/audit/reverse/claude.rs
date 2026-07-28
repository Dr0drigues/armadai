use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

use super::{
    ImportedAgent, ImportedConfig, ImportedInstructions, ImportedSkill, ParseIssue,
    PartialMetadata, ReverseLinker,
};
use crate::core::parser::frontmatter::extract_frontmatter;

/// Reads native Claude Code configuration surfaces.
pub struct ClaudeReverseLinker;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ClaudeAgentFrontmatter {
    name: Option<String>,
    description: Option<String>,
    model: Option<String>,
    tools: Option<ToolsField>,
    #[serde(flatten)]
    extra: BTreeMap<String, serde_yaml_ng::Value>,
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
    #[serde(flatten)]
    extra: BTreeMap<String, serde_yaml_ng::Value>,
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
            has_frontmatter: false,
            issues: Vec::new(),
            extra: BTreeMap::new(),
        };
    };
    let mut issues = Vec::new();
    let (fm_raw, _body) = extract_frontmatter(&content);
    let (fm, has_frontmatter) = match fm_raw {
        Some(raw) => {
            let fm = serde_yaml_ng::from_str::<SkillFm>(raw).unwrap_or_else(|e| {
                issues.push(issue(&skill_md, describe_yaml_error(&content, raw, &e)));
                SkillFm {
                    name: salvage_field(raw, "name"),
                    description: salvage_field(raw, "description"),
                    extra: BTreeMap::new(),
                }
            });
            (fm, true)
        }
        None => (SkillFm::default(), false),
    };
    ImportedSkill {
        name: fm.name.unwrap_or(dir_name),
        source_path: skill_md,
        description: fm.description,
        has_skill_md: true,
        has_frontmatter,
        issues,
        extra: fm.extra,
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
            issues.push(issue(path, describe_yaml_error(&content, raw, &e)));
            ClaudeAgentFrontmatter {
                name: salvage_field(raw, "name"),
                description: salvage_field(raw, "description"),
                model: salvage_field(raw, "model"),
                tools: salvage_field(raw, "tools").map(ToolsField::Csv),
                extra: BTreeMap::new(),
            }
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
            extra: fm.extra,
        },
        system_prompt: body.trim().to_string(),
        issues,
    }
}

/// Best-effort recovery of one simple top-level `key: value` line from a
/// frontmatter that strict YAML rejected. Broken flow scalars (`[`, `{`)
/// and block scalars (`|`, `>`) are skipped so historical fallbacks (file
/// stem) keep working, and indented lines (nested keys, block-scalar
/// bodies) never masquerade as a top-level key.
fn salvage_field(raw: &str, key: &str) -> Option<String> {
    raw.lines().find_map(|line| {
        // A top-level key starts at column 0: nested keys and block-scalar
        // bodies are indented and must not match.
        if line.starts_with(char::is_whitespace) {
            return None;
        }
        let (k, v) = line.split_once(':')?;
        if k.trim() != key {
            return None;
        }
        // Strip a trailing YAML comment before any further processing.
        let v = match v.find(" #") {
            Some(idx) => &v[..idx],
            None => v,
        };
        let v = v.trim();
        if v.is_empty()
            || v.starts_with('[')
            || v.starts_with('{')
            || v.starts_with('|')
            || v.starts_with('>')
        {
            return None;
        }
        Some(v.trim_matches('"').trim_matches('\'').to_string())
    })
}

/// Turn a strict-YAML error into a message the Markdown-writing user can
/// act on. The dominant real-world failure is an unquoted value containing
/// `: `, which YAML and Claude Code both reject.
///
/// `content` is the full file content (to calculate line offset correctly).
/// `raw` is the extracted frontmatter YAML (between `---` delimiters).
fn describe_yaml_error(content: &str, raw: &str, err: &serde_yaml_ng::Error) -> String {
    if let Some(loc) = err.location() {
        // Calculate how many lines precede the frontmatter in the original file.
        // extract_frontmatter() does trim_start(), so we must count stripped lines.
        let trimmed = content.trim_start();
        let prefix_len = content.len() - trimmed.len();
        let lines_before_frontmatter = if prefix_len > 0 {
            content[..prefix_len].chars().filter(|&c| c == '\n').count()
        } else {
            0
        };
        // File line (1-indexed) = lines_before + 1 (opening `---`) + loc.line().
        // `loc.line()` from serde_yaml_ng is already 1-indexed and counts from
        // the first YAML line (right after the opening `---`), so it must not
        // be double-counted with an extra "+1 for the first YAML line".
        let file_line = lines_before_frontmatter + 1 + loc.line();

        if let Some(line) = raw.lines().nth(loc.line().saturating_sub(1))
            && let Some((key, value)) = line.split_once(':')
            && value.contains(": ")
        {
            return format!(
                "unquoted '{}:' value contains ': ' (line {file_line}) — wrap the value in double quotes (YAML and Claude Code both reject it as-is)",
                key.trim()
            );
        }
        return format!("invalid YAML frontmatter (line {file_line}): {err}");
    }
    format!("invalid YAML frontmatter: {err}")
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
        assert!(deploy.has_skill_md && deploy.has_frontmatter && deploy.issues.is_empty());
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
    fn unquoted_colon_gets_pedagogical_message_and_salvage() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            ".claude/agents/gate.md",
            "---\nname: gate\nmodel: opus\ndescription: wraps both phases (one engine : selection + cache)\n---\nBody",
        );
        let config = ClaudeReverseLinker.parse(dir.path());
        let a = &config.agents[0];
        assert_eq!(a.issues.len(), 1);
        assert!(
            a.issues[0]
                .message
                .contains("wrap the value in double quotes")
        );
        // Salvage recovered the real fields despite the strict-YAML failure.
        assert_eq!(a.name, "gate");
        assert_eq!(a.metadata.model.as_deref(), Some("opus"));
        assert!(
            a.metadata
                .description
                .as_deref()
                .unwrap_or("")
                .starts_with("wraps both phases")
        );
    }

    #[test]
    fn salvage_skips_broken_flow_scalars() {
        // `name: [unclosed` must NOT be salvaged into the agent name —
        // keeps the historical stem fallback intact.
        assert_eq!(salvage_field("name: [unclosed", "name"), None);
        assert_eq!(
            salvage_field("model: opus", "model"),
            Some("opus".to_string())
        );
        assert_eq!(
            salvage_field("desc: \"quoted\"", "desc"),
            Some("quoted".to_string())
        );
    }

    #[test]
    fn salvage_skips_block_scalars_comments_and_indented_keys() {
        assert_eq!(salvage_field("description: >-", "description"), None);
        assert_eq!(salvage_field("description: |", "description"), None);
        assert_eq!(
            salvage_field("model: opus # fast", "model"),
            Some("opus".to_string())
        );
        assert_eq!(salvage_field("  model: nested", "model"), None);
    }

    #[test]
    fn skill_with_invalid_yaml_gets_issue_and_salvage() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            ".claude/skills/triage/SKILL.md",
            "---\nname: triage\ndescription: triage is HUMAN — the skill : just assists\n---\nBody",
        );
        let config = ClaudeReverseLinker.parse(dir.path());
        let skill = &config.skills[0];
        assert!(skill.has_skill_md && skill.has_frontmatter);
        assert_eq!(skill.issues.len(), 1);
        assert!(
            skill.issues[0]
                .message
                .contains("wrap the value in double quotes")
        );
        assert_eq!(skill.name, "triage"); // salvaged
        assert!(
            skill
                .description
                .as_deref()
                .unwrap_or("")
                .starts_with("triage is HUMAN")
        );
    }

    #[test]
    fn skill_without_frontmatter_has_no_issue() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            ".claude/skills/bare/SKILL.md",
            "# Just a title\nBody.",
        );
        let config = ClaudeReverseLinker.parse(dir.path());
        let skill = &config.skills[0];
        assert!(skill.has_skill_md && !skill.has_frontmatter);
        assert!(skill.issues.is_empty()); // standard violation, not a parse error
    }

    #[test]
    fn unknown_frontmatter_fields_are_kept_in_extra() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            ".claude/agents/scoped.md",
            "---\nname: scoped\ndescription: d\npaths:\n  - src/cli/**\n  - docs/\neffort: medium\n---\nBody",
        );
        let config = ClaudeReverseLinker.parse(dir.path());
        let a = &config.agents[0];
        assert!(a.metadata.extra.contains_key("effort"));
        assert!(a.metadata.extra.contains_key("paths"));
        assert!(!a.metadata.extra.contains_key("name")); // typed fields never land in extra
    }

    #[test]
    fn yaml_error_line_offset_without_leading_blank_lines() {
        // Regression test: with no blank lines before the opening `---`, the
        // reported file line must exactly match the physical line of the
        // erroring field. File layout:
        //   line 1: ---
        //   line 2: name: test
        //   line 3: description: bad : unquoted
        //   line 4: ---
        //   line 5: Body
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            ".claude/agents/offset_no_blank.md",
            "---\nname: test\ndescription: bad : unquoted\n---\nBody",
        );
        let config = ClaudeReverseLinker.parse(dir.path());
        let a = &config.agents[0];
        assert_eq!(a.issues.len(), 1);
        assert!(
            a.issues[0].message.contains("line 3"),
            "expected exact 'line 3' reference, got: {}",
            a.issues[0].message
        );
    }

    #[test]
    fn yaml_error_line_offset_with_leading_blank_lines() {
        // Regression test for the off-by-one offset bug: when the file has
        // leading blank lines, describe_yaml_error must correctly offset the
        // error line number relative to the file start (not the trimmed
        // content), without over- or under-counting. File layout:
        //   line 1-3: blank
        //   line 4: ---
        //   line 5: name: test
        //   line 6: description: bad : unquoted
        //   line 7: ---
        //   line 8: Body
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            ".claude/agents/offset_with_blank.md",
            "\n\n\n---\nname: test\ndescription: bad : unquoted\n---\nBody",
        );
        let config = ClaudeReverseLinker.parse(dir.path());
        let a = &config.agents[0];
        assert_eq!(a.issues.len(), 1);
        assert!(
            a.issues[0].message.contains("line 6"),
            "expected exact 'line 6' reference, got: {}",
            a.issues[0].message
        );
    }
}
