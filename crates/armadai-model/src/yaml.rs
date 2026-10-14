//! The workspace's single YAML entry point: serde-saphyr with fixed options.
//!
//! Reading:
//! - duplicate keys are rejected everywhere (typed fields and free maps alike), at the
//!   second occurrence;
//! - `<<` is an ordinary key (no merge); unknown tags (`!foo`, `!env`) are refused;
//! - plain scalars the previous reader (serde_yaml_ng) kept as text stay text where saphyr
//!   would read a number, a boolean or null: leading zeros (`0123`), `_` separators
//!   (`1_000`), upper-case prefixes (`0X1F`), mixed-case `tRuE`/`nUlL`, and number
//!   literals beyond 64-bit integers or `f64` (kept as their exact text, never rounded);
//!   A typed number or boolean field still parses that text as saphyr does: `1_000`,
//!   `0X10`, `0b1_0` and `tRuE` are accepted there (the previous reader refused them),
//!   `0123` is not;
//! - in a free (`native`) value `.inf`/`.nan` are refused (JSON data has no such number);
//! - mappings nested deeper than 64 levels are refused (the previous reader allowed 128);
//! - `!!binary` is base64-decoded (it was kept as its base64 text);
//! - `null`/`~` where a string is required is an error (it was the text `null`/`~`).
//!
//! Writing never folds long strings, and quotes any string that would read back as
//! something else.

use std::borrow::Cow;
use std::fmt;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_saphyr::granit_parser::{Event, Parser, ScalarStyle};

pub use serde_saphyr::SerializeError;

/// A read error with its position; the message carries no advice on saphyr's options.
#[derive(Debug)]
pub struct Error(serde_saphyr::Error);

impl Error {
    pub fn location(&self) -> Option<serde_saphyr::Location> {
        self.0.location()
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let msg = self.0.to_string();
        f.write_str(
            &msg.replace(", set DuplicateKeyPolicy in Options if acceptable", "")
                .replace("; use Option<String>", ""),
        )
    }
}

impl std::error::Error for Error {}

pub fn from_str<T: DeserializeOwned>(s: &str) -> Result<T, Error> {
    let mut o = serde_saphyr::Options::default();
    o.strict_booleans = true;
    o.with_snippet = false;
    o.reject_unsupported_tags = true;
    o.merge_keys = serde_saphyr::MergeKeyPolicy::AsOrdinary;
    serde_saphyr::from_str_with_options(&keep_text(s), o).map_err(Error)
}

pub fn to_string<T: Serialize>(v: &T) -> Result<String, SerializeError> {
    let mut o = serde_saphyr::SerializerOptions::default();
    // never fold long strings over several lines
    o.folded_wrap_chars = usize::MAX;
    serde_saphyr::to_string_with_options(v, o)
}

/// `s` with every plain, untagged scalar that the previous reader kept as text
/// single-quoted (`'` doubled), so saphyr reads it as text too. Quoting shifts the columns
/// that follow on the same line by two, plus one per `'` in the scalar. Input that does not parse is returned as is: saphyr reports it.
fn keep_text(s: &str) -> Cow<'_, str> {
    let mut spans = Vec::new();
    for ev in Parser::new_from_str(s) {
        let Ok((ev, span)) = ev else {
            return Cow::Borrowed(s);
        };
        let Event::Scalar(text, ScalarStyle::Plain, _, None) = ev else {
            continue;
        };
        let (Some(a), Some(b)) = (span.start.byte_offset(), span.end.byte_offset()) else {
            continue;
        };
        // only a scalar whose span is exactly its text is rewritten
        if s.get(a..b) == Some(&*text) && was_text(&text) {
            spans.push((a, b));
        }
    }
    if spans.is_empty() {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len() + 4 * spans.len());
    let mut at = 0;
    for (a, b) in spans {
        out.push_str(&s[at..a]);
        out.push('\'');
        out.push_str(&s[a..b].replace('\'', "''"));
        out.push('\'');
        at = b;
    }
    out.push_str(&s[at..]);
    Cow::Owned(out)
}

/// A plain scalar serde_yaml_ng read as a string but saphyr reads as a number, a boolean
/// or null, or refuses (a literal beyond `f64`).
fn was_text(t: &str) -> bool {
    let unsigned = t.strip_prefix(['-', '+']).unwrap_or(t);
    let numeric = unsigned.starts_with(|c: char| c.is_ascii_digit() || c == '.');
    let all_digits = !unsigned.is_empty() && unsigned.bytes().all(|c| c.is_ascii_digit());
    // `true`, `True` and `TRUE` are the only spellings serde_yaml_ng resolved
    let mixed_case =
        |w: &str| t.eq_ignore_ascii_case(w) && t != w && t != w.to_uppercase() && t[1..] != w[1..];
    (numeric && t.contains('_'))
        || (all_digits && unsigned.len() > 1 && unsigned.starts_with('0'))
        || ["0X", "0O", "0B"].iter().any(|p| unsigned.starts_with(p))
        || mixed_case("true")
        || mixed_case("false")
        || mixed_case("null")
        || (all_digits && t.parse::<i64>().is_err() && t.parse::<u64>().is_err())
        || (numeric && t.parse::<f64>().is_ok_and(f64::is_infinite))
}
