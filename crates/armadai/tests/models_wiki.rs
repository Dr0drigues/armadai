//! The wiki shows the table built into the binary: a stale page fails here.

use armadai_core::model_resolution::{ModelTier, fallback_model_for_tier};
use armadai_core::model_table::model_table;

fn rendered() -> String {
    let t = model_table();
    let mut out = format!(
        "Table as of {}.\n\n| Vendor | `fast` | `pro` | `max` |\n|---|---|---|---|\n",
        t.as_of
    );
    for vendor in t.tiers.keys() {
        let cell = |tier| format!("`{}`", fallback_model_for_tier(vendor, tier));
        out.push_str(&format!(
            "| `{vendor}` | {} | {} | {} |\n",
            cell(ModelTier::Fast),
            cell(ModelTier::Pro),
            cell(ModelTier::Max)
        ));
    }
    out
}

#[test]
fn the_wiki_shows_the_embedded_table() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/wiki/models.md");
    let wiki = std::fs::read_to_string(path).unwrap();
    let block = wiki
        .split("<!-- tier-table:start -->\n")
        .nth(1)
        .and_then(|r| r.split("<!-- tier-table:end -->").next())
        .expect("the tier-table markers are in docs/wiki/models.md");
    assert_eq!(block, rendered());
}

/// `models tiers` prints the embedded table: title with its date, then the rows.
#[test]
fn models_tiers_prints_the_embedded_table() {
    let home = tempfile::tempdir().unwrap();
    let out = assert_cmd::Command::cargo_bin("armadai")
        .unwrap()
        .env("HOME", home.path())
        .env("ARMADAI_CONFIG_DIR", home.path().join("config"))
        .env("NO_COLOR", "1")
        .args(["models", "tiers"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    let mut lines = stdout.lines().filter(|l| !l.is_empty());
    assert_eq!(
        lines.next().unwrap(),
        format!(
            "Tier table built into armadai, as of {}:",
            model_table().as_of
        )
    );
    assert!(lines.next().unwrap().starts_with("  vendor"));
}
