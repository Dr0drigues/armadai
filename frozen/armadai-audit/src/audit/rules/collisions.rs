//! Collision rules (C01-C05): conflicting claims between agentic assets.
use std::collections::BTreeMap;

use super::similarity::jaccard;
use super::{AuditContext, Finding, Severity};

/// The key every name-based grouping is built on: a name is only ambiguous
/// against the other names **one resolver** enumerates.
///
/// See [`ResolutionSpace`](crate::audit::reverse::ResolutionSpace): the global
/// pass assembles two unrelated trees, so grouping by name alone made every
/// agent `armadai link` had published collide with the library file it was
/// generated from (issue #399).
type NameKey<'a> = (&'a std::path::Path, &'a str);

/// C01 — two assets claim the same name **inside one resolution space**.
/// Same-kind duplicates are Critical (routing is ambiguous); an agent/skill
/// homonym is Warning (different namespaces, but confusing).
pub(super) fn c01_name_collisions(ctx: &AuditContext) -> Vec<Finding> {
    let mut findings = Vec::new();
    let mut agents_by_name: BTreeMap<NameKey, Vec<&std::path::Path>> = BTreeMap::new();
    for a in ctx.config.agents.iter().filter(|a| a.issues.is_empty()) {
        agents_by_name
            .entry((a.space.as_path(), a.name.as_str()))
            .or_default()
            .push(&a.source_path);
    }
    let mut skills_by_name: BTreeMap<NameKey, Vec<&std::path::Path>> = BTreeMap::new();
    for s in ctx.config.skills.iter().filter(|s| s.issues.is_empty()) {
        skills_by_name
            .entry((s.space.as_path(), s.name.as_str()))
            .or_default()
            .push(&s.source_path);
    }
    for (kind, by_name) in [("agent", &agents_by_name), ("skill", &skills_by_name)] {
        for (&(_, name), paths) in by_name {
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
    // Agent↔skill homonyms are cross-*namespace* but still same-space: the
    // two are confusing only to someone reading one tree.
    for (&(space, name), agent_paths) in &agents_by_name {
        if let Some(skill_paths) = skills_by_name.get(&(space, name)) {
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
/// Findings are aggregated into clusters (like A06/C02) to avoid N² noise.
pub(super) fn c03_activation_overlap(ctx: &AuditContext) -> Vec<Finding> {
    let threshold = ctx.settings.activation_similarity;
    // (kind, name, path, description, space)
    let mut surfaces: Vec<(&str, &str, &std::path::Path, &str, &std::path::Path)> = Vec::new();
    for s in ctx.config.skills.iter().filter(|s| s.issues.is_empty()) {
        if let Some(d) = &s.description {
            surfaces.push(("skill", &s.name, &s.source_path, d, &s.space));
        }
    }
    for a in ctx.config.agents.iter().filter(|a| a.issues.is_empty()) {
        if let Some(d) = &a.metadata.description {
            surfaces.push(("agent", &a.name, &a.source_path, d, &a.space));
        }
    }

    // Build similarity graph with UnionFind
    let mut uf = super::UnionFind::new(surfaces.len());
    for i in 0..surfaces.len() {
        for j in (i + 1)..surfaces.len() {
            let (ka, _, _, da, sa) = surfaces[i];
            let (kb, _, _, db, sb) = surfaces[j];
            if ka == "agent" && kb == "agent" {
                continue; // A07's turf
            }
            // Two routers, one entry each: an installed copy cannot make the
            // description it was copied from ambiguous (issue #399).
            if sa != sb {
                continue;
            }
            if jaccard(da, db) >= threshold {
                uf.union(i, j);
            }
        }
    }

    // Group into clusters
    let mut clusters: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for i in 0..surfaces.len() {
        clusters.entry(uf.find(i)).or_default().push(i);
    }

    // Create one finding per cluster (size >= 2)
    clusters
        .into_values()
        .filter(|members| members.len() >= 2)
        .map(|members| {
            let (_, _, first_path, _, _) = surfaces[members[0]];
            let related: Vec<_> = members[1..]
                .iter()
                .map(|&i| surfaces[i].2.to_path_buf())
                .collect();
            let names: Vec<String> = members
                .iter()
                .map(|&i| {
                    let (kind, name, _, _, _) = surfaces[i];
                    format!("{kind} '{name}'")
                })
                .collect();
            Finding {
                rule: "C03",
                severity: Severity::Warning,
                file: first_path.to_path_buf(),
                related,
                message: format!(
                    "{} assets have overlapping activation descriptions — routing is ambiguous: {}",
                    members.len(),
                    names.join(", ")
                ),
                suggestion: Some(
                    "sharpen the descriptions so each one triggers on distinct intents".to_string(),
                ),
            }
        })
        .collect()
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

/// Pairs of scoped agents whose globs overlap, **inside one resolution
/// space** — `C02` and `C05` both build on this.
///
/// The space guard is the same one `C01`/`A06`/`A07` carry, for the same
/// reason: two agents claiming `src/**` are only in conflict if one resolver
/// enumerates both. It is unreachable today and stays here on purpose — the
/// glob source is `PartialMetadata::extra["paths"]`, Claude Code frontmatter
/// the ArmadAI reverse pass deliberately leaves empty, so every scoped agent
/// currently comes from a single native tree. The day a second native root is
/// assembled, the alternative is not "a latent bug" but "the same false
/// positive #399 measured", and a rule that compares two assets without asking
/// which tree they came from is exactly what that issue was.
fn overlapping_pairs(
    scoped: &[(&crate::audit::reverse::ImportedAgent, Vec<String>)],
) -> Vec<(usize, usize)> {
    let mut pairs = Vec::new();
    for i in 0..scoped.len() {
        for j in (i + 1)..scoped.len() {
            if scoped[i].0.space != scoped[j].0.space {
                continue;
            }
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
/// Findings are aggregated into clusters to avoid N² noise.
pub(super) fn c05_inconsistent_tools(ctx: &AuditContext) -> Vec<Finding> {
    fn permissive(tools: &Option<Vec<String>>) -> bool {
        match tools {
            None => true,
            Some(t) => t.iter().any(|x| x == "*"),
        }
    }
    let scoped = scoped_agents(ctx);
    let conflicting_pairs: Vec<(usize, usize)> = overlapping_pairs(&scoped)
        .into_iter()
        .filter(|&(i, j)| {
            permissive(&scoped[i].0.metadata.tools) != permissive(&scoped[j].0.metadata.tools)
        })
        .collect();

    if conflicting_pairs.is_empty() {
        return Vec::new();
    }

    // Build clusters of agents with conflicting tool policies
    let mut uf = super::UnionFind::new(scoped.len());
    for &(i, j) in &conflicting_pairs {
        uf.union(i, j);
    }

    let mut clusters: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    // Collect all indices involved in conflicts
    for &(i, j) in &conflicting_pairs {
        let root_i = uf.find(i);
        let root_j = uf.find(j);
        clusters.entry(root_i).or_default().push(i);
        clusters.entry(root_j).or_default().push(j);
    }
    // Deduplicate indices within each cluster
    for cluster in clusters.values_mut() {
        cluster.sort_unstable();
        cluster.dedup();
    }

    clusters
        .into_values()
        .filter(|members| members.len() >= 2)
        .map(|members| {
            let names: Vec<&str> = members.iter().map(|&i| scoped[i].0.name.as_str()).collect();
            Finding {
                rule: "C05",
                severity: Severity::Info,
                file: scoped[members[0]].0.source_path.clone(),
                related: members[1..]
                    .iter()
                    .map(|&i| scoped[i].0.source_path.clone())
                    .collect(),
                message: format!(
                    "{} agents share path scopes but have inconsistent tool policies: {}",
                    members.len(),
                    names.join(", ")
                ),
                suggestion: Some(
                    "align the tool policies of agents working on the same files".to_string(),
                ),
            }
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
    let mut in_code_fence = false;
    for line in instructions.content.lines() {
        let trimmed = line.trim();
        // Track code fence boundaries to skip their content
        if trimmed.starts_with("```") {
            in_code_fence = !in_code_fence;
            continue;
        }
        // Skip lines inside code fences
        if in_code_fence {
            continue;
        }
        if !trimmed.starts_with('|') {
            continue;
        }
        let cells: Vec<&str> = trimmed.split('|').map(str::trim).collect();
        let Some(agent) = cells.iter().find(|c| known.contains(**c)).copied() else {
            continue;
        };
        for cell in &cells {
            // Skip URLs (contain protocol separator)
            if cell.contains("://") {
                continue;
            }
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
            usage: None,
        });
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].severity, Severity::Critical);
        assert!(f[0].message.contains("reviewer"));
        assert_eq!(f[0].related.len(), 1);
    }

    /// Issue #399. `armadai link --target claude` publishes the ArmadAI
    /// library into `~/.claude`, so the global pass reads the very same agent
    /// from two trees. Nothing resolves those two together — `~/.claude` is
    /// the native config, `~/.config/armadai` the source library — so the name
    /// is not ambiguous, and a healthy library must not exit 1.
    ///
    /// `armadai_agent` and `agent` carry different spaces by construction;
    /// `c01_flags_duplicate_agent_names` above is the control that a homonym
    /// *within* one tree stays Critical.
    #[test]
    fn c01_ignores_an_agent_published_into_a_second_root() {
        use crate::audit::rules::test_support::armadai_agent;
        let config = config_with(vec![
            armadai_agent("dev-lead", "You are the Dev Lead."),
            agent("dev-lead", "You are the Dev Lead."),
        ]);
        let settings = AuditSettings::default();
        let f = c01_name_collisions(&AuditContext {
            config: &config,
            settings: &settings,
            usage: None,
        });
        assert!(
            f.is_empty(),
            "one agent seen through two roots is not a collision, got {:?}",
            f.iter().map(|x| &x.message).collect::<Vec<_>>()
        );
    }

    /// Same shape for skills, which is where the defect was already reachable
    /// before #393: `~/.claude/skills/<x>` and `~/.config/armadai/skills/<x>`
    /// are the installed copy and the library it came from.
    #[test]
    fn c01_ignores_a_skill_installed_in_two_roots() {
        use crate::audit::reverse::ImportedConfig;
        let mut installed = skill("graphify", "turns any input into a knowledge graph");
        installed.space = ".claude".into();
        let mut library = skill("graphify", "turns any input into a knowledge graph");
        library.space = ".config/armadai".into();
        library.source_path = ".config/armadai/skills/graphify/SKILL.md".into();
        let config = ImportedConfig {
            skills: vec![installed, library],
            ..Default::default()
        };
        let settings = AuditSettings::default();
        let f = c01_name_collisions(&AuditContext {
            config: &config,
            settings: &settings,
            usage: None,
        });
        assert!(
            f.is_empty(),
            "one skill seen through two roots is not a collision, got {:?}",
            f.iter().map(|x| &x.message).collect::<Vec<_>>()
        );
    }

    /// The control for the skill case: two skills of one name *inside* one
    /// tree stay Critical. Without it, gating C01 on the space could be
    /// implemented as "never report skills" and stay green.
    #[test]
    fn c01_still_flags_two_skills_of_one_name_in_one_root() {
        use crate::audit::reverse::ImportedConfig;
        let a = skill("graphify", "turns any input into a knowledge graph");
        let mut b = skill("graphify", "turns any input into a knowledge graph");
        b.source_path = ".claude/skills/graphify-2/SKILL.md".into();
        let config = ImportedConfig {
            skills: vec![a, b],
            ..Default::default()
        };
        let settings = AuditSettings::default();
        let f = c01_name_collisions(&AuditContext {
            config: &config,
            settings: &settings,
            usage: None,
        });
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].severity, Severity::Critical);
        assert!(f[0].message.contains("graphify"));
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
                body_tokens: 0,
                issues: Vec::new(),
                extra: BTreeMap::new(),
                space: ".claude".into(),
            }],
            ..Default::default()
        };
        let settings = AuditSettings::default();
        let f = c01_name_collisions(&AuditContext {
            config: &config,
            settings: &settings,
            usage: None,
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
            body_tokens: 0,
            issues: Vec::new(),
            extra: std::collections::BTreeMap::new(),
            space: ".claude".into(),
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
            usage: None,
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
            usage: None,
        });
        assert_eq!(f.len(), 1);
    }

    /// Issue #399, `C03`'s share of it: one skill installed in two trees has
    /// the *same* description by construction, which is a perfect activation
    /// overlap and a Warning about nothing. Two routers, one entry each.
    /// `c03_flags_ambiguous_skill_descriptions` above is the control.
    #[test]
    fn c03_ignores_a_skill_installed_in_two_roots() {
        use crate::audit::reverse::ImportedConfig;
        let installed = skill("graphify", "turns any input into a knowledge graph");
        let mut library = skill("graphify", "turns any input into a knowledge graph");
        library.space = ".config/armadai".into();
        library.source_path = ".config/armadai/skills/graphify/SKILL.md".into();
        let config = ImportedConfig {
            skills: vec![installed, library],
            ..Default::default()
        };
        let settings = AuditSettings::default();
        let f = c03_activation_overlap(&AuditContext {
            config: &config,
            settings: &settings,
            usage: None,
        });
        assert!(
            f.is_empty(),
            "one skill seen through two roots cannot make its own routing \
             ambiguous, got {:?}",
            f.iter().map(|x| &x.message).collect::<Vec<_>>()
        );
    }

    #[test]
    fn c03_clusters_multiple_similar_skills() {
        use crate::audit::reverse::ImportedConfig;
        // UnionFind clustering: 3 similar skills should produce 1 finding, not 3.
        // The skills must have Jaccard >= 0.6 (60% word overlap).
        let config = ImportedConfig {
            skills: vec![
                skill("audit-a", "audit the code for style and bugs"),
                skill("audit-b", "audit the code for bugs and style"),
                skill("audit-c", "code audit for bugs and style"),
                skill("deploy", "ships the app to production servers"),
            ],
            ..Default::default()
        };
        let settings = AuditSettings::default();
        let f = c03_activation_overlap(&AuditContext {
            config: &config,
            settings: &settings,
            usage: None,
        });
        // All three audit-* skills are in one cluster (all pairwise Jaccard >= 0.6).
        // deploy is distinct. So we expect 1 finding covering the 3 similar skills.
        assert_eq!(f.len(), 1);
        assert!(f[0].message.contains("audit-a"));
        assert!(f[0].message.contains("audit-b"));
        assert!(f[0].message.contains("audit-c"));
        assert!(!f[0].message.contains("deploy"));
    }

    #[test]
    fn c03_skips_agent_to_agent_grouping() {
        use crate::audit::reverse::ImportedConfig;
        // C03 skips agent↔agent similarity (A07's turf): two agents with identical
        // descriptions should NOT produce a C03 finding.
        let mut a1 = agent("reviewer-backend", "Body");
        a1.metadata.description = Some("reviews backend code".into());
        let mut a2 = agent("reviewer-frontend", "Body");
        a2.metadata.description = Some("reviews backend code".into());
        let config = ImportedConfig {
            agents: vec![a1, a2],
            ..Default::default()
        };
        let settings = AuditSettings::default();
        let f = c03_activation_overlap(&AuditContext {
            config: &config,
            settings: &settings,
            usage: None,
        });
        // No C03 finding: agent↔agent similarity is deferred to A07.
        assert_eq!(f.len(), 0);
    }

    #[test]
    fn c03_produces_separate_findings_for_disjoint_clusters() {
        use crate::audit::reverse::ImportedConfig;
        // Two unrelated pairs of similar skills must NOT be merged into a
        // single finding: UnionFind clustering must keep disjoint clusters
        // disjoint.
        let config = ImportedConfig {
            skills: vec![
                skill("audit-a", "audit the code for style and bugs"),
                skill("audit-b", "audit the code for bugs and style"),
                skill(
                    "deploy-a",
                    "ships the app to production servers using containers",
                ),
                skill(
                    "deploy-b",
                    "ships the app to production servers with containers",
                ),
            ],
            ..Default::default()
        };
        let settings = AuditSettings::default();
        let f = c03_activation_overlap(&AuditContext {
            config: &config,
            settings: &settings,
            usage: None,
        });
        assert_eq!(f.len(), 2);
        let audit_finding = f
            .iter()
            .find(|finding| finding.message.contains("audit-a"))
            .expect("expected a finding for the audit-* cluster");
        assert!(audit_finding.message.contains("audit-b"));
        assert!(!audit_finding.message.contains("deploy-a"));
        assert!(!audit_finding.message.contains("deploy-b"));
        let deploy_finding = f
            .iter()
            .find(|finding| finding.message.contains("deploy-a"))
            .expect("expected a finding for the deploy-* cluster");
        assert!(deploy_finding.message.contains("deploy-b"));
        assert!(!deploy_finding.message.contains("audit-a"));
        assert!(!deploy_finding.message.contains("audit-b"));
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
            usage: None,
        });
        assert_eq!(f.len(), 1);
        assert!(f[0].message.contains("wide") && f[0].message.contains("narrow"));
        assert!(!f[0].message.contains("docs"));
    }

    /// `C02` and `C05` are the two rules built on `overlapping_pairs`, and
    /// they compare two assets exactly as `C01`/`A06`/`A07` do — so they ask
    /// the same question those three learned to ask in #399: are these files
    /// in one resolution space? Two agents claiming `src/**` in two unrelated
    /// trees are not competing for anything.
    ///
    /// Unreachable through today's importers (the glob source is Claude Code
    /// frontmatter, and every scoped agent therefore comes from one native
    /// tree), which is why it is pinned here at the rule's own level rather
    /// than through the binary.
    #[test]
    fn c02_and_c05_do_not_compare_agents_from_two_trees() {
        use serde_yaml_ng::Value;
        let scoped = |name: &str, globs: &str, space: &str, tools: Option<Vec<String>>| {
            let mut a = agent(name, "Body");
            a.metadata
                .extra
                .insert("paths".into(), Value::String(globs.into()));
            a.metadata.tools = tools;
            a.space = std::path::PathBuf::from(space);
            a
        };
        let settings = AuditSettings::default();
        let run = |config: &crate::audit::reverse::ImportedConfig| {
            let ctx = AuditContext {
                config,
                settings: &settings,
                usage: None,
            };
            (
                c02_scope_overlap(&ctx).len(),
                c05_inconsistent_tools(&ctx).len(),
            )
        };

        let across = config_with(vec![
            scoped("wide", "src/**", ".claude", Some(vec!["Read".into()])),
            scoped("narrow", "src/cli/**", ".config/armadai", None),
        ]);
        assert_eq!(run(&across), (0, 0), "two trees, two independent scopes");

        // The control: the same pair inside one tree is what both rules exist
        // to report, so the silence above is a guard and not a broken rule.
        let inside = config_with(vec![
            scoped("wide", "src/**", ".claude", Some(vec!["Read".into()])),
            scoped("narrow", "src/cli/**", ".claude", None),
        ]);
        assert_eq!(run(&inside), (1, 1));
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
            usage: None,
        });
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].severity, Severity::Info);
    }

    #[test]
    fn c05_clusters_multiple_conflicting_agents() {
        use serde_yaml_ng::Value;
        // UnionFind clustering: 3 agents with overlapping paths and conflicting
        // tool policies should produce 1 finding, not 3.
        let mut a = agent("permissive", "Body");
        a.metadata.tools = None; // open to all tools
        a.metadata
            .extra
            .insert("paths".into(), Value::String("src/**".into()));

        let mut b = agent("restricted-cli", "Body");
        b.metadata.tools = Some(vec!["Read".to_string(), "Edit".to_string()]);
        b.metadata
            .extra
            .insert("paths".into(), Value::String("src/cli/**".into()));

        let mut c = agent("restricted-core", "Body");
        c.metadata.tools = Some(vec!["Read".to_string()]);
        c.metadata
            .extra
            .insert("paths".into(), Value::String("src/core/**".into()));

        let config = config_with(vec![a, b, c]);
        let settings = AuditSettings::default();
        let f = c05_inconsistent_tools(&AuditContext {
            config: &config,
            settings: &settings,
            usage: None,
        });
        // All three agents overlap via src/** and have conflicting tool policies.
        // They should be in one cluster, producing 1 finding for the 3 agents.
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].related.len(), 2); // 2 related files + 1 primary
        assert!(f[0].message.contains("3 agents"));
    }

    #[test]
    fn c05_produces_separate_findings_for_disjoint_clusters() {
        use serde_yaml_ng::Value;
        // Two unrelated pairs of conflicting-tools agents, scoped to disjoint
        // path prefixes (`src/**` vs `docs/**`), must NOT be merged into a
        // single finding.
        let mut src_open = agent("src-open", "Body");
        src_open.metadata.tools = None; // permissive
        src_open
            .metadata
            .extra
            .insert("paths".into(), Value::String("src/**".into()));

        let mut src_locked = agent("src-locked", "Body");
        src_locked.metadata.tools = Some(vec!["Read".to_string()]);
        src_locked
            .metadata
            .extra
            .insert("paths".into(), Value::String("src/core/**".into()));

        let mut docs_open = agent("docs-open", "Body");
        docs_open.metadata.tools = None; // permissive
        docs_open
            .metadata
            .extra
            .insert("paths".into(), Value::String("docs/**".into()));

        let mut docs_locked = agent("docs-locked", "Body");
        docs_locked.metadata.tools = Some(vec!["Read".to_string()]);
        docs_locked
            .metadata
            .extra
            .insert("paths".into(), Value::String("docs/api/**".into()));

        let config = config_with(vec![src_open, src_locked, docs_open, docs_locked]);
        let settings = AuditSettings::default();
        let f = c05_inconsistent_tools(&AuditContext {
            config: &config,
            settings: &settings,
            usage: None,
        });
        assert_eq!(f.len(), 2);
        let src_finding = f
            .iter()
            .find(|finding| finding.message.contains("src-open"))
            .expect("expected a finding for the src-* cluster");
        assert!(src_finding.message.contains("src-locked"));
        assert!(!src_finding.message.contains("docs-open"));
        assert!(!src_finding.message.contains("docs-locked"));
        let docs_finding = f
            .iter()
            .find(|finding| finding.message.contains("docs-open"))
            .expect("expected a finding for the docs-* cluster");
        assert!(docs_finding.message.contains("docs-locked"));
        assert!(!docs_finding.message.contains("src-open"));
        assert!(!docs_finding.message.contains("src-locked"));
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
            usage: None,
        });
        assert_eq!(f.len(), 1);
        assert!(f[0].message.contains("src/core/"));
        assert!(f[0].message.contains("core-dev") && f[0].message.contains("cli-dev"));
    }

    #[test]
    fn c04_ignores_agent_name_in_code_fences() {
        use crate::audit::reverse::{ImportedConfig, ImportedInstructions};
        // C04 must not flag agent names inside code-fence blocks (```).
        let config = ImportedConfig {
            agents: vec![agent("core-dev", "Body"), agent("cli-dev", "Body")],
            instructions: Some(ImportedInstructions {
                source_path: "CLAUDE.md".into(),
                content: "# Owners\n\
```\n\
| Agent | Path |\n\
| core-dev | src/core/ |\n\
| cli-dev | src/core/ |\n\
```\n\
\n\
Actual table:\n\
| Agent | Path |\n\
| core-dev | src/core/ |\n"
                    .into(),
            }),
            ..Default::default()
        };
        let settings = AuditSettings::default();
        let f = c04_double_ownership(&AuditContext {
            config: &config,
            settings: &settings,
            usage: None,
        });
        // Only the real (non-fenced) table row counts. One agent owns src/core/,
        // so no collision finding should be generated.
        assert_eq!(f.len(), 0);
    }

    #[test]
    fn c04_ignores_agent_name_in_urls() {
        use crate::audit::reverse::{ImportedConfig, ImportedInstructions};
        // C04 must not interpret cells containing URLs (with ://) as path cells.
        let config = ImportedConfig {
            agents: vec![agent("dev", "Body"), agent("reviewer", "Body")],
            instructions: Some(ImportedInstructions {
                source_path: "CLAUDE.md".into(),
                content: "| Agent | Doc |\n\
|---|---|\n\
| dev | https://github.com/dev-repo |\n\
| reviewer | src/review/ |\n"
                    .into(),
            }),
            ..Default::default()
        };
        let settings = AuditSettings::default();
        let f = c04_double_ownership(&AuditContext {
            config: &config,
            settings: &settings,
            usage: None,
        });
        // No collision: the URL is not treated as a path, and no path is claimed twice.
        assert_eq!(f.len(), 0);
    }
}
