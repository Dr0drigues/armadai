//! Routing rules for `latest:auto` agents: the `routing:` section of
//! `armadai.yaml` (`ProjectConfig.routing`). Declaration types only.

use std::collections::HashMap;

use serde::Deserialize;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_keywords_override_keeps_embedded_default_for_omitted_field() {
        // Only `max` is overridden; `fast` is omitted from the YAML and must
        // keep its embedded default rather than becoming an empty vec.
        let yaml = "keywords:\n  max:\n    - custom-signal\n";
        let r: RoutingRules = armadai_model::yaml::from_str(yaml).unwrap();
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
}
