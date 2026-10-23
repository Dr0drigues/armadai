//! Golden snapshots of `armadai init --pack <name>` then `armadai link` for the 8 embedded
//! packs. The `.armadai/` tree is checked against the pack itself (a native pack is copied,
//! byte for byte, by a white list); the snapshot holds what the user sees and what Claude
//! Code receives: the init and link outputs, and every file link writes.
//! Regenerate with `UPDATE_GOLDEN=1 cargo test -p armadai --test init_pack_golden` ONLY when
//! a change is deliberate, re-read every changed file, and say so in the PR body.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use assert_cmd::Command;

const PACKS: [&str; 8] = [
    "armadai-authoring",
    "armadai-dev",
    "code-analysis-rust",
    "code-analysis-web",
    "concept-to-code",
    "fullstack",
    "pirate-crew",
    "rust-dev",
];

fn files(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for e in entries {
            let path = e.unwrap().path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                let rel = path.strip_prefix(root).unwrap().to_path_buf();
                out.insert(rel, std::fs::read(&path).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

fn armadai(root: &Path, config: &Path, args: &[&str]) -> String {
    let out = Command::cargo_bin("armadai")
        .unwrap()
        .current_dir(root)
        .env("ARMADAI_CONFIG_DIR", config)
        .env("HOME", config)
        .env("NO_COLOR", "1")
        .args(args)
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "{args:?}: {text}");
    // the canonical form first: on macOS the temporary folder is /private/var/...
    let real = root.canonicalize().unwrap();
    text.replace(real.to_str().unwrap(), "<root>")
        .replace(root.to_str().unwrap(), "<root>")
}

/// One text file: the init output, the link output, then each file link wrote.
fn snapshot(pack: &str) -> String {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("project");
    let config = tmp.path().join("config");
    std::fs::create_dir_all(root.join(".git")).unwrap();
    std::fs::create_dir_all(&config).unwrap();
    let init = armadai(&root, &config, &["init", "--pack", pack]);

    // the tree is the pack, copied: every listed file, byte for byte
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../armadai-core/starters")
        .join(pack);
    let mut installed = files(&root.join(".armadai"));
    assert!(
        installed.remove(Path::new(".gitignore")).is_some(),
        "{pack}"
    );
    let mut expected = files(&source);
    expected.remove(Path::new("pack.yaml"));
    assert_eq!(
        installed, expected,
        "{pack}: the installed tree is not the pack"
    );

    let link = armadai(&root, &config, &["link", "--target", "claude"]);
    let mut out =
        format!("$ armadai init --pack {pack}\n{init}\n$ armadai link --target claude\n{link}");
    for (rel, bytes) in files(&root) {
        if rel.starts_with(".armadai") || rel.starts_with(".git") {
            continue;
        }
        out.push_str(&format!(
            "\n=== {}\n{}",
            rel.display(),
            String::from_utf8_lossy(&bytes)
        ));
    }
    out
}

/// `snap` against `path`, or written there under `UPDATE_GOLDEN` (set, not empty,
/// not `0`).
fn check_golden(path: &Path, snap: &str) {
    if std::env::var("UPDATE_GOLDEN").is_ok_and(|v| !v.is_empty() && v != "0") {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, snap).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(path).unwrap_or_else(|_| {
        panic!(
            "missing snapshot {} — generate it with UPDATE_GOLDEN=1",
            path.display()
        )
    });
    assert_eq!(
        snap,
        expected,
        "{}: output changed. If deliberate, regenerate with UPDATE_GOLDEN=1 and justify it \
         in the PR.",
        path.display()
    );
}

#[test]
fn init_then_link_matches_the_golden_snapshots() {
    let golden = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/init_pack");
    for pack in PACKS {
        let snap = snapshot(pack);
        check_golden(&golden.join(format!("{pack}.txt")), &snap);
    }
}

/// The frontmatter keys OpenCode 2.x decodes for an agent
/// (https://opencode.ai/docs/agents/, v1 agent schema of the 2.x code): any other
/// key goes to the provider as a model option. Same list as the adapter's unit test.
const OPENCODE_AGENT_KEYS: [&str; 10] = [
    "description",
    "mode",
    "model",
    "temperature",
    "top_p",
    "steps",
    "disable",
    "hidden",
    "color",
    "permission",
];

/// The top-level keys of a generated agent's frontmatter.
fn frontmatter_keys(agent: &str) -> Vec<String> {
    let yaml = agent
        .strip_prefix("---\n")
        .and_then(|r| r.split_once("\n---\n"))
        .unwrap_or_else(|| panic!("no frontmatter:\n{agent}"))
        .0;
    let map: serde_json::Map<String, serde_json::Value> =
        armadai_model::yaml::from_str(yaml).unwrap();
    map.into_iter().map(|(k, _)| k).collect()
}

/// Each pack links to OpenCode (dry run, then for real) and every agent written
/// carries only keys OpenCode 2.x knows; the files and the report are frozen per pack
/// (`tests/golden/init_pack_opencode/`).
#[test]
fn every_pack_links_to_opencode_with_known_keys_only() {
    let golden = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/init_pack_opencode");
    for pack in PACKS {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("project");
        let config = tmp.path().join("config");
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::create_dir_all(&config).unwrap();
        armadai(&root, &config, &["init", "--pack", pack]);
        armadai(
            &root,
            &config,
            &["link", "--target", "opencode", "--dry-run"],
        );
        let link = armadai(&root, &config, &["link", "--target", "opencode"]);

        let agents = files(&root.join(".opencode/agents"));
        assert!(!agents.is_empty(), "{pack}: no agent written");
        for (rel, bytes) in &agents {
            for k in frontmatter_keys(&String::from_utf8_lossy(bytes)) {
                assert!(
                    OPENCODE_AGENT_KEYS.contains(&k.as_str()),
                    "{pack}: {}: key `{k}` unknown to OpenCode 2.x",
                    rel.display()
                );
            }
        }

        let mut snap = format!("$ armadai link --target opencode\n{link}");
        for (rel, bytes) in files(&root) {
            if rel.starts_with(".armadai") || rel.starts_with(".git") {
                continue;
            }
            snap.push_str(&format!(
                "\n=== {}\n{}",
                rel.display(),
                String::from_utf8_lossy(&bytes)
            ));
        }
        check_golden(&golden.join(format!("{pack}.txt")), &snap);
    }
}
