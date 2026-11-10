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
    // Rebuilt from the embedded table, not from the code under test.
    let t = model_table();
    let mut rows = vec![["vendor", "fast", "pro", "max"].map(String::from)];
    for vendor in t.tiers.keys() {
        rows.push([
            vendor.to_string(),
            fallback_model_for_tier(vendor, ModelTier::Fast).to_string(),
            fallback_model_for_tier(vendor, ModelTier::Pro).to_string(),
            fallback_model_for_tier(vendor, ModelTier::Max).to_string(),
        ]);
    }
    let w: Vec<usize> = (0..4)
        .map(|i| rows.iter().map(|r| r[i].len()).max().unwrap())
        .collect();
    let mut want = vec![
        format!("Tier table built into armadai, as of {}:", t.as_of),
        String::new(),
    ];
    for r in &rows {
        let cells: Vec<String> = (0..4).map(|i| format!("{:<1$}", r[i], w[i])).collect();
        want.push(format!("  {}", cells.join("  ").trim_end()));
    }
    want.push(String::new());
    want.push("`link --target codex` uses the openai row for an agent without native.codex.model; link never reads the models cache.".into());
    assert_eq!(stdout.lines().collect::<Vec<_>>(), want);
}

#[cfg(unix)]
fn run_sync(curl_body: &str) -> (std::process::Output, std::path::PathBuf, tempfile::TempDir) {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().to_path_buf();
    let bin = home.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let curl = bin.join("curl");
    std::fs::write(
        &curl,
        format!(
            "#!/bin/sh\nwhile [ $# -gt 1 ]; do [ \"$1\" = -o ] && out=$2; shift; done\n{curl_body}\n"
        ),
    )
    .unwrap();
    std::fs::set_permissions(&curl, std::fs::Permissions::from_mode(0o755)).unwrap();
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());
    let out = assert_cmd::Command::cargo_bin("armadai")
        .unwrap()
        .env("HOME", &home)
        .env("ARMADAI_CONFIG_DIR", home.join("config"))
        .env("PATH", path)
        .env("NO_COLOR", "1")
        .args(["models", "sync"])
        .output()
        .unwrap();
    (out, home.join("config").join("models-cache.json"), tmp)
}

/// `models sync` through a fake curl: the success line, then the unchanged-table note.
#[cfg(unix)]
#[test]
fn models_sync_reports_what_it_wrote() {
    let (out, cache, _home) =
        run_sync(r#"printf '%s' '{"p":{"models":{"m":{"id":"m1"}}}}' > "$out""#);
    assert!(out.status.success(), "{out:?}");
    let stdout = String::from_utf8(out.stdout).unwrap();
    let lines: Vec<_> = stdout.lines().collect();
    let head = format!(
        "Synced 1 providers, 1 models from https://models.dev/api.json into {} (fetched ",
        cache.display()
    );
    assert!(
        lines[0].starts_with(&head) && lines[0].ends_with(" UTC)."),
        "{stdout}"
    );
    assert_eq!(
        lines[1],
        format!(
            "link does not read this cache: the tier table built into armadai (as of {}) is unchanged.",
            model_table().as_of
        )
    );
    assert_eq!(lines.len(), 2);
    assert!(cache.exists());
}

/// A failing download is one `Error:` line on stderr, exit 1, and no cache written.
#[cfg(unix)]
#[test]
fn models_sync_failure_is_one_error_line() {
    let (out, cache, _home) = run_sync("printf 'curl: (6) first\\nsecond\\n' >&2; exit 6");
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(out.stderr).unwrap(),
        format!(
            "Error: models sync failed, {} unchanged: download https://models.dev/api.json failed (exit status: 6): curl: (6) first; second\n",
            cache.display()
        )
    );
    assert!(!cache.exists());
}
