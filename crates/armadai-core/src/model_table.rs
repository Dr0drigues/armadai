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

#[cfg(test)]
mod tests {
    use super::*;

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
}
