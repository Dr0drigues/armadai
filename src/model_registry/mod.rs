pub mod fetch;

use serde::{Deserialize, Serialize};

/// A model entry from the models.dev registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelEntry {
    pub id: String,
    pub name: Option<String>,
    #[serde(default)]
    pub cost: Option<ModelCost>,
    #[serde(default)]
    pub limit: Option<ModelLimits>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCost {
    pub input: Option<f64>,
    pub output: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelLimits {
    pub context: Option<u64>,
    pub output: Option<u64>,
}

impl ModelEntry {
    /// Build a display label for interactive selection.
    ///
    /// Examples:
    ///   "Claude Sonnet 4.5 — 200K ctx — $3.00/$15.00"
    ///   "gpt-4o — 128K ctx"
    pub fn display_label(&self) -> String {
        let name = self.name.as_deref().unwrap_or(&self.id);
        let mut parts = vec![name.to_string()];
        if let Some(ref limit) = self.limit
            && let Some(ctx) = limit.context
        {
            parts.push(format!("{}K ctx", ctx / 1000));
        }
        if let Some(ref cost) = self.cost
            && let (Some(i), Some(o)) = (cost.input, cost.output)
        {
            parts.push(format!("${:.2}/${:.2}", i, o));
        }
        parts.join(" — ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_label_full() {
        let entry = ModelEntry {
            id: "claude-sonnet-4-5".to_string(),
            name: Some("Claude Sonnet 4.5".to_string()),
            cost: Some(ModelCost {
                input: Some(3.0),
                output: Some(15.0),
            }),
            limit: Some(ModelLimits {
                context: Some(200_000),
                output: Some(8192),
            }),
        };
        assert_eq!(
            entry.display_label(),
            "Claude Sonnet 4.5 — 200K ctx — $3.00/$15.00"
        );
    }

    #[test]
    fn display_label_no_cost() {
        let entry = ModelEntry {
            id: "gpt-4o".to_string(),
            name: None,
            cost: None,
            limit: Some(ModelLimits {
                context: Some(128_000),
                output: None,
            }),
        };
        assert_eq!(entry.display_label(), "gpt-4o — 128K ctx");
    }

    #[test]
    fn display_label_no_metadata() {
        let entry = ModelEntry {
            id: "custom-model".to_string(),
            name: None,
            cost: None,
            limit: None,
        };
        assert_eq!(entry.display_label(), "custom-model");
    }

    #[test]
    fn display_label_partial_cost() {
        let entry = ModelEntry {
            id: "model".to_string(),
            name: Some("My Model".to_string()),
            cost: Some(ModelCost {
                input: Some(1.0),
                output: None,
            }),
            limit: None,
        };
        // Partial cost should not display
        assert_eq!(entry.display_label(), "My Model");
    }
}
