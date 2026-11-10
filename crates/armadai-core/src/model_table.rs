//! The model table built into the binary (`data/models.json`): the fallback
//! model of each vendor per tier, and the deprecated-model aliases.

use std::collections::BTreeMap;
use std::sync::OnceLock;

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelTable {
    pub version: u32,
    pub as_of: String,
    pub tiers: BTreeMap<String, VendorTiers>,
    pub aliases: BTreeMap<String, String>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VendorTiers {
    pub fast: String,
    pub pro: String,
    #[serde(default)]
    pub max: Option<String>,
}

/// The table built into the binary (`data/models.json`), parsed once.
pub fn model_table() -> &'static ModelTable {
    static TABLE: OnceLock<ModelTable> = OnceLock::new();
    TABLE.get_or_init(|| {
        serde_json::from_str(include_str!("../data/models.json"))
            .expect("data/models.json is a valid model table")
    })
}

/// The embedded table with each tier re-resolved from the local models cache
/// (`resolve_model_for_tier`), `as_of` set to `as_of`, aliases kept: what a
/// release proposes, for a human to check against the vendors' docs.
pub fn propose_tier_table(as_of: &str) -> serde_json::Value {
    use crate::model_resolution::{ModelTier, resolve_model_for_tier};
    let table = model_table();
    let tiers: serde_json::Map<String, serde_json::Value> = table
        .tiers
        .iter()
        .map(|(vendor, row)| {
            let mut out = serde_json::Map::new();
            out.insert(
                "fast".into(),
                resolve_model_for_tier(vendor, ModelTier::Fast).into(),
            );
            out.insert(
                "pro".into(),
                resolve_model_for_tier(vendor, ModelTier::Pro).into(),
            );
            if row.max.is_some() {
                out.insert(
                    "max".into(),
                    resolve_model_for_tier(vendor, ModelTier::Max).into(),
                );
            }
            (vendor.clone(), out.into())
        })
        .collect();
    serde_json::json!({
        "version": table.version,
        "as_of": as_of,
        "tiers": tiers,
        "aliases": table.aliases,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn propose_reads_the_cache() {
        let iso = crate::test_support::IsolatedConfigDir::enter();
        let models = |ids: &[&str]| -> Vec<serde_json::Value> {
            ids.iter().map(|id| serde_json::json!({"id": id})).collect()
        };
        let doc = serde_json::json!({
            "fetched_at": "2026-11-10T00:00:00Z",
            "providers": {
                "anthropic": models(&["claude-haiku-9-1", "claude-sonnet-9-2", "claude-opus-9-3"]),
                "google": models(&["gemini-9.4-flash", "gemini-9.5-pro"]),
                "openai": models(&["gpt-9-luna", "gpt-9.1-sol", "gpt-9.2-astra"]),
            },
        });
        std::fs::write(
            iso.config_dir().join(crate::config::MODELS_CACHE_FILE),
            serde_json::to_string(&doc).unwrap(),
        )
        .unwrap();
        let expected = serde_json::json!({
            "version": 1,
            "as_of": "2031-01-02",
            "tiers": {
                "anthropic": {"fast": "claude-haiku-9-1", "pro": "claude-sonnet-9-2", "max": "claude-opus-9-3"},
                "google": {"fast": "gemini-9.4-flash", "pro": "gemini-9.5-pro"},
                "openai": {"fast": "gpt-9-luna", "pro": "gpt-9.1-sol", "max": "gpt-9.2-astra"},
            },
            "aliases": model_table().aliases,
        });
        assert_eq!(propose_tier_table("2031-01-02"), expected);
    }

    #[test]
    fn the_embedded_table_reads() {
        let t = model_table();
        assert_eq!(t.version, 1);
        let b = t.as_of.as_bytes();
        assert_eq!(b.len(), 10);
        assert!(
            b.iter().enumerate().all(|(i, c)| if i == 4 || i == 7 {
                *c == b'-'
            } else {
                c.is_ascii_digit()
            }),
            "as_of is not AAAA-MM-JJ: {}",
            t.as_of
        );
        for vendor in ["anthropic", "google", "openai"] {
            assert!(t.tiers.contains_key(vendor), "missing vendor {vendor}");
        }
    }

    #[test]
    fn every_table_entry_classifies_to_its_tier() {
        use crate::model_resolution::{ModelTier, classify_model_tier, effective_tier};
        let mut seen = 0;
        for (vendor, row) in &model_table().tiers {
            let entries = [
                (ModelTier::Fast, Some(&row.fast)),
                (ModelTier::Pro, Some(&row.pro)),
                (ModelTier::Max, row.max.as_ref()),
            ];
            for (tier, id) in entries {
                let Some(id) = id else { continue };
                assert!(!id.contains("latest"), "{vendor} {tier:?}: {id}");
                assert_eq!(
                    classify_model_tier(id, vendor),
                    Some(effective_tier(vendor, tier)),
                    "{vendor} {tier:?}: {id}"
                );
                seen += 1;
            }
        }
        assert_eq!(seen, 8, "the table has eight entries");
    }
}
