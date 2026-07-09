//! Generation of an ArmadAI proposal pack from imported native configs.
use std::fmt::Write as _;
use std::path::Path;

use super::reverse::ImportedAgent;
use crate::linker::model_aliases::resolve_alias;
use crate::linker::model_resolution::{
    classify_model_tier, is_latest_placeholder, tier_placeholder,
};

/// Map a native model to a portable ArmadAI tier when possible.
pub(crate) fn portable_model(model: Option<&str>) -> String {
    let Some(model) = model else {
        return "latest:pro".to_string();
    };
    if is_latest_placeholder(model) {
        return model.to_string();
    }
    let resolved = resolve_alias(model).unwrap_or_else(|| model.to_string());
    if is_latest_placeholder(&resolved) {
        return resolved;
    }
    match classify_model_tier(&resolved, "anthropic") {
        Some(tier) => tier_placeholder(tier).to_string(),
        None => resolved,
    }
}

/// Render an imported agent in the ArmadAI agent format
/// (H1 + `## Metadata` list + `## System Prompt`).
pub(crate) fn render_agent(agent: &ImportedAgent) -> String {
    let mut md = String::new();
    let _ = writeln!(md, "# {}\n", agent.name);
    let description = agent
        .metadata
        .description
        .as_deref()
        .unwrap_or("Imported from native Claude Code configuration.");
    let _ = writeln!(md, "> {description}\n");
    let _ = writeln!(md, "## Metadata");
    let _ = writeln!(md, "- provider: claude");
    let _ = writeln!(
        md,
        "- model: {}",
        portable_model(agent.metadata.model.as_deref())
    );
    if agent.metadata.description.is_some() {
        // Ignored by today's parser (unknown key -> debug log); forward-compatible.
        let _ = writeln!(md, "- description: {description}");
    }
    let _ = writeln!(md, "- tags: [imported]");
    let globs = agent.metadata.scope_globs();
    if !globs.is_empty() {
        let _ = writeln!(md, "- scope: [{}]", globs.join(", "));
    }
    let _ = writeln!(md, "\n## System Prompt\n");
    let _ = writeln!(md, "{}", agent.system_prompt);
    md
}

/// A prompt fragment shared by several agents, extracted from a duplication cluster.
#[derive(Debug, Clone)]
pub(crate) struct SharedFragment {
    pub name: String,
    pub apply_to: Vec<String>,
    pub body: String,
}

fn norm_lines(text: &str) -> Vec<&str> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect()
}

/// Longest contiguous run of normalized lines from `base` that appears
/// (as a contiguous run) in every other member. O(n²·m) on small prompts.
fn longest_common_block<'a>(members: &[Vec<&'a str>]) -> Vec<&'a str> {
    let Some((base, others)) = members.split_first() else {
        return Vec::new();
    };
    let mut best: (usize, usize) = (0, 0); // (start, len)
    let n = base.len();
    for start in 0..n {
        if n - start <= best.1 {
            break;
        }
        let mut len = n - start;
        while len > best.1 {
            let candidate = &base[start..start + len];
            if others.iter().all(|o| contains_run(o, candidate)) {
                best = (start, len);
                break;
            }
            len -= 1;
        }
    }
    base[best.0..best.0 + best.1].to_vec()
}

fn contains_run(haystack: &[&str], needle: &[&str]) -> bool {
    if needle.is_empty() || haystack.len() < needle.len() {
        return false;
    }
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// Find the longest block of lines (≥8) common to every agent in `agents`,
/// and turn it into a named `SharedFragment`. Returns `None` when no common
/// block reaches the minimum window.
pub(crate) fn extract_shared_fragment(
    agents: &[&ImportedAgent],
    index: usize,
) -> Option<SharedFragment> {
    let members: Vec<Vec<&str>> = agents
        .iter()
        .map(|a| norm_lines(&a.system_prompt))
        .collect();
    let block = longest_common_block(&members);
    if block.len() < 8 {
        return None;
    }
    Some(SharedFragment {
        name: format!("shared-conventions-{}", index + 1),
        apply_to: agents.iter().map(|a| a.name.clone()).collect(),
        body: block.join("\n"),
    })
}

/// Remove the first occurrence of `fragment_body` (matched on trimmed,
/// non-empty lines) from `prompt`, then compact resulting triple newlines.
pub(crate) fn strip_fragment(prompt: &str, fragment_body: &str) -> String {
    let needle: Vec<&str> = norm_lines(fragment_body);
    let raw: Vec<&str> = prompt.lines().collect();
    // raw indices of non-empty lines
    let idx: Vec<usize> = raw
        .iter()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty())
        .map(|(i, _)| i)
        .collect();
    let norm: Vec<&str> = idx.iter().map(|&i| raw[i].trim()).collect();
    let Some(pos) = norm
        .windows(needle.len().max(1))
        .position(|w| w == needle.as_slice())
    else {
        return prompt.to_string();
    };
    let (from, to) = (idx[pos], idx[pos + needle.len() - 1]);
    let kept: Vec<&str> = raw
        .iter()
        .enumerate()
        .filter(|(i, _)| *i < from || *i > to)
        .map(|(_, l)| *l)
        .collect();
    let mut out = kept.join("\n");
    while out.contains("\n\n\n") {
        out = out.replace("\n\n\n", "\n\n");
    }
    out
}

/// Render a shared fragment in the ArmadAI prompt-fragment format
/// (YAML frontmatter + body).
pub(crate) fn render_prompt(f: &SharedFragment) -> String {
    let mut md = String::new();
    let _ = writeln!(md, "---");
    let _ = writeln!(md, "name: {}", f.name);
    let _ = writeln!(
        md,
        "description: Shared conventions extracted from {} agents by armadai audit --propose",
        f.apply_to.len()
    );
    let _ = writeln!(md, "apply_to:");
    for name in &f.apply_to {
        let _ = writeln!(md, "  - {name}");
    }
    let _ = writeln!(md, "---");
    let _ = writeln!(md, "{}", f.body);
    md
}

/// Rewrite a `SKILL.md` frontmatter (a) `tools:` -> `allowed-tools:` (the
/// field Claude Code actually reads) and (b) quote `description:`/`name:`
/// values that contain `: ` unquoted (would otherwise break YAML parsing).
/// Only lines between the first two `---` delimiters are touched.
pub(crate) fn fix_skill_md(content: &str) -> (String, Vec<&'static str>) {
    let mut fixes = Vec::new();
    let mut in_frontmatter = false;
    let mut seen_delims = 0u8;
    let lines: Vec<String> = content
        .lines()
        .map(|line| {
            if line.trim() == "---" && seen_delims < 2 {
                seen_delims += 1;
                in_frontmatter = seen_delims == 1;
                return line.to_string();
            }
            if !in_frontmatter || seen_delims != 1 {
                return line.to_string();
            }
            if let Some(rest) = line.strip_prefix("tools:") {
                fixes.push("tools->allowed-tools");
                return format!("allowed-tools:{rest}");
            }
            if let Some((key, value)) = line.split_once(':')
                && matches!(key.trim(), "description" | "name")
            {
                let v = value.trim();
                if v.contains(": ") && !v.starts_with('"') && !v.starts_with('\'') {
                    fixes.push("quoted-value");
                    return format!("{}: \"{v}\"", key.trim_end());
                }
            }
            line.to_string()
        })
        .collect();
    (
        lines.join("\n") + if content.ends_with('\n') { "\n" } else { "" },
        fixes,
    )
}

/// Recursively copy a skill directory (depth <= 5), applying `fix_skill_md`
/// to the root `SKILL.md`. Returns the list of fixes applied.
pub(crate) fn copy_skill_dir(src: &Path, dest: &Path) -> anyhow::Result<Vec<&'static str>> {
    fn copy_dir_recursive(src: &Path, dest: &Path, depth: u8) -> anyhow::Result<()> {
        if depth > 5 {
            return Ok(());
        }
        std::fs::create_dir_all(dest)?;
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            let path = entry.path();
            let dest_path = dest.join(entry.file_name());
            if path.is_dir() {
                copy_dir_recursive(&path, &dest_path, depth + 1)?;
            } else {
                std::fs::copy(&path, &dest_path)?;
            }
        }
        Ok(())
    }

    copy_dir_recursive(src, dest, 0)?;

    let skill_md = dest.join("SKILL.md");
    let fixes = if skill_md.is_file() {
        let content = std::fs::read_to_string(&skill_md)?;
        let (fixed, fixes) = fix_skill_md(&content);
        std::fs::write(&skill_md, fixed)?;
        fixes
    } else {
        Vec::new()
    };
    Ok(fixes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::rules::test_support::agent;

    #[test]
    fn portable_model_maps_concrete_models_to_tiers() {
        assert_eq!(portable_model(Some("opus")), "latest:max");
        assert_eq!(portable_model(Some("claude-sonnet-5")), "latest:pro");
        assert_eq!(portable_model(Some("latest:fast")), "latest:fast");
        assert_eq!(portable_model(None), "latest:pro");
        // Deprecated alias resolved first, then classified.
        assert_eq!(portable_model(Some("gemini-3.0-pro")), "latest:pro");
    }

    #[test]
    fn render_agent_produces_armadai_format() {
        let mut a = agent("reviewer", "You review code.");
        a.metadata.model = Some("opus".to_string());
        a.metadata.extra.insert(
            "paths".into(),
            serde_yaml_ng::Value::String("src/**".into()),
        );
        let md = render_agent(&a);
        assert!(md.starts_with("# reviewer\n"));
        assert!(md.contains("## Metadata"));
        assert!(md.contains("- provider: claude"));
        assert!(md.contains("- model: latest:max"));
        assert!(md.contains("- scope: [src/**]"));
        assert!(md.contains("## System Prompt"));
        assert!(md.contains("You review code."));
    }

    fn block() -> String {
        (1..=10).map(|i| format!("Convention line {i}\n")).collect()
    }

    #[test]
    fn extract_shared_fragment_finds_longest_common_block() {
        let b = block();
        let a1 = agent("g1", &format!("Intro one.\n\n{b}Outro one."));
        let a2 = agent("g2", &format!("{b}Outro two."));
        let refs: Vec<&ImportedAgent> = vec![&a1, &a2];
        let f = extract_shared_fragment(&refs, 0).unwrap();
        assert_eq!(f.name, "shared-conventions-1");
        assert_eq!(f.apply_to, vec!["g1".to_string(), "g2".to_string()]);
        assert!(f.body.contains("Convention line 1"));
        assert!(f.body.contains("Convention line 10"));
        assert!(!f.body.contains("Intro"));
    }

    #[test]
    fn extract_returns_none_below_window() {
        let a1 = agent("s1", "short\ncommon\ntext");
        let a2 = agent("s2", "short\ncommon\ntext");
        let refs: Vec<&ImportedAgent> = vec![&a1, &a2];
        // 3 common lines < 8-line window: not worth a shared fragment.
        assert!(extract_shared_fragment(&refs, 0).is_none());
    }

    #[test]
    fn strip_fragment_removes_block_and_keeps_rest() {
        let b = block();
        let prompt = format!("Intro.\n\n{b}\nOutro.");
        let f_body = b.trim_end().to_string();
        let stripped = strip_fragment(&prompt, &f_body);
        assert!(stripped.contains("Intro."));
        assert!(stripped.contains("Outro."));
        assert!(!stripped.contains("Convention line 5"));
    }

    #[test]
    fn render_prompt_has_frontmatter_and_apply_to() {
        let f = SharedFragment {
            name: "shared-conventions-1".into(),
            apply_to: vec!["g1".into(), "g2".into()],
            body: "Some shared text.".into(),
        };
        let md = render_prompt(&f);
        assert!(md.starts_with("---\n"));
        assert!(md.contains("apply_to:\n  - g1\n  - g2"));
        assert!(md.contains("Some shared text."));
    }

    #[test]
    fn fix_skill_md_renames_tools_and_quotes_colons() {
        let content = "---\nname: triage\ndescription: triage is HUMAN — the skill : just assists\ntools: Read, Grep\n---\nBody";
        let (fixed, fixes) = fix_skill_md(content);
        assert!(fixed.contains("allowed-tools: Read, Grep"));
        assert!(fixed.contains("description: \"triage is HUMAN — the skill : just assists\""));
        assert!(!fixed.contains("\ntools:"));
        assert_eq!(fixes.len(), 2);
    }

    #[test]
    fn fix_skill_md_leaves_clean_files_alone() {
        let content = "---\nname: ok\ndescription: fine\nallowed-tools: Read\n---\nBody";
        let (fixed, fixes) = fix_skill_md(content);
        assert_eq!(fixed, content);
        assert!(fixes.is_empty());
    }

    #[test]
    fn copy_skill_dir_copies_recursively_and_fixes() {
        let src = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(src.path().join("references")).unwrap();
        std::fs::write(
            src.path().join("SKILL.md"),
            "---\nname: s\ndescription: d\ntools: Read\n---\nBody",
        )
        .unwrap();
        std::fs::write(src.path().join("references/ref.md"), "ref").unwrap();
        let dest = tempfile::tempdir().unwrap();
        let dest_dir = dest.path().join("s");
        let fixes = copy_skill_dir(src.path(), &dest_dir).unwrap();
        assert_eq!(fixes, vec!["tools->allowed-tools"]);
        assert!(dest_dir.join("references/ref.md").exists());
        let skill = std::fs::read_to_string(dest_dir.join("SKILL.md")).unwrap();
        assert!(skill.contains("allowed-tools: Read"));
    }
}
