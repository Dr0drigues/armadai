//! Collision rules (C01-C05): conflicting claims between agentic assets.
use std::collections::BTreeMap;

use super::similarity::jaccard;
use super::{AuditContext, Finding, Severity};

/// C01 — two assets claim the same name.
/// Same-kind duplicates are Critical (routing is ambiguous); an agent/skill
/// homonym is Warning (different namespaces, but confusing).
pub(super) fn c01_name_collisions(ctx: &AuditContext) -> Vec<Finding> {
    let mut findings = Vec::new();
    let mut agents_by_name: BTreeMap<&str, Vec<&std::path::Path>> = BTreeMap::new();
    for a in ctx.config.agents.iter().filter(|a| a.issues.is_empty()) {
        agents_by_name
            .entry(a.name.as_str())
            .or_default()
            .push(&a.source_path);
    }
    let mut skills_by_name: BTreeMap<&str, Vec<&std::path::Path>> = BTreeMap::new();
    for s in ctx.config.skills.iter().filter(|s| s.issues.is_empty()) {
        skills_by_name
            .entry(s.name.as_str())
            .or_default()
            .push(&s.source_path);
    }
    for (kind, by_name) in [("agent", &agents_by_name), ("skill", &skills_by_name)] {
        for (name, paths) in by_name {
            if paths.len() > 1 {
                findings.push(Finding {
                    rule: "C01",
                    severity: Severity::Critical,
                    file: paths[0].to_path_buf(),
                    related: paths[1..].iter().map(|p| p.to_path_buf()).collect(),
                    message: format!(
                        "{} {kind} files share the name '{name}' — routing is ambiguous",
                        paths.len()
                    ),
                    suggestion: Some("rename or remove the duplicates".to_string()),
                });
            }
        }
    }
    for (name, agent_paths) in &agents_by_name {
        if let Some(skill_paths) = skills_by_name.get(name) {
            findings.push(Finding {
                rule: "C01",
                severity: Severity::Warning,
                file: agent_paths[0].to_path_buf(),
                related: skill_paths.iter().map(|p| p.to_path_buf()).collect(),
                message: format!("agent and skill share the name '{name}'"),
                suggestion: Some("give the skill or the agent a distinct name".to_string()),
            });
        }
    }
    findings
}

/// C03 — two activation surfaces (skill↔skill or agent↔skill) with
/// near-identical descriptions: the router cannot pick reliably.
/// Agent↔agent redundancy stays A07 (higher threshold, Info): merging two
/// agents is a design suggestion, an ambiguous skill trigger is a defect.
pub(super) fn c03_activation_overlap(ctx: &AuditContext) -> Vec<Finding> {
    let threshold = ctx.settings.activation_similarity;
    // (kind, name, path, description)
    let mut surfaces: Vec<(&str, &str, &std::path::Path, &str)> = Vec::new();
    for s in ctx.config.skills.iter().filter(|s| s.issues.is_empty()) {
        if let Some(d) = &s.description {
            surfaces.push(("skill", &s.name, &s.source_path, d));
        }
    }
    for a in ctx.config.agents.iter().filter(|a| a.issues.is_empty()) {
        if let Some(d) = &a.metadata.description {
            surfaces.push(("agent", &a.name, &a.source_path, d));
        }
    }
    let mut findings = Vec::new();
    for i in 0..surfaces.len() {
        for j in (i + 1)..surfaces.len() {
            let (ka, na, pa, da) = surfaces[i];
            let (kb, nb, pb, db) = surfaces[j];
            if ka == "agent" && kb == "agent" {
                continue; // A07's turf
            }
            if jaccard(da, db) >= threshold {
                findings.push(Finding {
                    rule: "C03",
                    severity: Severity::Warning,
                    file: pa.to_path_buf(),
                    related: vec![pb.to_path_buf()],
                    message: format!(
                        "{ka} '{na}' and {kb} '{nb}' have overlapping activation descriptions — routing is ambiguous"
                    ),
                    suggestion: Some(
                        "sharpen the descriptions so each one triggers on distinct intents"
                            .to_string(),
                    ),
                });
            }
        }
    }
    findings
}

/// Prefix of a glob up to its first wildcard — a cheap, dependency-free
/// overlap test: two globs can match a common path iff one literal prefix
/// contains the other.
fn glob_prefix(g: &str) -> &str {
    let idx = g.find(['*', '?', '[']).unwrap_or(g.len());
    &g[..idx]
}

fn prefix_contains(short: &str, long: &str) -> bool {
    if !long.starts_with(short) {
        return false;
    }
    short.is_empty()
        || short.ends_with('/')
        || long.len() == short.len()
        || long[short.len()..].starts_with('/')
}

fn globs_overlap(a: &str, b: &str) -> bool {
    let (pa, pb) = (glob_prefix(a), glob_prefix(b));
    prefix_contains(pa, pb) || prefix_contains(pb, pa)
}

fn scoped_agents<'a>(
    ctx: &'a AuditContext,
) -> Vec<(&'a crate::audit::reverse::ImportedAgent, Vec<String>)> {
    ctx.config
        .agents
        .iter()
        .filter(|a| a.issues.is_empty())
        .filter_map(|a| {
            let globs = a.metadata.scope_globs();
            (!globs.is_empty()).then_some((a, globs))
        })
        .collect()
}

fn overlapping_pairs(
    scoped: &[(&crate::audit::reverse::ImportedAgent, Vec<String>)],
) -> Vec<(usize, usize)> {
    let mut pairs = Vec::new();
    for i in 0..scoped.len() {
        for j in (i + 1)..scoped.len() {
            let overlap = scoped[i]
                .1
                .iter()
                .any(|ga| scoped[j].1.iter().any(|gb| globs_overlap(ga, gb)));
            if overlap {
                pairs.push((i, j));
            }
        }
    }
    pairs
}

/// C02 — agents claim overlapping path scopes (custom `paths:` field),
/// clustered like A06.
pub(super) fn c02_scope_overlap(ctx: &AuditContext) -> Vec<Finding> {
    let scoped = scoped_agents(ctx);
    let pairs = overlapping_pairs(&scoped);
    let mut uf = super::UnionFind::new(scoped.len());
    for &(i, j) in &pairs {
        uf.union(i, j);
    }
    let mut clusters: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for i in 0..scoped.len() {
        clusters.entry(uf.find(i)).or_default().push(i);
    }
    clusters
        .into_values()
        .filter(|members| members.len() >= 2)
        .map(|members| {
            let names: Vec<&str> = members.iter().map(|&i| scoped[i].0.name.as_str()).collect();
            Finding {
                rule: "C02",
                severity: Severity::Warning,
                file: scoped[members[0]].0.source_path.clone(),
                related: members[1..]
                    .iter()
                    .map(|&i| scoped[i].0.source_path.clone())
                    .collect(),
                message: format!(
                    "{} agents claim overlapping path scopes: {}",
                    members.len(),
                    names.join(", ")
                ),
                suggestion: Some(
                    "split the scopes or make the ownership hierarchy explicit".to_string(),
                ),
            }
        })
        .collect()
}

/// C05 — same scope, inconsistent tool restriction (one locked, one open).
pub(super) fn c05_inconsistent_tools(ctx: &AuditContext) -> Vec<Finding> {
    fn permissive(tools: &Option<Vec<String>>) -> bool {
        match tools {
            None => true,
            Some(t) => t.iter().any(|x| x == "*"),
        }
    }
    let scoped = scoped_agents(ctx);
    overlapping_pairs(&scoped)
        .into_iter()
        .filter(|&(i, j)| {
            permissive(&scoped[i].0.metadata.tools) != permissive(&scoped[j].0.metadata.tools)
        })
        .map(|(i, j)| Finding {
            rule: "C05",
            severity: Severity::Info,
            file: scoped[i].0.source_path.clone(),
            related: vec![scoped[j].0.source_path.clone()],
            message: format!(
                "agents '{}' and '{}' share a path scope but one restricts tools and the other does not",
                scoped[i].0.name, scoped[j].0.name
            ),
            suggestion: Some("align the tool policies of agents working on the same files".to_string()),
        })
        .collect()
}

/// C04 — two agents are declared owners of the same module in CLAUDE.md
/// coordination tables. Deliberately strict (exact agent-name cell + a
/// path-looking cell on the same row): low recall, near-zero noise.
pub(super) fn c04_double_ownership(ctx: &AuditContext) -> Vec<Finding> {
    let Some(instructions) = &ctx.config.instructions else {
        return Vec::new();
    };
    let known: std::collections::HashSet<&str> = ctx
        .config
        .agents
        .iter()
        .filter(|a| a.issues.is_empty())
        .map(|a| a.name.as_str())
        .collect();
    if known.is_empty() {
        return Vec::new();
    }
    let mut claims: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for line in instructions.content.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with('|') {
            continue;
        }
        let cells: Vec<&str> = trimmed.split('|').map(str::trim).collect();
        let Some(agent) = cells.iter().find(|c| known.contains(**c)).copied() else {
            continue;
        };
        for cell in &cells {
            if cell.contains('/') && !cell.contains(' ') && !cell.is_empty() {
                let owners = claims.entry(*cell).or_default();
                if !owners.contains(&agent) {
                    owners.push(agent);
                }
            }
        }
    }
    claims
        .into_iter()
        .filter(|(_, owners)| owners.len() > 1)
        .map(|(path, owners)| Finding {
            rule: "C04",
            severity: Severity::Warning,
            file: instructions.source_path.clone(),
            related: Vec::new(),
            message: format!(
                "agents {} are all declared owners of '{path}'",
                owners
                    .iter()
                    .map(|o| format!("'{o}'"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            suggestion: Some(
                "pick a single owner per module or document the shared ownership".to_string(),
            ),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::rules::test_support::{agent, config_with};
    use crate::audit::rules::{AuditContext, AuditSettings, Severity};

    #[test]
    fn c01_flags_duplicate_agent_names() {
        let mut a = agent("reviewer", "Body");
        a.source_path = ".claude/agents/backend/reviewer.md".into();
        let b = agent("reviewer", "Body");
        let c = agent("other", "Body");
        let config = config_with(vec![a, b, c]);
        let settings = AuditSettings::default();
        let f = c01_name_collisions(&AuditContext {
            config: &config,
            settings: &settings,
        });
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].severity, Severity::Critical);
        assert!(f[0].message.contains("reviewer"));
        assert_eq!(f[0].related.len(), 1);
    }

    #[test]
    fn c01_flags_agent_skill_homonym_as_warning() {
        use crate::audit::reverse::{ImportedConfig, ImportedSkill};
        use std::collections::BTreeMap;
        let config = ImportedConfig {
            agents: vec![agent("deploy", "Body")],
            skills: vec![ImportedSkill {
                name: "deploy".into(),
                source_path: ".claude/skills/deploy/SKILL.md".into(),
                description: Some("d".into()),
                has_skill_md: true,
                has_frontmatter: true,
                issues: Vec::new(),
                extra: BTreeMap::new(),
            }],
            ..Default::default()
        };
        let settings = AuditSettings::default();
        let f = c01_name_collisions(&AuditContext {
            config: &config,
            settings: &settings,
        });
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].severity, Severity::Warning);
    }

    fn skill(name: &str, description: &str) -> crate::audit::reverse::ImportedSkill {
        crate::audit::reverse::ImportedSkill {
            name: name.into(),
            source_path: format!(".claude/skills/{name}/SKILL.md").into(),
            description: Some(description.into()),
            has_skill_md: true,
            has_frontmatter: true,
            issues: Vec::new(),
            extra: std::collections::BTreeMap::new(),
        }
    }

    #[test]
    fn c03_flags_ambiguous_skill_descriptions() {
        use crate::audit::reverse::ImportedConfig;
        let config = ImportedConfig {
            skills: vec![
                skill("audit-a", "runs a full audit of the project quality"),
                skill("audit-b", "runs a full quality audit of the project"),
                skill("deploy", "ships the app to production servers"),
            ],
            ..Default::default()
        };
        let settings = AuditSettings::default(); // activation_similarity 0.6
        let f = c03_activation_overlap(&AuditContext {
            config: &config,
            settings: &settings,
        });
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].severity, Severity::Warning);
        assert!(f[0].message.contains("audit-a") && f[0].message.contains("audit-b"));
    }

    #[test]
    fn c03_crosses_agents_and_skills() {
        use crate::audit::reverse::ImportedConfig;
        let mut a = agent("checker", "Body");
        a.metadata.description = Some("reviews rust code for style and bugs".into());
        let config = ImportedConfig {
            agents: vec![a],
            skills: vec![skill("review", "reviews rust code for bugs and style")],
            ..Default::default()
        };
        let settings = AuditSettings::default();
        let f = c03_activation_overlap(&AuditContext {
            config: &config,
            settings: &settings,
        });
        assert_eq!(f.len(), 1);
    }

    #[test]
    fn globs_overlap_by_prefix() {
        assert!(globs_overlap("src/**", "src/cli/**"));
        assert!(globs_overlap("src/cli/mod.rs", "src/**"));
        assert!(!globs_overlap("src/**", "docs/**"));
        assert!(globs_overlap("**", "docs/**")); // catch-all overlaps everything
        assert!(!globs_overlap("api", "api-gateway/**"));
        assert!(!globs_overlap("src/cli", "src/climate/**"));
        assert!(globs_overlap("src/", "src/cli/**"));
        assert!(globs_overlap("src/cli", "src/cli/**"));
    }

    #[test]
    fn c02_clusters_agents_with_overlapping_paths() {
        use serde_yaml_ng::Value;
        let mut a = agent("wide", "Body");
        a.metadata
            .extra
            .insert("paths".into(), Value::String("src/**".into()));
        let mut b = agent("narrow", "Body");
        b.metadata
            .extra
            .insert("paths".into(), Value::String("src/cli/**".into()));
        let mut c = agent("docs", "Body");
        c.metadata
            .extra
            .insert("paths".into(), Value::String("docs/**".into()));
        let config = config_with(vec![a, b, c]);
        let settings = AuditSettings::default();
        let f = c02_scope_overlap(&AuditContext {
            config: &config,
            settings: &settings,
        });
        assert_eq!(f.len(), 1);
        assert!(f[0].message.contains("wide") && f[0].message.contains("narrow"));
        assert!(!f[0].message.contains("docs"));
    }

    #[test]
    fn c05_flags_inconsistent_tools_on_shared_scope() {
        use serde_yaml_ng::Value;
        let mut a = agent("locked", "Body"); // tools: [Read]
        a.metadata
            .extra
            .insert("paths".into(), Value::String("src/**".into()));
        let mut b = agent("open", "Body");
        b.metadata.tools = None;
        b.metadata
            .extra
            .insert("paths".into(), Value::String("src/cli/**".into()));
        let config = config_with(vec![a, b]);
        let settings = AuditSettings::default();
        let f = c05_inconsistent_tools(&AuditContext {
            config: &config,
            settings: &settings,
        });
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].severity, Severity::Info);
    }

    #[test]
    fn c04_flags_two_agents_owning_the_same_module() {
        use crate::audit::reverse::{ImportedConfig, ImportedInstructions};
        let config = ImportedConfig {
            agents: vec![agent("core-dev", "Body"), agent("cli-dev", "Body")],
            instructions: Some(ImportedInstructions {
                source_path: "CLAUDE.md".into(),
                content: "\
| Agent | Scope |\n\
|---|---|\n\
| core-dev | src/core/ |\n\
| cli-dev | src/core/ |\n\
| cli-dev | src/cli/ |\n"
                    .into(),
            }),
            ..Default::default()
        };
        let settings = AuditSettings::default();
        let f = c04_double_ownership(&AuditContext {
            config: &config,
            settings: &settings,
        });
        assert_eq!(f.len(), 1);
        assert!(f[0].message.contains("src/core/"));
        assert!(f[0].message.contains("core-dev") && f[0].message.contains("cli-dev"));
    }
}
