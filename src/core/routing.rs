use std::collections::HashMap;

use serde::Deserialize;

use crate::linker::model_resolution::ModelTier;

fn tier_from_str(s: &str) -> Option<ModelTier> {
    match s.to_lowercase().as_str() {
        "fast" => Some(ModelTier::Fast),
        "pro" => Some(ModelTier::Pro),
        "max" => Some(ModelTier::Max),
        _ => None,
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct LengthThresholds {
    pub fast_max: usize,
    pub pro_max: usize,
}
impl Default for LengthThresholds {
    fn default() -> Self {
        LengthThresholds {
            fast_max: 500,
            pro_max: 4000,
        }
    }
}

/// `#[serde(default)]` here means: a missing `max` or `fast` key falls back to
/// this struct's `Default` impl *per field* — not to an empty `Vec`. This is
/// what makes a partial override (e.g. `keywords: { max: [...] }` alone)
/// preserve the embedded default list for the field that was omitted.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Keywords {
    pub max: Vec<String>,
    pub fast: Vec<String>,
}

impl Default for Keywords {
    fn default() -> Self {
        Keywords {
            max: ["refactor", "architecture", "prove", "debug"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            fast: ["list", "format", "summarize"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
        }
    }
}

/// Routing rules for `latest:auto` agents, loaded from the `routing:` section
/// of `armadai.yaml`.
///
/// Merge semantics: each top-level sub-section (`length_thresholds`,
/// `keywords`, `tags`, `budget_downgrade_ratio`) falls back to its own
/// embedded default *field by field* when omitted, thanks to `#[serde(default)]`
/// combined with a matching `Default` impl on the sub-section type. A partial
/// override such as `keywords: { max: [...] }` therefore keeps the embedded
/// default for `fast` rather than resetting it to empty. `tags` is a
/// `HashMap` and is replaced wholesale by any keys present in the override
/// (existing embedded tag mappings for keys not repeated are lost).
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct RoutingRules {
    pub length_thresholds: LengthThresholds,
    pub keywords: Keywords,
    /// tag -> tier ("fast"|"pro"|"max")
    pub tags: HashMap<String, String>,
    pub budget_downgrade_ratio: f64,
}

impl Default for RoutingRules {
    fn default() -> Self {
        let mut tags = HashMap::new();
        tags.insert("critical".into(), "max".into());
        tags.insert("architecture".into(), "max".into());
        tags.insert("format".into(), "fast".into());
        RoutingRules {
            length_thresholds: LengthThresholds::default(),
            keywords: Keywords::default(),
            tags,
            budget_downgrade_ratio: 0.2,
        }
    }
}

pub struct BudgetState {
    pub remaining_ratio: f64,
}

fn tier_by_length(input: &str, t: &LengthThresholds) -> ModelTier {
    let n = input.chars().count();
    if n < t.fast_max {
        ModelTier::Fast
    } else if n < t.pro_max {
        ModelTier::Pro
    } else {
        ModelTier::Max
    }
}

fn tier_by_keywords(input: &str, k: &Keywords) -> Option<ModelTier> {
    let low = input.to_lowercase();
    if k.max.iter().any(|w| low.contains(&w.to_lowercase())) {
        Some(ModelTier::Max)
    } else if k.fast.iter().any(|w| low.contains(&w.to_lowercase())) {
        Some(ModelTier::Fast)
    } else {
        None
    }
}

/// Decide the model tier for a `latest:auto` agent. Pure & deterministic.
pub fn route(
    input: &str,
    agent_tags: &[String],
    budget: Option<BudgetState>,
    rules: &RoutingRules,
) -> ModelTier {
    // 1. Tag override: highest tier among mapped tags wins, ignoring input signals.
    let tag_tier = agent_tags
        .iter()
        .filter_map(|t| rules.tags.get(t))
        .filter_map(|s| tier_from_str(s))
        .max();

    let mut tier = if let Some(t) = tag_tier {
        t
    } else {
        // 2. Otherwise: max(length, keywords)
        let by_len = tier_by_length(input, &rules.length_thresholds);
        match tier_by_keywords(input, &rules.keywords) {
            Some(kw) => by_len.max(kw),
            None => by_len,
        }
    };

    // 3. Budget cap (downgrade only), applied last.
    if let Some(b) = budget
        && b.remaining_ratio <= rules.budget_downgrade_ratio
    {
        tier = tier.min(ModelTier::Fast);
    }
    tier
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::linker::model_resolution::ModelTier::*;

    fn rules() -> RoutingRules {
        RoutingRules::default()
    }

    #[test]
    fn length_thresholds_select_tier() {
        assert_eq!(route("hi", &[], None, &rules()), Fast); // short
        assert_eq!(route(&"x".repeat(1000), &[], None, &rules()), Pro);
        assert_eq!(route(&"x".repeat(5000), &[], None, &rules()), Max);
    }

    #[test]
    fn keyword_escalates_over_length() {
        // short input but contains a Max keyword
        assert_eq!(route("please refactor this", &[], None, &rules()), Max);
    }

    #[test]
    fn tag_overrides_input_signals() {
        // long input (would be Max by length) but agent tagged fast → Fast
        assert_eq!(
            route(&"x".repeat(5000), &["format".into()], None, &rules()),
            Fast
        );
        // tag critical → Max regardless of short input
        assert_eq!(route("hi", &["critical".into()], None, &rules()), Max);
    }

    #[test]
    fn partial_keywords_override_keeps_embedded_default_for_omitted_field() {
        // Only `max` is overridden; `fast` is omitted from the YAML and must
        // keep its embedded default rather than becoming an empty vec.
        let yaml = "keywords:\n  max:\n    - custom-signal\n";
        let r: RoutingRules = serde_yaml_ng::from_str(yaml).unwrap();
        assert_eq!(r.keywords.max, vec!["custom-signal".to_string()]);
        assert_eq!(
            r.keywords.fast,
            vec![
                "list".to_string(),
                "format".to_string(),
                "summarize".to_string()
            ]
        );
    }

    #[test]
    fn budget_caps_downward_last() {
        // tag critical → Max, but budget nearly exhausted → capped to Fast
        let b = Some(BudgetState {
            remaining_ratio: 0.05,
        });
        assert_eq!(route("hi", &["critical".into()], b, &rules()), Fast);
    }
}
