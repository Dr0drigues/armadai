//! Golden snapshots of `armadai migrate` on the six legacy fixtures: the `.armadai/` tree it
//! writes (backup excluded) and its report. Regenerate with
//! `UPDATE_GOLDEN=1 cargo test -p armadai --test migrate_golden` ONLY when a change to the
//! conversion is deliberate, re-read every changed file, and say so in the PR body.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use assert_cmd::Command;

const CASES: [&str; 6] = [
    "armadai-dev",
    "code-analysis-rust",
    "declarative",
    "demo-rust-team",
    "fullstack",
    "rust-dev",
];

/// Stands for `.armadai/.migrated/<unix seconds>` in `report.txt`.
const BACKUP_TOKEN: &str = ".armadai/.migrated/<timestamp>";

fn copy_tree(from: &Path, to: &Path) {
    for entry in std::fs::read_dir(from).unwrap() {
        let path = entry.unwrap().path();
        let dest = to.join(path.file_name().unwrap());
        if path.is_dir() {
            std::fs::create_dir_all(&dest).unwrap();
            copy_tree(&path, &dest);
        } else {
            std::fs::copy(&path, &dest).unwrap();
        }
    }
}

/// Files under `dir`, relative to `dir`, skipping `skip` subtrees.
fn files(dir: &Path, skip: &[&str]) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, skip: &[&str], out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for e in entries {
            let path = e.unwrap().path();
            let rel = path.strip_prefix(root).unwrap().to_path_buf();
            if skip.iter().any(|s| rel == Path::new(s)) {
                continue;
            }
            if path.is_dir() {
                walk(root, &path, skip, out);
            } else {
                out.insert(rel, std::fs::read(&path).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, skip, &mut out);
    out
}

fn armadai(args: &[&str], dir: &Path) -> assert_cmd::assert::Assert {
    let cfg = tempfile::tempdir().unwrap();
    Command::cargo_bin("armadai")
        .unwrap()
        .env("ARMADAI_CONFIG_DIR", cfg.path())
        .env("NO_COLOR", "1")
        .args(args)
        .arg(dir)
        .assert()
}

fn fixture_copy(case: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let src = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/migrate")
        .join(case);
    copy_tree(&src, dir.path());
    dir
}

/// The snapshot of one migration: `.armadai/` files plus `report.txt`.
fn snapshot(case: &str) -> BTreeMap<PathBuf, Vec<u8>> {
    let dir = fixture_copy(case);
    let out = armadai(&["migrate", "--path"], dir.path()).success();
    let report = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let migrated = dir.path().join(".armadai/.migrated");
    let backups: Vec<PathBuf> = std::fs::read_dir(&migrated)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(backups.len(), 1, "{case}");
    let stamp = backups[0].file_name().unwrap().to_str().unwrap();
    let report = report.replace(&format!(".armadai/.migrated/{stamp}"), BACKUP_TOKEN);
    assert!(
        !report.contains(dir.path().to_str().unwrap()),
        "{case}: report has an absolute path"
    );

    armadai(&["validate"], dir.path()).success();
    // second run refused, nothing touched
    let before = files(dir.path(), &[]);
    armadai(&["migrate", "--path"], dir.path()).failure();
    assert_eq!(files(dir.path(), &[]), before, "{case}");

    let mut snap: BTreeMap<PathBuf, Vec<u8>> = files(&dir.path().join(".armadai"), &[".migrated"])
        .into_iter()
        .map(|(p, b)| (Path::new(".armadai").join(p), b))
        .collect();
    snap.insert("report.txt".into(), report.into_bytes());
    snap
}

#[test]
fn migrate_output_matches_the_golden_snapshots() {
    for case in CASES {
        let snap = snapshot(case);
        let golden = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/golden/migrate")
            .join(case);
        if std::env::var("UPDATE_GOLDEN").is_ok_and(|v| !v.is_empty() && v != "0") {
            let _ = std::fs::remove_dir_all(&golden);
            for (rel, bytes) in &snap {
                let p = golden.join(rel);
                std::fs::create_dir_all(p.parent().unwrap()).unwrap();
                std::fs::write(p, bytes).unwrap();
            }
            continue;
        }
        let expected = files(&golden, &[]);
        assert!(
            !expected.is_empty(),
            "missing snapshot {} — generate it with UPDATE_GOLDEN=1",
            golden.display()
        );
        assert_eq!(
            snap.keys().collect::<Vec<_>>(),
            expected.keys().collect::<Vec<_>>(),
            "{case}: the set of written files changed"
        );
        for (rel, bytes) in &snap {
            assert_eq!(
                String::from_utf8_lossy(bytes),
                String::from_utf8_lossy(&expected[rel]),
                "{case}/{}: migrate output changed. If deliberate, regenerate with \
                 UPDATE_GOLDEN=1 and justify it in the PR.",
                rel.display()
            );
        }
    }
}

#[test]
fn dry_run_leaves_every_fixture_untouched() {
    for case in CASES {
        let dir = fixture_copy(case);
        let before = files(dir.path(), &[]);
        armadai(&["migrate", "--dry-run", "--path"], dir.path()).success();
        assert_eq!(files(dir.path(), &[]), before, "{case}");
    }
}
