//! Generation of an ArmadAI proposal pack from imported native configs.
use std::fmt::Write as _;
use std::path::Path;

use super::reverse::{ImportedAgent, ImportedConfig};
use crate::linker::model_aliases::resolve_alias;
use crate::linker::model_resolution::{
    classify_model_tier, is_latest_placeholder, tier_placeholder,
};

/// Map a native model to a portable ArmadAI tier when possible.
pub(crate) fn portable_model(model: Option<&str>) -> String {
    let Some(model) = model else {
        return "latest:pro".to_string();
    };
    if model.trim().eq_ignore_ascii_case("inherit") {
        // "inherit" means "use whatever the host CLI defaults to" — there is
        // no portable equivalent, so fall back to the same default as "no
        // model specified".
        return "latest:pro".to_string();
    }
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

/// Collapse any whitespace (incl. newlines) to single spaces so a description
/// is safe on one blockquote line and one `- description:` metadata line.
fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The ArmadAI agent format terminates a section at every markdown heading,
/// so a system prompt cannot contain headings. Native Claude Code prompts are
/// often full markdown documents with headings (including literal `## Metadata`
/// / `## System Prompt` lines that would otherwise clobber the agent's real
/// sections on re-parse). Demote ATX headings to bold text: content and visual
/// emphasis are preserved, and no heading survives to break section parsing.
///
/// Fenced code blocks (``` or ~~~) are left completely untouched — a line
/// starting with `#` inside a fence is code, not a heading. Setext headings
/// (`Title` followed by a line of only `=` or only `-`) are handled too:
/// they truncate the ArmadAI parser's System Prompt section just as ATX
/// headings do, so the underline is dropped and the title line is bolded in
/// place, unless the previous line doesn't look like a plain paragraph (e.g.
/// it is blank, a list/quote/table line, or already a heading/underline) —
/// in that case a `---`/`===` line is left alone as a real thematic break.
fn demote_headings(prompt: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut in_fence = false;

    for line in prompt.lines() {
        let trimmed = line.trim();

        // Fenced code blocks: toggle state, never transform their contents.
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            out.push(line.to_string());
            continue;
        }
        if in_fence {
            out.push(line.to_string());
            continue;
        }

        // Setext heading: current line is all '=' or all '-', and the
        // previously emitted line is a plain paragraph line.
        let is_all_eq = !trimmed.is_empty() && trimmed.chars().all(|c| c == '=');
        let is_all_dash = !trimmed.is_empty() && trimmed.chars().all(|c| c == '-');
        if is_all_eq || is_all_dash {
            if let Some(prev) = out.last() {
                let is_plain_paragraph = !prev.is_empty()
                    && !prev.starts_with(char::is_whitespace)
                    && !prev.starts_with(['#', '-', '*', '>', '|'])
                    && !prev.chars().all(|c| c == '=')
                    && !prev.chars().all(|c| c == '-');
                if is_plain_paragraph {
                    let title = prev.trim().to_string();
                    if let Some(last) = out.last_mut() {
                        *last = format!("**{title}**");
                    }
                    continue; // drop the underline line
                }
            }
            out.push(line.to_string());
            continue;
        }

        // ATX heading: 1-6 leading '#', then a space, then the title.
        let trimmed_start = line.trim_start();
        if let Some(rest) = trimmed_start.strip_prefix('#') {
            let hashes = 1 + rest.chars().take_while(|&c| c == '#').count();
            let after = &trimmed_start[hashes..];
            if hashes <= 6 && after.starts_with(' ') {
                let title = after.trim().trim_end_matches('#').trim();
                if !title.is_empty() {
                    out.push(format!("**{title}**"));
                    continue;
                }
            }
        }
        out.push(line.to_string());
    }

    out.join("\n")
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
    // Native descriptions can be multiline (e.g. YAML `description: |` blocks);
    // collapse to one line so it stays a single blockquote / metadata line.
    let description = one_line(description);
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
    let _ = writeln!(md, "{}", demote_headings(&agent.system_prompt));
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
/// and turn it into a named `SharedFragment`. `slugs` must be parallel to
/// `agents` and contain each member's final agent slug: the fragment's
/// `apply_to` must reference slugs (matched against the generated agent's
/// H1/name), not raw display names, or runtime prompt matching
/// (`core::prompt`) and pack.yaml validation (R5, matched against
/// pack.yaml's agent slug list) would disagree on what an agent is called.
/// Returns `None` when no common block reaches the minimum window.
pub(crate) fn extract_shared_fragment(
    agents: &[&ImportedAgent],
    slugs: &[&str],
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
        apply_to: slugs.iter().map(|s| s.to_string()).collect(),
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

/// Return a slug guaranteed not to already be in `used`. On collision,
/// appends `-2`, `-3`, ... until a free slug is found, then reserves it.
/// Falls back to `fallback` (e.g. `"agent"` / `"skill"`) when `base` is
/// empty — e.g. a fully non-Latin name slugifies to `""` — so we never
/// write a bare `.md` file or an empty pack.yaml entry.
fn unique_slug(
    base: String,
    used: &mut std::collections::HashSet<String>,
    fallback: &str,
) -> String {
    let base = if base.is_empty() {
        fallback.to_string()
    } else {
        base
    };
    if used.insert(base.clone()) {
        return base;
    }
    let mut n = 2;
    loop {
        let candidate = format!("{base}-{n}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
        n += 1;
    }
}

/// Outcome of a successful `armadai audit --propose` run.
#[derive(Debug, Clone)]
pub struct ProposalSummary {
    pub out_dir: std::path::PathBuf,
    pub agents: usize,
    pub prompts: usize,
    pub skills: usize,
    pub skill_fixes: usize,
    pub skipped_agents: Vec<String>,
}

/// Prompts above this size are excluded from duplication-cluster extraction
/// (guard against pathological inputs).
const MAX_PROMPT_CHARS_FOR_EXTRACTION: usize = 100_000;

/// Generate an installable ArmadAI proposal pack under `root/.armadai-proposal`
/// from the assets imported by `import_surfaces`. Never overwrites an
/// existing proposal, and never ships a pack that fails `pack_validation`.
pub fn generate_proposal(root: &Path, config: &ImportedConfig) -> anyhow::Result<ProposalSummary> {
    let out_dir = root.join(".armadai-proposal");
    if out_dir.exists() {
        anyhow::bail!(
            "{} already exists — remove it first (the proposal never overwrites)",
            out_dir.display()
        );
    }
    // Convertible agents: salvage must have recovered the essentials.
    let mut skipped_agents = Vec::new();
    let convertible: Vec<&ImportedAgent> = config
        .agents
        .iter()
        .filter(|a| {
            let ok = !a.name.is_empty() && !a.system_prompt.is_empty();
            if !ok {
                skipped_agents.push(a.name.clone());
            }
            ok
        })
        .collect();

    let owned: Vec<ImportedAgent> = convertible.iter().map(|a| (*a).clone()).collect();

    // Compute each agent's final slug once, disambiguating collisions (e.g. a
    // `name:` frontmatter and a filename-stem fallback both slugifying to the
    // same value). Both the fragment `apply_to` lists and the agent-write
    // loop reuse this exact pairing: the generated agent's H1/name is set to
    // its slug (not its raw display name), because runtime prompt matching
    // (`core::prompt`) matches `apply_to` against the H1 name while pack.yaml
    // validation (R5) matches it against the pack's slug list — the two can
    // only agree if H1 == slug.
    let mut used_agent_slugs = std::collections::HashSet::new();
    let agent_slugs: Vec<String> = owned
        .iter()
        .map(|a| {
            unique_slug(
                crate::linker::slugify(&a.name),
                &mut used_agent_slugs,
                "agent",
            )
        })
        .collect();

    // Shared fragments from duplication clusters (guard very large prompts).
    let clusters = crate::audit::rules::duplication_clusters(&owned);
    let mut fragments: Vec<SharedFragment> = Vec::new();
    for (i, members) in clusters.iter().enumerate() {
        let refs: Vec<&ImportedAgent> = members.iter().map(|&m| &owned[m]).collect();
        if refs
            .iter()
            .any(|a| a.system_prompt.len() > MAX_PROMPT_CHARS_FOR_EXTRACTION)
        {
            continue;
        }
        let slugs: Vec<&str> = members.iter().map(|&m| agent_slugs[m].as_str()).collect();
        if let Some(f) = extract_shared_fragment(&refs, &slugs, i) {
            fragments.push(f);
        }
    }

    std::fs::create_dir_all(out_dir.join("agents"))?;
    // Agents: render with their shared fragments stripped out. The clone's
    // name is set to its slug BEFORE the strip loop so `apply_to` (slugs)
    // matches, and BEFORE `render_agent` so the H1 is the slug too. The
    // original display name stays visible via `metadata.description`.
    for (a, slug) in owned.iter().zip(agent_slugs.iter()) {
        let mut agent = a.clone();
        agent.name = slug.clone();
        for f in &fragments {
            if f.apply_to.iter().any(|n| n == slug) {
                agent.system_prompt = strip_fragment(&agent.system_prompt, &f.body);
            }
        }
        let file = out_dir.join("agents").join(format!("{slug}.md"));
        std::fs::write(file, render_agent(&agent))?;
    }
    // Prompts.
    if !fragments.is_empty() {
        std::fs::create_dir_all(out_dir.join("prompts"))?;
        for f in &fragments {
            std::fs::write(
                out_dir.join("prompts").join(format!("{}.md", f.name)),
                render_prompt(f),
            )?;
        }
    }
    // Skills. Slugs are computed once (same disambiguation strategy as
    // agents) and reused for both the destination directory and the
    // pack.yaml `skills:` list.
    let mut skill_fixes = 0usize;
    let installable_skills: Vec<&super::reverse::ImportedSkill> =
        config.skills.iter().filter(|s| s.has_skill_md).collect();
    let mut used_skill_slugs = std::collections::HashSet::new();
    let skill_slugs: Vec<String> = installable_skills
        .iter()
        .map(|s| {
            unique_slug(
                crate::linker::slugify(&s.name),
                &mut used_skill_slugs,
                "skill",
            )
        })
        .collect();
    if !installable_skills.is_empty() {
        std::fs::create_dir_all(out_dir.join("skills"))?;
        for (s, slug) in installable_skills.iter().zip(skill_slugs.iter()) {
            // source_path points at SKILL.md; the skill dir is its parent.
            let src = s
                .source_path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| s.source_path.clone());
            let dest = out_dir.join("skills").join(slug);
            skill_fixes += copy_skill_dir(&src, &dest)?.len();
        }
    }
    // pack.yaml
    let pack_name = root
        .file_name()
        .map(|n| crate::linker::slugify(&n.to_string_lossy()))
        .unwrap_or_else(|| "imported".to_string());
    let mut pack = String::new();
    let _ = writeln!(pack, "name: {pack_name}-agents");
    let _ = writeln!(
        pack,
        "description: \"Generated by `armadai audit --propose` from the native Claude Code configuration\""
    );
    let _ = writeln!(pack, "agents:");
    for slug in &agent_slugs {
        let _ = writeln!(pack, "  - {slug}");
    }
    if !fragments.is_empty() {
        let _ = writeln!(pack, "prompts:");
        for f in &fragments {
            let _ = writeln!(pack, "  - {}", f.name);
        }
    }
    if !installable_skills.is_empty() {
        let _ = writeln!(pack, "skills:");
        for slug in &skill_slugs {
            let _ = writeln!(pack, "  - {slug}");
        }
    }
    std::fs::write(out_dir.join("pack.yaml"), pack)?;

    // Invariant: the generator never ships an invalid pack.
    let mut errors: Vec<String> = crate::core::pack_validation::validate_pack(&out_dir)
        .into_iter()
        .filter(|i| matches!(i.severity, crate::core::pack_validation::Severity::Error))
        .map(|i| format!("{}: {}", i.location, i.message))
        .collect();

    // Behavioral round-trip gate: pack_validation only checks structure/YAML,
    // not that the real ArmadAI parser can still make sense of each agent
    // (this is the backstop for any future regression in one_line/
    // demote_headings). Every generated agent must re-parse, and must not
    // have lost its whole system prompt in the process.
    for (a, slug) in owned.iter().zip(agent_slugs.iter()) {
        let file = out_dir.join("agents").join(format!("{slug}.md"));
        match crate::parser::parse_agent_file(&file) {
            Ok(parsed) => {
                if parsed.system_prompt.trim().is_empty() && !a.system_prompt.trim().is_empty() {
                    errors.push(format!(
                        "agents/{slug}.md: system prompt became empty on re-parse"
                    ));
                }
            }
            Err(e) => errors.push(format!("agents/{slug}.md: failed to re-parse: {e}")),
        }
    }

    if !errors.is_empty() {
        // Never leave a broken or partial proposal on disk: a leftover
        // `.armadai-proposal/` would also block any retry (the generator
        // refuses to overwrite an existing directory).
        let _ = std::fs::remove_dir_all(&out_dir);
        anyhow::bail!(
            "generated proposal failed validation:\n{}",
            errors.join("\n")
        );
    }

    Ok(ProposalSummary {
        out_dir,
        agents: owned.len(),
        prompts: fragments.len(),
        skills: installable_skills.len(),
        skill_fixes,
        skipped_agents,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::rules::test_support::{agent, config_with};

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
    fn portable_model_treats_inherit_as_default() {
        assert_eq!(portable_model(Some("inherit")), "latest:pro");
        assert_eq!(portable_model(Some("  Inherit  ")), "latest:pro");
        assert_eq!(portable_model(Some("INHERIT")), "latest:pro");
    }

    #[test]
    fn demote_headings_handles_setext_and_skips_fences() {
        let input = "Workflow\n---\nkeep me\n\n```bash\n# not a heading\n```\n\nTitle\n===";
        let out = demote_headings(input);
        assert!(out.contains("**Workflow**"));
        assert!(!out.lines().any(|l| l.trim() == "---")); // setext underline dropped
        assert!(out.contains("keep me"));
        assert!(out.contains("# not a heading")); // untouched inside fence
        assert!(out.contains("**Title**"));
    }

    #[test]
    fn demote_headings_neutralizes_atx_headings() {
        let input = "# Title\n\nsome text\n\n## Metadata\n- provider: x\n\n### Deep\nmore";
        let out = demote_headings(input);
        assert!(!out.contains("# Title"));
        assert!(out.contains("**Title**"));
        assert!(out.contains("**Metadata**"));
        assert!(out.contains("**Deep**"));
        assert!(out.contains("some text"));
        assert!(out.contains("- provider: x")); // list items untouched
    }

    #[test]
    fn render_agent_survives_headings_in_system_prompt() {
        let mut a = agent(
            "doc",
            "# Overview\n\nDoes things.\n\n## Metadata\n- fake: value\n\n## System Prompt\nnested",
        );
        a.metadata.model = Some("latest:pro".to_string());
        let md = render_agent(&a);
        // Write + re-parse through the real ArmadAI parser: metadata must survive.
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("doc.md");
        std::fs::write(&file, &md).unwrap();
        let parsed = crate::parser::parse_agent_file(&file).unwrap();
        assert_eq!(parsed.metadata.provider, "claude");
        assert!(parsed.system_prompt.contains("Does things."));
        assert!(parsed.system_prompt.contains("**Overview**"));
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
        let f = extract_shared_fragment(&refs, &["g1", "g2"], 0).unwrap();
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
        assert!(extract_shared_fragment(&refs, &["s1", "s2"], 0).is_none());
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

    #[test]
    fn generate_proposal_produces_valid_installable_pack() {
        let dir = tempfile::tempdir().unwrap();
        let agents = dir.path().join(".claude/agents");
        std::fs::create_dir_all(&agents).unwrap();
        let block = block();
        std::fs::write(
            agents.join("gate-a.md"),
            format!(
                "---\nname: gate-a\ndescription: Gate A\nmodel: opus\n---\nIntro A.\n\n{block}"
            ),
        )
        .unwrap();
        std::fs::write(
            agents.join("gate-b.md"),
            format!("---\nname: gate-b\ndescription: Gate B\n---\n{block}Outro B."),
        )
        .unwrap();
        let skill = dir.path().join(".claude/skills/deploy");
        std::fs::create_dir_all(&skill).unwrap();
        std::fs::write(
            skill.join("SKILL.md"),
            "---\nname: deploy\ndescription: Deploys\ntools: Read\n---\nSteps.",
        )
        .unwrap();

        let (_, config) = crate::audit::import_surfaces(dir.path());
        let summary = generate_proposal(dir.path(), &config).unwrap();
        assert_eq!(summary.agents, 2);
        assert_eq!(summary.prompts, 1);
        assert_eq!(summary.skills, 1);
        assert_eq!(summary.skill_fixes, 1);

        let out = dir.path().join(".armadai-proposal");
        // The generated agents parse with the real ArmadAI parser.
        for name in ["gate-a", "gate-b"] {
            let parsed =
                crate::parser::parse_agent_file(&out.join(format!("agents/{name}.md"))).unwrap();
            assert_eq!(parsed.metadata.provider, "claude");
        }
        // Shared block was factored out of the agents.
        let a = std::fs::read_to_string(out.join("agents/gate-a.md")).unwrap();
        assert!(!a.contains("Convention line 5"));
        let p = std::fs::read_to_string(out.join("prompts/shared-conventions-1.md")).unwrap();
        assert!(p.contains("Convention line 5"));
        // Second run refuses to overwrite.
        assert!(generate_proposal(dir.path(), &config).is_err());
    }

    #[test]
    fn generate_proposal_disambiguates_colliding_slugs() {
        let dir = tempfile::tempdir().unwrap();
        let agents = dir.path().join(".claude/agents");
        std::fs::create_dir_all(&agents).unwrap();
        std::fs::write(
            agents.join("a1.md"),
            "---\nname: Gate A\ndescription: d\n---\nPrompt one.",
        )
        .unwrap();
        std::fs::write(
            agents.join("gate-a.md"),
            "---\ndescription: d\n---\nPrompt two.",
        )
        .unwrap();
        let (_, config) = crate::audit::import_surfaces(dir.path());
        let summary = generate_proposal(dir.path(), &config).unwrap();
        assert_eq!(summary.agents, 2);
        let out = dir.path().join(".armadai-proposal/agents");
        let mut files: Vec<String> = std::fs::read_dir(&out)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        files.sort();
        assert_eq!(
            files,
            vec!["gate-a-2.md".to_string(), "gate-a.md".to_string()]
        );
        let pack = std::fs::read_to_string(dir.path().join(".armadai-proposal/pack.yaml")).unwrap();
        assert!(pack.contains("- gate-a\n"));
        assert!(pack.contains("- gate-a-2\n"));
    }

    // --- Adversarial round-trip tests (through the REAL parser) ---------

    #[test]
    fn generate_proposal_collapses_multiline_description_without_corrupting_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = agent(
            "multiline-desc",
            "Line one of the prompt.\n\nLine two of the prompt, long enough to matter.",
        );
        // A multiline description containing something that looks like a
        // metadata key on its own line: if emitted verbatim this would be
        // parsed as a bogus `- mode: strict` metadata entry.
        a.metadata.description =
            Some("Handles triage.\n- mode: strict\nAlso reviews PRs.".to_string());
        let config = config_with(vec![a]);

        let summary = generate_proposal(dir.path(), &config).unwrap();
        assert_eq!(summary.agents, 1);

        let out = dir.path().join(".armadai-proposal");
        let parsed =
            crate::parser::parse_agent_file(&out.join("agents/multiline-desc.md")).unwrap();
        assert_eq!(parsed.metadata.provider, "claude");
    }

    #[test]
    fn generate_proposal_setext_heading_does_not_truncate_prompt() {
        let dir = tempfile::tempdir().unwrap();
        let a = agent("setext-agent", "Intro\n\nWorkflow\n---\nSENTINEL_TAIL");
        let config = config_with(vec![a]);

        let summary = generate_proposal(dir.path(), &config).unwrap();
        assert_eq!(summary.agents, 1);

        let out = dir.path().join(".armadai-proposal");
        let parsed = crate::parser::parse_agent_file(&out.join("agents/setext-agent.md")).unwrap();
        assert!(parsed.system_prompt.contains("SENTINEL_TAIL"));
    }

    #[test]
    fn generate_proposal_shared_fragment_apply_to_uses_slugs_not_names() {
        let dir = tempfile::tempdir().unwrap();
        let b = block();
        let a1 = agent("Gate Alpha", &format!("Intro alpha.\n\n{b}Outro alpha."));
        let a2 = agent("Gate Beta", &format!("{b}Outro beta."));
        let config = config_with(vec![a1, a2]);

        let summary = generate_proposal(dir.path(), &config).unwrap();
        assert_eq!(summary.agents, 2);
        assert_eq!(summary.prompts, 1);

        let out = dir.path().join(".armadai-proposal");
        let pack = std::fs::read_to_string(out.join("pack.yaml")).unwrap();
        assert!(pack.contains("- gate-alpha\n"));
        assert!(pack.contains("- gate-beta\n"));

        let prompt = std::fs::read_to_string(out.join("prompts/shared-conventions-1.md")).unwrap();
        assert!(prompt.contains("apply_to:\n  - gate-alpha\n  - gate-beta"));

        // Both generated agents parse and the shared block was stripped.
        for slug in ["gate-alpha", "gate-beta"] {
            let parsed =
                crate::parser::parse_agent_file(&out.join(format!("agents/{slug}.md"))).unwrap();
            assert_eq!(parsed.metadata.provider, "claude");
        }
    }

    #[test]
    fn generate_proposal_inherit_model_becomes_default_tier() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = agent("inherit-agent", "Does the thing on behalf of the user.");
        a.metadata.model = Some("inherit".to_string());
        let config = config_with(vec![a]);

        let summary = generate_proposal(dir.path(), &config).unwrap();
        assert_eq!(summary.agents, 1);

        let out = dir.path().join(".armadai-proposal");
        let content = std::fs::read_to_string(out.join("agents/inherit-agent.md")).unwrap();
        assert!(content.contains("- model: latest:pro"));
    }
}
