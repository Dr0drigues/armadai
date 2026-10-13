//! The workspace's single YAML entry point: serde-saphyr with fixed options.
//!
//! Duplicate keys are rejected everywhere (typed fields and free maps alike), with the
//! position of the second occurrence. Only `true`/`false` are booleans, `<<` is an ordinary
//! key (no merge), and unknown tags (`!foo`) are refused.

use serde::Serialize;
use serde::de::DeserializeOwned;

pub use serde_saphyr::{Error, SerializeError};

pub fn from_str<T: DeserializeOwned>(s: &str) -> Result<T, Error> {
    let mut o = serde_saphyr::Options::default();
    o.strict_booleans = true;
    o.with_snippet = false;
    o.reject_unsupported_tags = true;
    o.merge_keys = serde_saphyr::MergeKeyPolicy::AsOrdinary;
    serde_saphyr::from_str_with_options(s, o)
}

pub fn to_string<T: Serialize>(v: &T) -> Result<String, SerializeError> {
    let mut o = serde_saphyr::SerializerOptions::default();
    // never fold long strings over several lines
    o.folded_wrap_chars = usize::MAX;
    serde_saphyr::to_string_with_options(v, o)
}
