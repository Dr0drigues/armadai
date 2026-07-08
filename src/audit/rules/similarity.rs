use std::collections::{BTreeMap, HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};

use super::{AuditContext, Finding, Severity, UnionFind};

pub(super) const DUPLICATION_WINDOW: usize = 8;
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

/// A06 — duplicated content, one finding per connected cluster of agents
/// (pairwise output explodes in O(n²) on real fleets — cls-monorepo showed
/// 6 findings for one shared block across 4 gate agents).
pub(super) fn a06_duplicated_blocks(ctx: &AuditContext) -> Vec<Finding> {
    let agents = &ctx.config.agents;
    let hashes: Vec<HashSet<u64>> = agents
        .iter()
        .map(|a| window_hashes(&a.system_prompt, DUPLICATION_WINDOW))
        .collect();
    let mut uf = UnionFind::new(agents.len());
    let mut pairs = Vec::new();
    for i in 0..agents.len() {
        for j in (i + 1)..agents.len() {
            let shared = hashes[i].intersection(&hashes[j]).count();
            if shared > 0 {
                uf.union(i, j);
                pairs.push((i, shared));
            }
        }
    }
    let mut clusters: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for i in 0..agents.len() {
        clusters.entry(uf.find(i)).or_default().push(i);
    }
    let mut max_shared: HashMap<usize, usize> = HashMap::new();
    for (i, shared) in pairs {
        let root = uf.find(i);
        let entry = max_shared.entry(root).or_insert(0);
        *entry = (*entry).max(shared);
    }
    clusters
        .into_iter()
        .filter(|(_, members)| members.len() >= 2)
        .map(|(root, members)| {
            let names: Vec<&str> = members.iter().map(|&i| agents[i].name.as_str()).collect();
            let strength = max_shared.get(&root).copied().unwrap_or(0);
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
        });
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].rule, "A07");
    }

    #[test]
    fn jaccard_of_identical_sets_is_one() {
        assert!((jaccard("a b c", "c b a") - 1.0).abs() < f64::EPSILON);
    }
}
