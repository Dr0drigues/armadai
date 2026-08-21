use std::path::{Path, PathBuf};

use crate::model_aliases::resolve_alias;
use crate::project::{self, ProjectConfig};

// ---------------------------------------------------------------------------
// Data model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct DeprecationFinding {
    pub agent_path: PathBuf,
    pub agent_name: String,
    pub field: String,
    pub current: String,
    pub replacement: String,
}

// ---------------------------------------------------------------------------
// Check
// ---------------------------------------------------------------------------

/// Check a single agent file for deprecated model references.
pub fn check_agent_file(path: &Path) -> Vec<DeprecationFinding> {
    let agent = match crate::parser::parse_agent_file(path) {
        Ok(a) => a,
        Err(_) => return Vec::new(),
    };

    let mut findings = Vec::new();

    if let Some(ref model) = agent.metadata.model
        && let Some(replacement) = resolve_alias(model)
    {
        findings.push(DeprecationFinding {
            agent_path: path.to_path_buf(),
            agent_name: agent.name.clone(),
            field: "model".to_string(),
            current: model.clone(),
            replacement,
        });
    }

    for (i, fb) in agent.metadata.model_fallback.iter().enumerate() {
        if let Some(replacement) = resolve_alias(fb) {
            findings.push(DeprecationFinding {
                agent_path: path.to_path_buf(),
                agent_name: agent.name.clone(),
                field: format!("model_fallback[{i}]"),
                current: fb.clone(),
                replacement,
            });
        }
    }

    findings
}

/// Check all agents in a project for deprecated models.
///
/// Covers both formats: `.md` files (via [`check_agent_file`]) and, when the
/// project has one, `.armadai/agents.yaml` (via [`check_declarations`]).
/// Without the latter, a declaration file would be the one place in a
/// project where a dead model goes unnoticed while every `.md` around it
/// gets fixed.
pub fn check_project(project_root: &Path) -> anyhow::Result<Vec<DeprecationFinding>> {
    let config = load_project_config(project_root)?;
    let (paths, _errors) = project::resolve_all_agents(&config, project_root);

    let mut all_findings = Vec::new();
    for path in &paths {
        all_findings.extend(check_agent_file(path));
    }

    let decls_path = crate::agent_source::declarations_path(project_root);
    if decls_path.is_file() {
        all_findings.extend(check_declarations(&decls_path));
    }

    Ok(all_findings)
}

// ---------------------------------------------------------------------------
// Update
// ---------------------------------------------------------------------------

/// Update deprecated models in an agent file in-place.
/// Returns the number of replacements made.
pub fn update_agent_file(path: &Path, findings: &[DeprecationFinding]) -> anyhow::Result<usize> {
    if findings.is_empty() {
        return Ok(0);
    }

    let mut content = std::fs::read_to_string(path)?;
    let mut count = 0;

    for finding in findings {
        // Replace `model: <old>` patterns (handles both model and model_fallback values)
        let old_pattern = format!(": {}", finding.current);
        let new_pattern = format!(": {}", finding.replacement);

        if content.contains(&old_pattern) {
            content = content.replacen(&old_pattern, &new_pattern, 1);
            count += 1;
        }

        // Also handle fallback values that appear as list items: `  - <old>`
        let old_list_item = format!("- {}", finding.current);
        let new_list_item = format!("- {}", finding.replacement);
        if content.contains(&old_list_item) {
            content = content.replacen(&old_list_item, &new_list_item, 1);
            // Only count if we didn't already count from the `: ` pattern
            if count == 0 {
                count += 1;
            }
        }
    }

    if count > 0 {
        std::fs::write(path, content)?;
    }

    Ok(count)
}

// ---------------------------------------------------------------------------
// Declarative agents (`.armadai/agents.yaml`)
// ---------------------------------------------------------------------------

/// Deprecated models declared in an `agents.yaml`.
///
/// One finding **per occurrence**: unlike a `.md` agent, which declares
/// `model` once, a declaration file carries it in `defaults` and in every
/// agent that deviates. `field` distinguishes them (`defaults.model`,
/// `<agent>.model`, `<agent>.model_fallback[i]`) so the rewrite can target
/// each one.
///
/// Detection walks the parsed declaration, so it can only ever see real
/// `model` / `model_fallback` values — never prose in a comment or a
/// `description`.
pub fn check_declarations(path: &Path) -> Vec<DeprecationFinding> {
    let Ok(decls) = crate::agent_decl::load(path) else {
        return Vec::new(); // unreadable: not this function's problem
    };
    let mut out = Vec::new();
    let mut push = |field: String, agent_name: String, current: &str| {
        if let Some(replacement) = resolve_alias(current) {
            out.push(DeprecationFinding {
                agent_path: path.to_path_buf(),
                agent_name,
                field,
                current: current.to_string(),
                replacement,
            });
        }
    };
    if let Some(m) = &decls.defaults.model {
        push("defaults.model".into(), "defaults".into(), m);
    }
    for (i, fb) in decls.defaults.model_fallback.iter().enumerate() {
        push(
            format!("defaults.model_fallback[{i}]"),
            "defaults".into(),
            fb,
        );
    }
    for a in &decls.agents {
        if let Some(m) = &a.model {
            push(format!("{}.model", a.name), a.name.clone(), m);
        }
        for (i, fb) in a.model_fallback.iter().flatten().enumerate() {
            push(
                format!("{}.model_fallback[{i}]", a.name),
                a.name.clone(),
                fb,
            );
        }
    }
    out
}

/// Whether `value` (a line's content right after its `key:`, already
/// trimmed) opens a YAML block scalar — `|` or `>`, optionally followed by
/// a chomping indicator (`+`/`-`) and/or an explicit indentation indicator
/// (`1`-`9`), the two in either order (`c-b-block-header` in the YAML
/// spec: `|-`, `|+`, `|2`, `|2-`, `|-2`, and the `>` folded-scalar
/// equivalents). A trailing comment after the header is allowed and
/// ignored.
fn opens_block_scalar(value: &str) -> bool {
    let value = value.split('#').next().unwrap_or(value).trim();
    let Some(rest) = value.strip_prefix(['|', '>']) else {
        return false;
    };
    if rest.len() > 2 {
        return false;
    }
    let digits = rest.chars().filter(|c| c.is_ascii_digit()).count();
    let signs = rest.chars().filter(|c| *c == '+' || *c == '-').count();
    digits <= 1 && signs <= 1 && digits + signs == rest.chars().count()
}

/// Rewrite deprecated models in an `agents.yaml`.
///
/// Textual substitution, like `update_agent_file` — a `serde_yaml_ng`
/// round-trip would silently drop every comment and reorder keys.
///
/// Bounded to lines whose key is `model:`/`model_fallback:`, or which are
/// list items belonging to one of those keys' own block list. A raw
/// `: <model>` pattern would also match inside a comment or a
/// `description`, and correcting a configuration must not rewrite prose.
///
/// A bare `- ` line is only a candidate while it is at or below the
/// indentation of the nearest preceding `model:`/`model_fallback:` key that
/// opened a block list (i.e. a line that is *only* that key, with the
/// values on the following, indented lines) — any other line, whatever its
/// own indentation, closes that scope. Without tracking the enclosing key,
/// any `- ` line anywhere would be a candidate: a declared agent's `args:`
/// block list happens to be able to hold a string that is itself a model
/// name (e.g. `args: [--model, claude-3-sonnet-20240229]`, passed through
/// to a `cli`-provider agent), and rewriting that would silently corrupt an
/// argument, not fix a model.
///
/// The same enclosing-scope tracking covers block scalars (`description:
/// |`, `>`, …): every line more indented than the key that opened one (or
/// blank) belongs to that scalar's *value*, never to the mapping — `#` is
/// literal there, not a comment, and a line that happens to start with
/// `model:` is prose, not a key. Such lines are never rewrite candidates,
/// whatever they start with.
///
/// Detection ([`check_declarations`]) is a structured YAML parse; this
/// rewrite is textual, so the two can disagree on a form the parser
/// accepts but this scan does not recognise (e.g. a quoted `"model":` key).
/// Rather than let that surface as a silently-too-low count, this function
/// insists the number of textual replacements it actually made equals
/// `findings.len()` and returns an `Err` naming whatever finding(s) it
/// could not locate otherwise — and, so a caller never has to reason about
/// a half-applied fix, writes nothing at all when it errors.
pub fn update_declarations(path: &Path, findings: &[DeprecationFinding]) -> anyhow::Result<usize> {
    if findings.is_empty() {
        return Ok(0);
    }
    let content = std::fs::read_to_string(path)?;
    let mut count = 0;
    let mut out = String::with_capacity(content.len());
    // Indentation of the nearest `model:`/`model_fallback:` key that opened
    // a block list still in scope, if any.
    let mut active_list_indent: Option<usize> = None;
    // Indentation of the key that opened a block scalar (`|`/`>`) still in
    // scope, if any. Tracked separately: a block scalar can open under ANY
    // key (`description: |`), not just `model:`/`model_fallback:`.
    let mut block_scalar_indent: Option<usize> = None;
    // How many findings still need a textual match, grouped by the exact
    // deprecated value — several findings (e.g. `defaults.model` and an
    // agent's own `model`) legitimately share the same `current` string, so
    // this is decremented, not looked up by finding identity.
    let mut remaining: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for f in findings {
        *remaining.entry(f.current.as_str()).or_insert(0) += 1;
    }

    for line in content.lines() {
        let raw_indent = line.len() - line.trim_start().len();
        let is_blank = line.trim().is_empty();

        if let Some(bs_indent) = block_scalar_indent {
            if is_blank || raw_indent > bs_indent {
                // Still inside the scalar's value: copy through untouched,
                // whatever this line looks like.
                out.push_str(line);
                out.push('\n');
                continue;
            }
            block_scalar_indent = None; // dedented: the scalar just ended
        }

        let code = line.split('#').next().unwrap_or(line);
        let trimmed = code.trim_start();
        let indent = code.len() - trimmed.len();

        let is_model_line = trimmed.starts_with("model:") || trimmed.starts_with("model_fallback:");
        let is_active_list_item = trimmed.starts_with("- ")
            && active_list_indent.is_some_and(|key_indent| indent >= key_indent);

        if !trimmed.is_empty() && !is_active_list_item {
            // Not a continuation of the active list: a bare `model:`/
            // `model_fallback:` key (no inline value) opens a new one — its
            // values are the following, more-indented `- ` lines — and any
            // other line closes whatever scope was open.
            active_list_indent =
                (trimmed == "model:" || trimmed == "model_fallback:").then_some(indent);
        }

        // Does THIS key's own value open a block scalar? Checked for every
        // key, not just model/model_fallback — `description: |` is the
        // case this exists for.
        if let Some(colon) = trimmed.find(':')
            && opens_block_scalar(&trimmed[colon + 1..])
        {
            block_scalar_indent = Some(indent);
        }

        let is_model_key = is_model_line || is_active_list_item;
        let mut kept = line.to_string();
        if is_model_key {
            for f in findings {
                if kept.contains(&f.current) {
                    kept = kept.replace(&f.current, &f.replacement);
                    count += 1;
                    if let Some(n) = remaining.get_mut(f.current.as_str()) {
                        *n = n.saturating_sub(1);
                    }
                }
            }
        }
        out.push_str(&kept);
        out.push('\n');
    }

    if count != findings.len() {
        // Attribute the shortfall to specific findings, deterministically:
        // for each value still outstanding, the trailing findings that
        // share it (findings sharing a `current` are textually
        // interchangeable, so which exact one is named does not matter —
        // only that the named count matches the real shortfall).
        let mut left = remaining;
        let mut unapplied: Vec<&DeprecationFinding> = Vec::new();
        for f in findings.iter().rev() {
            if let Some(n) = left.get_mut(f.current.as_str())
                && *n > 0
            {
                *n -= 1;
                unapplied.push(f);
            }
        }
        unapplied.reverse();
        let named = if unapplied.is_empty() {
            "unable to attribute the mismatch to a specific finding".to_string()
        } else {
            unapplied
                .iter()
                .map(|f| format!("{} [{}]", f.agent_name, f.field))
                .collect::<Vec<_>>()
                .join(", ")
        };
        anyhow::bail!(
            "detection and rewrite disagree on {}: parsing found {} deprecated model(s), the \
             textual rewrite could only locate {} — left everything unfixed rather than guess \
             (an unsupported form, such as a quoted `\"model\":` key, is the likely cause): {named}",
            path.display(),
            findings.len(),
            count,
        );
    }

    std::fs::write(path, out)?;
    Ok(count)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn load_project_config(project_root: &Path) -> anyhow::Result<ProjectConfig> {
    let config_path = project_root.join(".armadai").join("config.yaml");
    if config_path.is_file() {
        return ProjectConfig::load(&config_path);
    }

    for name in &["armadai.yaml", "armadai.yml"] {
        let path = project_root.join(name);
        if path.is_file() {
            return ProjectConfig::load(&path);
        }
    }

    anyhow::bail!("No project config found in {}", project_root.display())
}

// ---------------------------------------------------------------------------
// Auto-check & interactive prompt
// ---------------------------------------------------------------------------

/// Auto-check deprecated models in a project and optionally prompt for update.
///
/// - If `interactive` is true and deprecations found: prompt user with dialoguer::Confirm
/// - If `interactive` is false: print hint to stderr
///
/// Returns true if models were updated.
pub fn auto_check_and_prompt(project_root: &Path, interactive: bool) -> bool {
    let findings = match check_project(project_root) {
        Ok(f) if !f.is_empty() => f,
        _ => return false,
    };

    // Print summary to stderr
    eprintln!("\nhint: {} deprecated model(s) found:", findings.len());
    for f in &findings {
        eprintln!(
            "  {} [{}]: {} -> {}",
            f.agent_name, f.field, f.current, f.replacement
        );
    }

    if !interactive {
        eprintln!("hint: run `armadai models update` to fix.\n");
        return false;
    }

    // Interactive prompt
    let confirm = dialoguer::Confirm::new()
        .with_prompt("Update deprecated models now?")
        .default(true)
        .interact()
        .unwrap_or(false);

    if confirm {
        let decls_path = crate::agent_source::declarations_path(project_root);
        let mut total = 0;
        for f in &findings {
            if let Ok(n) = apply_finding(f, &decls_path) {
                total += n;
            }
        }
        if total > 0 {
            eprintln!("  Updated {total} model(s).\n");
        }
        return true;
    }

    eprintln!("hint: run `armadai models update` when ready.\n");
    false
}

/// Rewrite one finding in place, choosing the rewriter that matches where it
/// came from.
///
/// A finding whose `agent_path` is the project's `agents.yaml` came from
/// [`check_declarations`] and must go through [`update_declarations`] —
/// [`update_agent_file`]'s single `replacen(.., 1)` would only fix the first
/// occurrence, and its raw `: <model>` pattern is not bounded to
/// `model:`/`model_fallback:` lines the way the declarative rewrite is.
///
/// Public, and used by two callers: `auto_check_and_prompt`'s own
/// confirmation branch below, and `armadai models update`
/// (`crates/armadai/src/cli/models.rs`) — the other place that turns a
/// [`DeprecationFinding`] into an actual file edit. Both need the exact same
/// routing decision; extracted here once so neither can drift from it (and
/// so the decision itself has a direct test, independent of the interactive
/// prompt one of its callers sits behind).
pub fn apply_finding(finding: &DeprecationFinding, decls_path: &Path) -> anyhow::Result<usize> {
    if finding.agent_path == decls_path {
        update_declarations(&finding.agent_path, std::slice::from_ref(finding))
    } else {
        update_agent_file(&finding.agent_path, std::slice::from_ref(finding))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn create_agent_md(name: &str, model: &str, fallbacks: &[&str]) -> String {
        let mut md =
            format!("# {name}\n\n## Metadata\n\n```yaml\nprovider: anthropic\nmodel: {model}\n");
        if !fallbacks.is_empty() {
            let fb_str = fallbacks.join(", ");
            md.push_str(&format!("model_fallback: [{fb_str}]\n"));
        }
        md.push_str("```\n\n## System Prompt\n\nYou are a helpful assistant.\n");
        md
    }

    #[test]
    fn test_check_no_deprecation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.md");
        std::fs::write(&path, create_agent_md("Test", "claude-sonnet-4-5", &[])).unwrap();

        let findings = check_agent_file(&path);
        assert!(findings.is_empty());
    }

    #[test]
    fn test_check_deprecated_model() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.md");
        std::fs::write(&path, create_agent_md("Test", "gpt-4-turbo", &[])).unwrap();

        let findings = check_agent_file(&path);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].field, "model");
        assert_eq!(findings[0].current, "gpt-4-turbo");
        assert_eq!(findings[0].replacement, "gpt-4o");
    }

    #[test]
    fn test_check_deprecated_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.md");
        std::fs::write(
            &path,
            create_agent_md("Test", "claude-sonnet-4-5", &["gemini-3.0-pro"]),
        )
        .unwrap();

        let findings = check_agent_file(&path);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].field, "model_fallback[0]");
        assert_eq!(findings[0].current, "gemini-3.0-pro");
        assert_eq!(findings[0].replacement, "latest:pro");
    }

    #[test]
    fn test_update_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.md");
        let original = create_agent_md("Test", "gpt-4-turbo", &[]);
        std::fs::write(&path, &original).unwrap();

        let findings = check_agent_file(&path);
        let count = update_agent_file(&path, &findings).unwrap();
        assert_eq!(count, 1);

        let updated = std::fs::read_to_string(&path).unwrap();
        assert!(updated.contains("model: gpt-4o"));
        assert!(!updated.contains("model: gpt-4-turbo"));
    }

    #[test]
    fn test_update_preserves_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.md");
        let original = create_agent_md("My Agent", "gpt-4-turbo", &[]);
        std::fs::write(&path, &original).unwrap();

        let findings = check_agent_file(&path);
        update_agent_file(&path, &findings).unwrap();

        let updated = std::fs::read_to_string(&path).unwrap();
        assert!(updated.contains("# My Agent"));
        assert!(updated.contains("provider: anthropic"));
        assert!(updated.contains("You are a helpful assistant."));
    }

    #[test]
    fn test_update_no_findings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.md");
        let original = create_agent_md("Test", "claude-sonnet-4-5", &[]);
        std::fs::write(&path, &original).unwrap();

        let count = update_agent_file(&path, &[]).unwrap();
        assert_eq!(count, 0);

        // Content unchanged
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, original);
    }

    #[test]
    fn test_check_project_with_agents() {
        let dir = tempfile::tempdir().unwrap();

        // Create project config
        let armadai_dir = dir.path().join(".armadai");
        std::fs::create_dir_all(armadai_dir.join("agents")).unwrap();
        std::fs::write(
            armadai_dir.join("config.yaml"),
            "agents:\n  - name: test-agent\n",
        )
        .unwrap();

        // Create agent with deprecated model
        std::fs::write(
            armadai_dir.join("agents").join("test-agent.md"),
            create_agent_md("Test Agent", "gpt-3.5-turbo", &[]),
        )
        .unwrap();

        let findings = check_project(dir.path()).unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].current, "gpt-3.5-turbo");
        assert_eq!(findings[0].replacement, "gpt-4o-mini");
    }

    // -----------------------------------------------------------------
    // Declarative agents (`.armadai/agents.yaml`)
    // -----------------------------------------------------------------

    /// Take a real deprecated model from the alias registry, so the test
    /// does not encode a value that may stop being deprecated.
    ///
    /// The two Claude names are tried first, in case the registry ever
    /// grows Anthropic entries, but `embedded_aliases()` (in
    /// `model_aliases.rs`) carries none today — verified by reading it, and
    /// by this helper actually panicking on an unpatched version of itself
    /// that tried only those two. The fallback candidates ARE in that map
    /// today, so this stays deterministic without depending on a machine's
    /// optional `~/.config/armadai/model-aliases.json` override.
    fn a_deprecated_model() -> (String, String) {
        for candidate in [
            "claude-3-sonnet-20240229",
            "claude-3-opus-20240229",
            "gpt-4-turbo",
            "gpt-3.5-turbo",
            "gemini-1.5-flash",
            "gemini-3.0-pro",
        ] {
            if let Some(r) = resolve_alias(candidate) {
                return (candidate.to_string(), r);
            }
        }
        panic!("no known deprecated model in the alias registry — update this helper");
    }

    fn write_yaml(dir: &Path, body: &str) -> std::path::PathBuf {
        let p = dir.join("agents.yaml");
        std::fs::write(&p, body).unwrap();
        p
    }

    #[test]
    fn a_deprecated_model_in_defaults_is_found_and_fixed() {
        let (old, new) = a_deprecated_model();
        let dir = tempfile::tempdir().unwrap();
        let p = write_yaml(
            dir.path(),
            &format!("defaults:\n  model: {old}\nagents:\n  - name: a\n"),
        );
        let findings = check_declarations(&p);
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(update_declarations(&p, &findings).unwrap(), 1);
        let after = std::fs::read_to_string(&p).unwrap();
        assert!(after.contains(&new) && !after.contains(&old));
    }

    /// A `.md` agent declares `model` once; `agents.yaml` carries it in
    /// `defaults` and in every agent that deviates. `replacen(.., 1)` would
    /// fix only the first.
    #[test]
    fn the_same_deprecated_model_is_fixed_at_every_occurrence() {
        let (old, new) = a_deprecated_model();
        let dir = tempfile::tempdir().unwrap();
        let p = write_yaml(
            dir.path(),
            &format!(
                "defaults:\n  model: {old}\nagents:\n  \
                 - name: a\n    model: {old}\n  - name: b\n    model: {old}\n"
            ),
        );
        let findings = check_declarations(&p);
        assert_eq!(findings.len(), 3, "one per occurrence: {findings:?}");
        update_declarations(&p, &findings).unwrap();
        let after = std::fs::read_to_string(&p).unwrap();
        assert!(
            !after.contains(&old),
            "every occurrence must be fixed:\n{after}"
        );
        assert_eq!(after.matches(&new).count(), 3);
    }

    /// A raw `: <model>` pattern would also match inside prose. Correcting a
    /// configuration is one thing; rewriting a comment is another.
    #[test]
    fn a_deprecated_model_named_in_a_comment_or_description_is_left_alone() {
        let (old, _new) = a_deprecated_model();
        let dir = tempfile::tempdir().unwrap();
        let p = write_yaml(
            dir.path(),
            &format!(
                "# we used to run {old} here\nagents:\n  - name: a\n    \
                 description: migrated away from {old}\n"
            ),
        );
        assert!(
            check_declarations(&p).is_empty(),
            "no `model:` key carries it, so there is nothing to fix"
        );
        let before = std::fs::read_to_string(&p).unwrap();
        update_declarations(&p, &[]).unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), before);
    }

    #[test]
    fn comments_and_key_order_survive_the_rewrite() {
        let (old, new) = a_deprecated_model();
        let dir = tempfile::tempdir().unwrap();
        let p = write_yaml(
            dir.path(),
            &format!(
                "# fleet defaults\ndefaults:\n  model: {old}\n  \
                 temperature: 0.3   # deliberately warm\nagents:\n  - name: a\n"
            ),
        );
        let findings = check_declarations(&p);
        update_declarations(&p, &findings).unwrap();
        let after = std::fs::read_to_string(&p).unwrap();
        // The rewrite must actually have happened — otherwise "comments and
        // key order survive" would pass just as well for a no-op that never
        // touched the file at all.
        assert!(
            after.contains(&new) && !after.contains(&old),
            "the model must actually be rewritten:\n{after}"
        );
        assert!(after.contains("# fleet defaults"), "comment lost:\n{after}");
        assert!(
            after.contains("# deliberately warm"),
            "inline comment lost:\n{after}"
        );
        assert!(
            after.find("model:").unwrap() < after.find("temperature:").unwrap(),
            "key order changed — a serde round-trip would do this:\n{after}"
        );
    }

    #[test]
    fn a_deprecated_model_in_model_fallback_is_fixed() {
        let (old, new) = a_deprecated_model();
        let dir = tempfile::tempdir().unwrap();
        let p = write_yaml(
            dir.path(),
            &format!("agents:\n  - name: a\n    model_fallback: [{old}]\n"),
        );
        let findings = check_declarations(&p);
        assert_eq!(findings.len(), 1, "{findings:?}");
        update_declarations(&p, &findings).unwrap();
        assert!(std::fs::read_to_string(&p).unwrap().contains(&new));
    }

    /// `is_model_key` must not treat every `- ` line as a candidate. An
    /// `args:` block list — added alongside `command` so a declared
    /// `provider: cli` agent can be expressed — can perfectly well hold a
    /// value that is itself a deprecated model name, e.g. `--model
    /// <name>` passed straight through to the CLI. Rewriting that would
    /// corrupt an argument instead of fixing a model, while the real
    /// `model:` field on the same agent must still be fixed.
    #[test]
    fn a_list_item_under_an_unrelated_key_is_left_alone() {
        let (old, new) = a_deprecated_model();
        let dir = tempfile::tempdir().unwrap();
        let p = write_yaml(
            dir.path(),
            &format!(
                "agents:\n  - name: a\n    provider: cli\n    model: {old}\n    \
                 args:\n      - --model\n      - {old}\n"
            ),
        );
        let findings = check_declarations(&p);
        assert_eq!(
            findings.len(),
            1,
            "only the `model:` field is a finding: {findings:?}"
        );
        update_declarations(&p, &findings).unwrap();
        let after = std::fs::read_to_string(&p).unwrap();
        assert!(
            after.contains(&format!("model: {new}")),
            "the real model field must still be fixed:\n{after}"
        );
        assert!(
            after.contains(&format!("- {old}")),
            "the args list item must survive untouched:\n{after}"
        );
    }

    /// `model_fallback` written as a block list (each value on its own,
    /// indented `- ` line) rather than the inline `[a, b]` form used
    /// elsewhere in this file — both are valid YAML, and both must be
    /// fixed. Two distinct deprecated models (rather than
    /// `a_deprecated_model`'s single pair) so a mutation that only fixes
    /// the first occurrence cannot hide behind two identical strings.
    #[test]
    fn a_block_style_model_fallback_list_is_fixed() {
        let dir = tempfile::tempdir().unwrap();
        let p = write_yaml(
            dir.path(),
            "agents:\n  - name: a\n    model_fallback:\n      - gpt-4-turbo\n      \
             - gemini-1.5-flash\n",
        );
        let findings = check_declarations(&p);
        assert_eq!(findings.len(), 2, "{findings:?}");
        assert_eq!(update_declarations(&p, &findings).unwrap(), 2);
        let after = std::fs::read_to_string(&p).unwrap();
        assert!(after.contains("gpt-4o") && !after.contains("gpt-4-turbo"));
        assert!(after.contains("gemini-2.5-flash") && !after.contains("gemini-1.5-flash"));
    }

    /// A `description: |` block scalar whose indented continuation line
    /// itself starts with `model:` must not be mistaken for a real `model:`
    /// key — it is prose inside the scalar's value, not a mapping key. The
    /// sibling agent's real `model:` field, outside the scalar, must still
    /// be fixed.
    #[test]
    fn a_block_scalar_is_left_byte_identical_while_a_real_model_field_is_still_fixed() {
        let (old, new) = a_deprecated_model();
        let dir = tempfile::tempdir().unwrap();
        let body = format!(
            "agents:\n  - name: a\n    description: |\n      notes:\n      model: {old}\n      \
             more prose about {old}\n    model: {old}\n"
        );
        let p = write_yaml(dir.path(), &body);

        let findings = check_declarations(&p);
        assert_eq!(
            findings.len(),
            1,
            "only the real `model:` field is a finding: {findings:?}"
        );
        assert_eq!(update_declarations(&p, &findings).unwrap(), 1);

        let after = std::fs::read_to_string(&p).unwrap();
        let block_scalar_lines = "      notes:\n      model: {old}\n      more prose about {old}\n"
            .replace("{old}", &old);
        assert!(
            after.contains(&block_scalar_lines),
            "the block scalar's content must survive byte-identical:\n{after}"
        );
        assert!(
            after.contains(&format!("\n    model: {new}\n")),
            "the real, sibling model: field must still be fixed:\n{after}"
        );
    }

    /// `is_model_key`'s textual scan does not recognise a quoted key
    /// (`"model":`), while `check_declarations`'s structured parse does —
    /// a genuine detection/rewrite disagreement. `update_declarations` must
    /// refuse rather than silently under-report, and must leave the file
    /// completely untouched (no partial rewrite the caller never asked
    /// for).
    #[test]
    fn a_quoted_key_is_detected_but_not_textually_matched_and_the_disagreement_is_a_hard_error() {
        let (old, _new) = a_deprecated_model();
        let dir = tempfile::tempdir().unwrap();
        let p = write_yaml(
            dir.path(),
            &format!("defaults:\n  provider: claude\n  \"model\": {old}\nagents:\n  - name: a\n"),
        );

        let findings = check_declarations(&p);
        assert_eq!(
            findings.len(),
            1,
            "the structured parse must still find it: {findings:?}"
        );

        let before = std::fs::read_to_string(&p).unwrap();
        let err = update_declarations(&p, &findings).unwrap_err().to_string();
        assert!(
            err.contains("detection and rewrite disagree"),
            "must name what kind of failure this is: {err}"
        );
        assert!(
            err.contains("defaults [defaults.model]"),
            "must name the specific finding that could not be applied: {err}"
        );
        assert_eq!(
            std::fs::read_to_string(&p).unwrap(),
            before,
            "a disagreement must leave the file completely untouched, not partially rewritten"
        );
    }

    #[test]
    fn check_declarations_of_an_unreadable_file_is_empty_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("does-not-exist.yaml");
        assert!(check_declarations(&p).is_empty());
    }

    #[test]
    fn update_declarations_with_no_findings_leaves_the_file_untouched() {
        let (old, _new) = a_deprecated_model();
        let dir = tempfile::tempdir().unwrap();
        let p = write_yaml(dir.path(), &format!("defaults:\n  model: {old}\n"));
        let before = std::fs::read_to_string(&p).unwrap();
        assert_eq!(update_declarations(&p, &[]).unwrap(), 0);
        assert_eq!(std::fs::read_to_string(&p).unwrap(), before);
    }

    /// `check_project` must also see `.armadai/agents.yaml` — the wiring
    /// this task exists to add (see also `auto_check_and_prompt`, verified
    /// by hand rather than here since it drives an interactive prompt).
    /// Same project-fixture shape as `test_check_project_with_agents`, but
    /// the deprecated model lives in the declaration file instead of a
    /// `.md`.
    #[test]
    fn check_project_also_scans_the_declarations_file() {
        let dir = tempfile::tempdir().unwrap();
        let armadai_dir = dir.path().join(".armadai");
        std::fs::create_dir_all(&armadai_dir).unwrap();
        std::fs::write(armadai_dir.join("config.yaml"), "agents: []\n").unwrap();
        let (old, new) = a_deprecated_model();
        std::fs::write(
            armadai_dir.join("agents.yaml"),
            format!("defaults:\n  provider: claude\n  model: {old}\nagents:\n  - name: a\n"),
        )
        .unwrap();

        let findings = check_project(dir.path()).unwrap();
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(findings[0].current, old);
        assert_eq!(findings[0].replacement, new);
    }

    // -----------------------------------------------------------------
    // `apply_finding` — the routing decision `auto_check_and_prompt` makes
    // on confirmation. Tested directly, since exercising the surrounding
    // `dialoguer::Confirm` prompt would need a real terminal.
    // -----------------------------------------------------------------

    #[test]
    fn apply_finding_routes_a_declarations_finding_to_update_declarations() {
        let (old, new) = a_deprecated_model();
        let dir = tempfile::tempdir().unwrap();
        let p = write_yaml(
            dir.path(),
            &format!("defaults:\n  model: {old}\nagents:\n  - name: a\n"),
        );
        let finding = DeprecationFinding {
            agent_path: p.clone(),
            agent_name: "defaults".into(),
            field: "defaults.model".into(),
            current: old,
            replacement: new.clone(),
        };

        // `decls_path` equal to the finding's own path is what marks it as
        // declarations-sourced.
        let n = apply_finding(&finding, &p).unwrap();
        assert_eq!(n, 1);
        assert!(std::fs::read_to_string(&p).unwrap().contains(&new));
    }

    #[test]
    fn apply_finding_routes_a_file_finding_to_update_agent_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.md");
        std::fs::write(&path, create_agent_md("Test", "gpt-4-turbo", &[])).unwrap();
        let finding = DeprecationFinding {
            agent_path: path.clone(),
            agent_name: "Test".into(),
            field: "model".into(),
            current: "gpt-4-turbo".into(),
            replacement: "gpt-4o".into(),
        };
        // `decls_path` deliberately different from the finding's own path,
        // so only the else-branch (`update_agent_file`) can produce a fix.
        let decls_path = dir.path().join("agents.yaml");

        let n = apply_finding(&finding, &decls_path).unwrap();
        assert_eq!(n, 1);
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("model: gpt-4o")
        );
    }
}
