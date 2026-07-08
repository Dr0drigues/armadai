use std::collections::HashSet;
use std::hash::{DefaultHasher, Hash, Hasher};

use super::{AuditContext, Finding, Severity};

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

/// A06 — the same block of lines appears in two or more agent prompts.
pub(super) fn a06_duplicated_blocks(ctx: &AuditContext) -> Vec<Finding> {
    let agents = &ctx.config.agents;
    let hashes: Vec<HashSet<u64>> = agents
        .iter()
        .map(|a| window_hashes(&a.system_prompt, DUPLICATION_WINDOW))
        .collect();
    let mut findings = Vec::new();
    for i in 0..agents.len() {
        for j in (i + 1)..agents.len() {
            let shared = hashes[i].intersection(&hashes[j]).count();
            if shared > 0 {
                findings.push(Finding {
                    rule: "A06",
                    severity: Severity::Warning,
                    file: agents[i].source_path.clone(),
                    message: format!(
                        "agents '{}' and '{}' share {shared} duplicated block(s) of {DUPLICATION_WINDOW}+ lines",
                        agents[i].name, agents[j].name
                    ),
                    suggestion: Some(
                        "extract the shared block into one reusable prompt fragment".to_string(),
                    ),
                });
            }
        }
    }
    findings
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
