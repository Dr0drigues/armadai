//! `armadai migrate` end to end, on copies of the legacy fixtures.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use tempfile::TempDir;

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

/// A scratch copy of `tests/fixtures/migrate/<case>`.
fn fixture(case: &str) -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    let src = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/migrate")
        .join(case);
    copy_tree(&src, dir.path());
    dir
}

/// Every file under `root`, relative path -> bytes.
fn tree(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                let rel = path.strip_prefix(root).unwrap().to_path_buf();
                out.insert(rel, std::fs::read(&path).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
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

fn migrate(dir: &TempDir, extra: &[&str]) -> assert_cmd::assert::Assert {
    let mut args = vec!["migrate"];
    args.extend_from_slice(extra);
    args.push("--path");
    armadai(&args, dir.path())
}

fn stdout(a: assert_cmd::assert::Assert) -> String {
    String::from_utf8_lossy(&a.get_output().stdout).into_owned()
}

/// The single backup directory under `.armadai/.migrated/`.
fn backup(dir: &TempDir) -> PathBuf {
    let base = dir.path().join(".armadai/.migrated");
    let mut entries: Vec<PathBuf> = std::fs::read_dir(&base)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(entries.len(), 1, "{entries:?}");
    entries.pop().unwrap()
}

/// `migrate`, then `armadai validate` on the result exits 0.
fn migrates_and_validates(case: &str) -> (TempDir, String) {
    let dir = fixture(case);
    let out = stdout(migrate(&dir, &[]).success());
    assert!(dir.path().join(".armadai/project.yaml").is_file(), "{case}");
    armadai(&["validate"], dir.path()).success();
    (dir, out)
}

#[test]
fn armadai_dev_migrates_and_validates() {
    let (dir, _) = migrates_and_validates("armadai-dev");
    let m = std::fs::read_to_string(dir.path().join(".armadai/project.yaml")).unwrap();
    assert!(m.contains("coordinator: dev-lead"), "{m}");
}

#[test]
fn code_analysis_rust_migrates_and_validates() {
    migrates_and_validates("code-analysis-rust");
}

#[test]
fn declarative_migrates_and_validates() {
    let (dir, _) = migrates_and_validates("declarative");
    assert!(
        dir.path()
            .join(".armadai/skills/review-checklist/scripts/check.sh")
            .is_file()
    );
    assert!(!dir.path().join(".armadai/agents.yaml").exists());
    let skill =
        std::fs::read_to_string(dir.path().join(".armadai/skills/review-checklist/SKILL.md"))
            .unwrap();
    assert!(
        !skill.contains("tools:") && skill.contains("license: MIT"),
        "{skill}"
    );
    // fragments composed into declared agents are sources, not unconverted files
    let out = stdout(migrate(&fixture("declarative"), &["--dry-run"]).success());
    assert!(
        !out.contains("kept only in the backup: .armadai/prompts/"),
        "{out}"
    );
}

#[test]
fn demo_rust_team_migrates_and_validates() {
    migrates_and_validates("demo-rust-team");
}

#[test]
fn fullstack_migrates_and_validates() {
    let (dir, out) = migrates_and_validates("fullstack");
    assert!(
        dir.path()
            .join(".armadai/flows/orchestration.yaml")
            .is_file()
    );
    assert!(out.contains("ghost-agent"), "{out}");
}

#[test]
fn rust_dev_migrates_and_validates() {
    let (dir, _) = migrates_and_validates("rust-dev");
    assert!(!dir.path().join("armadai.yaml").exists());
}

#[test]
fn report_lists_written_dropped_warnings_and_the_backup() {
    let (_, out) = migrates_and_validates("fullstack");
    for needle in [
        "Written (",
        ".armadai/project.yaml",
        "Dropped (",
        "provider: dropped for 6 agents",
        "orchestration: max_depth",
        "Warnings (",
        "YAML comments",
        "not detected",
        "Note: run 'armadai link'",
        ".armadai/.migrated/",
    ] {
        assert!(out.contains(needle), "{needle}:\n{out}");
    }
}

#[test]
fn dry_run_writes_nothing() {
    let dir = fixture("fullstack");
    let before = tree(dir.path());
    let out = stdout(migrate(&dir, &["--dry-run"]).success());
    assert_eq!(tree(dir.path()), before);
    assert!(!dir.path().join(".armadai/.migrated").exists());
    assert!(out.starts_with("Dry run: nothing was written."), "{out}");
    assert!(out.contains(".armadai/agents/code-reviewer.md"), "{out}");
}

#[test]
fn second_run_is_refused() {
    let dir = fixture("rust-dev");
    migrate(&dir, &[]).success();
    let before = tree(dir.path());
    let err = migrate(&dir, &[]).failure().get_output().stderr.clone();
    assert!(
        String::from_utf8_lossy(&err).contains("already migrated (use --force to redo it)"),
        "{}",
        String::from_utf8_lossy(&err)
    );
    assert_eq!(tree(dir.path()), before);
}

#[test]
fn force_redoes_it() {
    let dir = fixture("demo-rust-team");
    migrate(&dir, &[]).success();
    // the user restores the legacy files from the backup, then redoes the migration
    copy_tree(&backup(&dir), dir.path());
    migrate(&dir, &["--force"]).success();
    armadai(&["validate"], dir.path()).success();
    let backups = std::fs::read_dir(dir.path().join(".armadai/.migrated"))
        .unwrap()
        .count();
    assert_eq!(backups, 2);
}

#[test]
fn backup_exists_and_contains_the_original_config() {
    let dir = fixture("demo-rust-team");
    let before = tree(dir.path());
    migrate(&dir, &[]).success();
    let b = backup(&dir);
    for rel in [
        ".armadai/config.yaml",
        ".armadai/agents/rust-reviewer.md",
        ".armadai/agents/doc-writer.md",
    ] {
        assert_eq!(
            std::fs::read(b.join(rel)).unwrap(),
            before[Path::new(rel)],
            "{rel}"
        );
    }
    assert!(!dir.path().join(".armadai/config.yaml").exists());
}

#[test]
fn legacy_file_outside_armadai_is_left_and_reported() {
    let dir = fixture("code-analysis-rust");
    let original = std::fs::read(dir.path().join("agents/lead-analyst.md")).unwrap();
    let out = stdout(migrate(&dir, &[]).success());
    assert_eq!(
        std::fs::read(dir.path().join("agents/lead-analyst.md")).unwrap(),
        original
    );
    assert!(
        out.contains("legacy file left in place: agents/lead-analyst.md"),
        "{out}"
    );
    assert!(backup(&dir).join("agents/lead-analyst.md").is_file());
    assert!(!dir.path().join("armadai.yml").exists());
}

#[test]
fn unreferenced_legacy_file_is_reported_and_kept_in_the_backup() {
    let dir = fixture("rust-dev");
    std::fs::write(dir.path().join(".armadai/agents/stray.md"), "# Stray\n").unwrap();
    let out = stdout(migrate(&dir, &[]).success());
    assert!(
        out.contains("not converted, kept only in the backup: .armadai/agents/stray.md"),
        "{out}"
    );
    assert!(backup(&dir).join(".armadai/agents/stray.md").is_file());
    assert!(!dir.path().join(".armadai/agents/stray.md").exists());
}

#[test]
fn no_legacy_project_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let err = migrate(&dir, &[]).failure().get_output().stderr.clone();
    assert!(String::from_utf8_lossy(&err).contains("No legacy project found"));
}

/// Every legacy agent `.md` of the fixture, as (H1 title, text).
fn legacy_agents(case: &str) -> Vec<(String, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/migrate")
        .join(case);
    let mut out = Vec::new();
    for (rel, bytes) in tree(&root) {
        let is_agent = rel.extension().is_some_and(|e| e == "md")
            && rel.parent().is_some_and(|p| p.ends_with("agents"));
        if is_agent {
            let text = String::from_utf8(bytes).unwrap();
            let title = text
                .lines()
                .find_map(|l| l.strip_prefix("# "))
                .unwrap()
                .trim()
                .to_string();
            out.push((title, text));
        }
    }
    out
}

/// Paragraphs (blank-line separated, fences kept whole) with the H2 they sit under.
fn paragraphs(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let (mut section, mut current, mut fence) = (String::new(), Vec::new(), false);
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            fence = !fence;
        }
        if !fence && line.trim().is_empty() {
            if !current.is_empty() {
                out.push((section.clone(), current.join("\n")));
                current.clear();
            }
            continue;
        }
        if !fence && let Some(h) = line.strip_prefix("## ") {
            section = h.trim().to_lowercase();
        }
        current.push(line);
    }
    if !current.is_empty() {
        out.push((section, current.join("\n")));
    }
    out
}

/// Every H2 and every non-empty paragraph of each legacy agent is in its new file,
/// or named in the report.
fn assert_nothing_lost(case: &str) {
    let dir = fixture(case);
    let report = stdout(migrate(&dir, &[]).success());
    let new: Vec<String> = tree(&dir.path().join(".armadai/agents"))
        .into_values()
        .map(|b| String::from_utf8(b).unwrap())
        .collect();
    let structural = [
        "system prompt",
        "metadata",
        "pipeline",
        "triggers",
        "ring config",
    ];
    for (title, text) in legacy_agents(case) {
        let file = new
            .iter()
            .find(|f| f.contains(&format!("\ntitle: {title}\n")))
            .unwrap_or_else(|| panic!("{case}: no migrated agent titled {title}"));
        for (section, para) in paragraphs(&text) {
            let mut lines: Vec<&str> = para.lines().collect();
            if lines[0].starts_with("# ") {
                lines.remove(0);
            }
            if lines.first().is_some_and(|l| {
                l.strip_prefix("## ")
                    .is_some_and(|h| structural.contains(&h.trim().to_lowercase().as_str()))
            }) {
                lines.remove(0);
            }
            if lines.is_empty() {
                continue;
            }
            match section.as_str() {
                "metadata" => {
                    for l in lines {
                        let key = l.trim_start_matches('-').split(':').next().unwrap().trim();
                        assert!(
                            ["model", "temperature", "tags"].contains(&key) || report.contains(key),
                            "{case}/{title}: metadata `{key}` lost"
                        );
                    }
                }
                "pipeline" | "triggers" | "ring config" => assert!(
                    report.contains(&section.replace(' ', "_")),
                    "{case}/{title}: ## {section} not reported"
                ),
                _ => {
                    let para = lines.join("\n");
                    assert!(
                        file.contains(para.trim()),
                        "{case}/{title}: lost paragraph:\n{para}"
                    );
                }
            }
        }
    }
}

#[test]
fn no_text_is_lost_in_any_fixture() {
    for case in [
        "armadai-dev",
        "code-analysis-rust",
        "declarative",
        "demo-rust-team",
        "fullstack",
        "rust-dev",
    ] {
        assert_nothing_lost(case);
    }
}

#[test]
fn unread_legacy_config_file_is_left_in_place() {
    let dir = fixture("demo-rust-team");
    std::fs::write(dir.path().join("armadai.yaml"), "agents: []\n").unwrap();
    let out = stdout(migrate(&dir, &[]).success());
    assert!(dir.path().join("armadai.yaml").is_file());
    assert!(
        out.contains("legacy file left in place: armadai.yaml"),
        "{out}"
    );
    assert!(!dir.path().join(".armadai/config.yaml").exists());
}

#[cfg(unix)]
#[test]
fn a_failure_after_the_backup_names_it() {
    use std::os::unix::fs::PermissionsExt;
    let dir = fixture("demo-rust-team");
    let agents = dir.path().join(".armadai/agents");
    std::fs::set_permissions(&agents, std::fs::Permissions::from_mode(0o555)).unwrap();
    let a = migrate(&dir, &[]).failure();
    std::fs::set_permissions(&agents, std::fs::Permissions::from_mode(0o755)).unwrap();
    let err = String::from_utf8_lossy(&a.get_output().stderr).into_owned();
    assert!(
        err.contains("the previous project is in .armadai/.migrated/"),
        "{err}"
    );
    assert!(backup(&dir).join(".armadai/config.yaml").is_file());
}

/// A duplicated key is named with its position, without saphyr's advice on its options.
#[test]
fn a_duplicated_key_in_agents_yaml_is_reported_plainly() {
    let dir = fixture("declarative");
    let path = dir.path().join(".armadai/agents.yaml");
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(
        &path,
        text.replacen(
            "  provider: claude\n",
            "  provider: claude\n  provider: codex\n",
            1,
        ),
    )
    .unwrap();
    let out = migrate(&dir, &[]).failure().get_output().stderr.clone();
    let err = String::from_utf8_lossy(&out);
    assert!(
        err.contains("duplicate mapping key: provider at line 3, column 3"),
        "{err}"
    );
    assert!(!err.contains("DuplicateKeyPolicy"), "{err}");
}
