//! Golden snapshots of `armadai init --pack <name>` then `armadai link` for the 7 embedded
//! packs. The `.armadai/` tree is checked against the pack itself (a native pack is copied,
//! byte for byte, by a white list); the snapshot holds what the user sees and what Claude
//! Code receives: the init and link outputs, and every file link writes.
//! Regenerate with `UPDATE_GOLDEN=1 cargo test -p armadai --test init_pack_golden` ONLY when
//! a change is deliberate, re-read every changed file, and say so in the PR body.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use assert_cmd::Command;

const PACKS: [&str; 7] = [
    "armadai-authoring",
    "armadai-dev",
    "code-analysis-rust",
    "code-analysis-web",
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

#[test]
fn init_then_link_matches_the_golden_snapshots() {
    let golden = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/init_pack");
    let update = std::env::var("UPDATE_GOLDEN").is_ok_and(|v| !v.is_empty() && v != "0");
    for pack in PACKS {
        let snap = snapshot(pack);
        let path = golden.join(format!("{pack}.txt"));
        if update {
            std::fs::create_dir_all(&golden).unwrap();
            std::fs::write(&path, &snap).unwrap();
            continue;
        }
        let expected = std::fs::read_to_string(&path).unwrap_or_else(|_| {
            panic!(
                "missing snapshot {} — generate it with UPDATE_GOLDEN=1",
                path.display()
            )
        });
        assert_eq!(
            snap, expected,
            "{pack}: init/link output changed. If deliberate, regenerate with UPDATE_GOLDEN=1 \
             and justify it in the PR."
        );
    }
}
