use std::collections::{BTreeMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};

use super::{AuditContext, Finding, Severity, UnionFind};
use crate::audit::reverse::ImportedAgent;

pub(crate) const DUPLICATION_WINDOW: usize = 8;
pub(super) const REDUNDANCY_THRESHOLD: f64 = 0.8;

/// Hashes of every window of `window` consecutive non-empty trimmed lines.
fn window_hashes(text: &str, window: usize) -> HashSet<u64> {
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    lines
        .windows(window)
        .map(|w| {
            let mut h = DefaultHasher::new();
            for line in w {
                line.hash(&mut h);
            }
            h.finish()
        })
        .collect()
}

/// Word-set Jaccard similarity on lowercase text.
pub(super) fn jaccard(a: &str, b: &str) -> f64 {
    let la = a.to_lowercase();
    let lb = b.to_lowercase();
    let sa: HashSet<&str> = la.split_whitespace().collect();
    let sb: HashSet<&str> = lb.split_whitespace().collect();
    if sa.is_empty() || sb.is_empty() {
        return 0.0;
    }
    let inter = sa.intersection(&sb).count() as f64;
    let union = sa.union(&sb).count() as f64;
    inter / union
}

/// Connected components of agents sharing at least one `DUPLICATION_WINDOW`-line
/// window, computed via union-find over pairwise window-hash intersections.
/// Only components with 2+ members are kept; components are sorted by their
/// lowest member index. Shared by A06 and `--propose`'s fragment extraction.
pub(crate) fn duplication_clusters(agents: &[ImportedAgent]) -> Vec<Vec<usize>> {
    let hashes: Vec<HashSet<u64>> = agents
        .iter()
        .map(|a| window_hashes(&a.system_prompt, DUPLICATION_WINDOW))
        .collect();
    let mut uf = UnionFind::new(agents.len());
    for i in 0..agents.len() {
        for j in (i + 1)..agents.len() {
            if hashes[i].intersection(&hashes[j]).next().is_some() {
                uf.union(i, j);
            }
        }
    }
    let mut clusters: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for i in 0..agents.len() {
        clusters.entry(uf.find(i)).or_default().push(i);
    }
    clusters
        .into_values()
        .filter(|members| members.len() >= 2)
        .collect()
}

/// A06 — duplicated content, one finding per connected cluster of agents
/// (pairwise output explodes in O(n²) on real fleets — cls-monorepo showed
/// 6 findings for one shared block across 4 gate agents).
pub(super) fn a06_duplicated_blocks(ctx: &AuditContext) -> Vec<Finding> {
    let agents = &ctx.config.agents;
    let hashes: Vec<HashSet<u64>> = agents
        .iter()
        .map(|a| window_hashes(&a.system_prompt, DUPLICATION_WINDOW))
        .collect();
    duplication_clusters(agents)
        .into_iter()
        .map(|members| {
            let names: Vec<&str> = members.iter().map(|&i| agents[i].name.as_str()).collect();
            let mut strength = 0;
            for (pos, &i) in members.iter().enumerate() {
                for &j in &members[pos + 1..] {
                    strength = strength.max(hashes[i].intersection(&hashes[j]).count());
                }
            }
            Finding {
                rule: "A06",
                severity: Severity::Warning,
                file: agents[members[0]].source_path.clone(),
                related: members[1..]
                    .iter()
                    .map(|&i| agents[i].source_path.clone())
                    .collect(),
                message: format!(
                    "{} agents share duplicated content: {} (up to {strength} matching {DUPLICATION_WINDOW}-line windows)",
                    members.len(),
                    names.join(", ")
                ),
                suggestion: Some(
                    "extract the shared block into one reusable prompt fragment".to_string(),
                ),
            }
        })
        .collect()
}

/// A07 — two agents look interchangeable (near-identical descriptions).
pub(super) fn a07_redundant_agents(ctx: &AuditContext) -> Vec<Finding> {
    let agents = &ctx.config.agents;
    let mut findings = Vec::new();
    for i in 0..agents.len() {
        for j in (i + 1)..agents.len() {
            let (Some(da), Some(db)) = (
                &agents[i].metadata.description,
                &agents[j].metadata.description,
            ) else {
                continue;
            };
            if jaccard(da, db) >= REDUNDANCY_THRESHOLD {
                findings.push(Finding {
                    rule: "A07",
                    severity: Severity::Info,
                    file: agents[i].source_path.clone(),
                    related: Vec::new(),
                    message: format!(
                        "agents '{}' and '{}' have near-identical descriptions",
                        agents[i].name, agents[j].name
                    ),
                    suggestion: Some("consider merging them or sharpening their roles".to_string()),
                });
            }
        }
    }
    findings
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::rules::test_support::{agent, config_with};
    use crate::audit::rules::{AuditContext, AuditSettings};

    fn shared_block() -> String {
        (1..=10)
            .map(|i| format!("Shared convention line {i}\n"))
            .collect()
    }

    #[test]
    fn a06_flags_agents_sharing_a_block() {
        let block = shared_block();
        let a = agent("one", &format!("{block}Specific to one."));
        let b = agent("two", &format!("Intro.\n{block}"));
        let c = agent("three", "Totally different.");
        let config = config_with(vec![a, b, c]);
        let settings = AuditSettings::default();
        let f = a06_duplicated_blocks(&AuditContext {
            config: &config,
            settings: &settings,
            usage: None,
        });
        assert_eq!(f.len(), 1);
        assert!(f[0].message.contains("one") && f[0].message.contains("two"));
    }

    #[test]
    fn a06_clusters_four_agents_into_one_finding() {
        let block = shared_block();
        let agents: Vec<_> = ["doc", "perf", "quality", "security"]
            .iter()
            .map(|n| agent(n, &format!("{block}Specific to {n}.")))
            .collect();
        let config = config_with(agents);
        let settings = AuditSettings::default();
        let f = a06_duplicated_blocks(&AuditContext {
            config: &config,
            settings: &settings,
            usage: None,
        });
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].related.len(), 3);
        assert!(f[0].message.contains("4 agents"));
        assert!(f[0].message.contains("quality"));
    }

    #[test]
    fn a06_keeps_disjoint_clusters_separate() {
        let block_a = shared_block();
        let block_b: String = (1..=10)
            .map(|i| format!("Other convention {i}\n"))
            .collect();
        let config = config_with(vec![
            agent("a1", &block_a),
            agent("a2", &block_a),
            agent("b1", &block_b),
            agent("b2", &block_b),
        ]);
        let settings = AuditSettings::default();
        let f = a06_duplicated_blocks(&AuditContext {
            config: &config,
            settings: &settings,
            usage: None,
        });
        assert_eq!(f.len(), 2);
    }

    #[test]
    fn a07_flags_near_identical_descriptions() {
        let mut a = agent("rev-a", "Body A");
        let mut b = agent("rev-b", "Body B");
        a.metadata.description = Some("reviews rust code for bugs and style".to_string());
        b.metadata.description = Some("reviews rust code for style and bugs".to_string());
        let config = config_with(vec![a, b]);
        let settings = AuditSettings::default();
        let f = a07_redundant_agents(&AuditContext {
            config: &config,
            settings: &settings,
            usage: None,
        });
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].rule, "A07");
    }

    #[test]
    fn jaccard_of_identical_sets_is_one() {
        assert!((jaccard("a b c", "c b a") - 1.0).abs() < f64::EPSILON);
    }
}
