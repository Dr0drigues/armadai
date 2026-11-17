//! `link` and a `.claude/` or `.armadai/` that is a symbolic link leaving the checkout:
//! refused under a pack pin (the other checkout is left untouched), a note otherwise.
#![cfg(unix)]

use std::path::{Path, PathBuf};

use assert_cmd::Command;

const PIN: &str = "format: 1\npack: demo\nsource: https://example.com/packs.git\nresolved:\n  version: \"1.0.0\"\n  tag: demo-v1.0.0\n  commit: \"1111111111111111111111111111111111111111\"\n";
const AGENT: &str = "---\ndescription: Leads\n---\nYou lead.\n";

fn put(p: &Path, text: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

/// `chantier` (the other checkout, with its own `.claude/` and `.armadai/`) and `wt`
/// (this checkout, a `.git` folder, optionally pinned).
struct Env(tempfile::TempDir);

impl Env {
    fn new(pinned: bool) -> Env {
        let dir = tempfile::tempdir().unwrap();
        let env = Env(dir);
        for base in [env.chantier(), env.wt()] {
            std::fs::create_dir_all(base.join(".git")).unwrap();
        }
        put(&env.chantier().join(".claude/keep.txt"), "mine\n");
        put(
            &env.chantier().join(".armadai/project.yaml"),
            "schema_version: 1\n",
        );
        put(&env.chantier().join(".armadai/agents/lead.md"), AGENT);
        // a committed lock, so that "has a pin" and "has a lock" differ in every case
        put(&env.chantier().join(".armadai/pack.lock"), PIN);
        if pinned {
            put(&env.wt().join(".git/armadai/pack.lock"), PIN);
        }
        std::fs::create_dir_all(env.0.path().join("home")).unwrap();
        std::fs::create_dir_all(env.0.path().join("config")).unwrap();
        env
    }

    fn chantier(&self) -> PathBuf {
        self.0.path().join("chantier")
    }

    fn wt(&self) -> PathBuf {
        self.0.path().join("wt")
    }

    /// Every file under the chantier with its bytes, to compare before and after.
    fn snapshot(&self) -> Vec<(PathBuf, Vec<u8>)> {
        fn walk(d: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
            let mut es: Vec<_> = std::fs::read_dir(d)
                .unwrap()
                .map(|e| e.unwrap().path())
                .collect();
            es.sort();
            for p in es {
                if p.is_dir() {
                    walk(&p, out);
                } else {
                    out.push((p.clone(), std::fs::read(&p).unwrap()));
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.chantier(), &mut out);
        out
    }

    fn link(&self) -> (Option<i32>, String, String) {
        self.run(&self.wt(), "link")
    }

    fn run(&self, dir: &Path, cmd: &str) -> (Option<i32>, String, String) {
        let o = Command::cargo_bin("armadai")
            .unwrap()
            .current_dir(dir)
            .env("HOME", self.0.path().join("home"))
            .env("ARMADAI_CONFIG_DIR", self.0.path().join("config"))
            .env("CLAUDE_CONFIG_DIR", self.0.path().join("claude"))
            .env_remove("CLAUDE_CODE_PLUGIN_CACHE_DIR")
            .args(cmd.split_whitespace())
            .output()
            .unwrap();
        (
            o.status.code(),
            String::from_utf8_lossy(&o.stdout).into_owned(),
            String::from_utf8_lossy(&o.stderr).into_owned(),
        )
    }

    /// The wt's own project, for the cases where `.armadai` is not the link.
    fn own_project(&self) {
        put(
            &self.wt().join(".armadai/project.yaml"),
            "schema_version: 1\n",
        );
        put(&self.wt().join(".armadai/agents/lead.md"), AGENT);
        put(&self.wt().join(".armadai/pack.lock"), PIN);
    }

    fn link_to(&self, name: &str, target: &Path) {
        std::os::unix::fs::symlink(target, self.wt().join(name)).unwrap();
    }
}

fn t9(verb: &str, name: &str, target: &Path) -> String {
    format!(
        "Error: {verb} refused: {name} is a symbolic link to {}, outside this checkout, and this checkout pins its pack; give the worktree its own {name}/ or run armadai pack unpin\n",
        std::fs::canonicalize(target).unwrap().display()
    )
}

fn t10(name: &str, target: &Path) -> String {
    format!(
        "note: {name} is a symbolic link to {}, outside this checkout; link writes through it",
        std::fs::canonicalize(target).unwrap().display()
    )
}

#[test]
fn a_pinned_checkout_refuses_a_claude_link_out_and_leaves_the_other_checkout_alone() {
    let env = Env::new(true);
    env.own_project();
    let target = env.chantier().join(".claude");
    env.link_to(".claude", &target);
    let before = env.snapshot();
    let (code, _, err) = env.link();
    assert_eq!(code, Some(1), "{err}");
    assert_eq!(err, t9("link", ".claude", &target));
    assert_eq!(env.snapshot(), before);
}

#[test]
fn a_pinned_checkout_refuses_an_armadai_link_out() {
    let env = Env::new(true);
    let target = env.chantier().join(".armadai");
    env.link_to(".armadai", &target);
    let before = env.snapshot();
    let (code, _, err) = env.link();
    assert_eq!(code, Some(1), "{err}");
    assert_eq!(err, t9("link", ".armadai", &target));
    assert_eq!(env.snapshot(), before);
}

#[test]
fn without_a_pin_an_armadai_link_out_is_a_note_and_link_goes_on() {
    let env = Env::new(false);
    let target = env.chantier().join(".armadai");
    env.link_to(".armadai", &target);
    let (code, _, err) = env.link();
    assert_eq!(code, Some(0), "{err}");
    assert_eq!(
        err.lines()
            .filter(|l| l.starts_with("note: "))
            .collect::<Vec<_>>(),
        [t10(".armadai", &target)]
    );
    assert!(env.chantier().join(".armadai/link-manifest.yaml").exists());
}

#[test]
fn without_a_pin_a_claude_link_out_keeps_its_existing_refusal_and_no_note() {
    let env = Env::new(false);
    env.own_project();
    env.link_to(".claude", &env.chantier().join(".claude"));
    let before = env.snapshot();
    let (code, _, err) = env.link();
    assert_eq!(code, Some(1), "{err}");
    assert_eq!(err, "Error: .claude resolves outside the project\n");
    assert_eq!(env.snapshot(), before);
}

#[test]
fn a_link_that_stays_inside_the_checkout_is_neither_refused_nor_noted() {
    let env = Env::new(true);
    env.own_project();
    std::fs::create_dir_all(env.wt().join("shared/claude")).unwrap();
    env.link_to(".claude", Path::new("shared/claude"));
    let (code, _, err) = env.link();
    assert_eq!(code, Some(0), "{err}");
    assert!(!err.contains("symbolic link"), "{err}");
    assert!(!err.contains("link refused"), "{err}");
}

/// A chantier already linked, and a worktree whose `.claude` and `.armadai` lead to it.
fn linked_chantier(pinned: bool) -> Env {
    let env = Env::new(pinned);
    let (code, _, err) = env.run(&env.chantier(), "link");
    assert_eq!(code, Some(0), "{err}");
    assert!(env.chantier().join(".claude/agents/lead.md").exists());
    env.link_to(".claude", &env.chantier().join(".claude"));
    env.link_to(".armadai", &env.chantier().join(".armadai"));
    env
}

#[test]
fn a_pinned_checkout_refuses_unlink_through_a_claude_link_out() {
    let env = linked_chantier(true);
    std::fs::remove_file(env.wt().join(".armadai")).unwrap(); // only `.claude` leads out
    env.own_project();
    let before = env.snapshot();
    let (code, out, err) = env.run(&env.wt(), "unlink");
    assert_eq!(code, Some(1), "{out}{err}");
    assert_eq!(
        err,
        t9("unlink", ".claude", &env.chantier().join(".claude"))
    );
    assert_eq!(env.snapshot(), before);
}

#[test]
fn a_pinned_checkout_refuses_unlink_through_an_armadai_link_out() {
    let env = linked_chantier(true);
    std::fs::remove_file(env.wt().join(".claude")).unwrap(); // only `.armadai` leads out
    let before = env.snapshot();
    let (code, out, err) = env.run(&env.wt(), "unlink");
    assert_eq!(code, Some(1), "{out}{err}");
    assert_eq!(
        err,
        t9("unlink", ".armadai", &env.chantier().join(".armadai"))
    );
    assert_eq!(env.snapshot(), before);
}

#[test]
fn without_a_pin_unlink_through_the_links_is_unchanged() {
    let env = linked_chantier(false);
    let (code, out, err) = env.run(&env.wt(), "unlink");
    assert_eq!(code, Some(0), "{out}{err}");
    assert!(!err.contains("refused") && !err.contains("note:"), "{err}");
    assert!(out.contains("deleted"), "{out}");
    assert!(!env.chantier().join(".claude/agents/lead.md").exists());
}

/// A worktree whose `.git` leads nowhere, whose `.armadai` leads to a linked chantier and
/// whose project references a pack: it has no pin to look for, so every command refuses.
#[test]
fn a_project_with_a_pack_behind_a_dot_git_that_leads_nowhere_is_refused_and_left_alone() {
    let env = linked_chantier(false);
    std::fs::remove_file(env.wt().join(".claude")).unwrap();
    put(
        &env.chantier().join(".armadai/project.yaml"),
        "schema_version: 1\npack:\n  name: demo\n  source: https://example.com/packs.git\n",
    );
    std::fs::remove_dir_all(env.wt().join(".git")).unwrap();
    put(
        &env.wt().join(".git"),
        &format!("gitdir: {}\n", env.0.path().join("gone").display()),
    );
    let t14 = format!(
        "Error: {}/.git does not lead to a git directory, where this checkout keeps its pack pin; repair it (git worktree repair) and run the command again\n",
        std::fs::canonicalize(env.wt()).unwrap().display()
    );
    let before = env.snapshot();
    for cmd in ["unlink --dry-run", "unlink", "link"] {
        let (code, out, err) = env.run(&env.wt(), cmd);
        assert_eq!(code, Some(1), "{cmd}: {out}{err}");
        assert_eq!(err, t14, "{cmd}");
        assert_eq!(env.snapshot(), before, "{cmd}");
    }
}

/// The same with only `.claude` leading out: the second name of the loop.
#[test]
fn a_project_with_a_pack_and_a_claude_link_out_is_refused_behind_a_dot_git_that_leads_nowhere() {
    let env = linked_chantier(false);
    std::fs::remove_file(env.wt().join(".armadai")).unwrap(); // only `.claude` leads out
    env.own_project();
    put(
        &env.wt().join(".armadai/project.yaml"),
        "schema_version: 1\npack:\n  name: demo\n  source: https://example.com/packs.git\n",
    );
    std::fs::remove_dir_all(env.wt().join(".git")).unwrap();
    put(
        &env.wt().join(".git"),
        &format!("gitdir: {}\n", env.0.path().join("gone").display()),
    );
    let t14 = format!(
        "Error: {}/.git does not lead to a git directory, where this checkout keeps its pack pin; repair it (git worktree repair) and run the command again\n",
        std::fs::canonicalize(env.wt()).unwrap().display()
    );
    let before = env.snapshot();
    for cmd in ["unlink --dry-run", "unlink"] {
        let (code, out, err) = env.run(&env.wt(), cmd);
        assert_eq!(code, Some(1), "{cmd}: {out}{err}");
        assert_eq!(err, t14, "{cmd}");
        assert_eq!(env.snapshot(), before, "{cmd}");
    }
}

const STANDARDS_ONLY: &str = "schema_version: 1\nstandards:\n  - name: rust-std\n    source: https://example.com/packs.git\n    version: latest\n";

/// A project that references standards packs and no `pack:` still has a lock to keep: it
/// is concerned by the broken `.git` like a project with a pack (lot 14 rule).
fn standards_only_behind_a_broken_dot_git(name: &str, cmds: &[&str]) {
    let env = linked_chantier(false);
    if name == ".claude" {
        std::fs::remove_file(env.wt().join(".armadai")).unwrap();
        env.own_project();
        put(&env.wt().join(".armadai/project.yaml"), STANDARDS_ONLY);
    } else {
        std::fs::remove_file(env.wt().join(".claude")).unwrap();
        put(
            &env.chantier().join(".armadai/project.yaml"),
            STANDARDS_ONLY,
        );
    }
    std::fs::remove_dir_all(env.wt().join(".git")).unwrap();
    put(
        &env.wt().join(".git"),
        &format!("gitdir: {}\n", env.0.path().join("gone").display()),
    );
    let t14 = format!(
        "Error: {}/.git does not lead to a git directory, where this checkout keeps its pack pin; repair it (git worktree repair) and run the command again\n",
        std::fs::canonicalize(env.wt()).unwrap().display()
    );
    let before = env.snapshot();
    for cmd in cmds {
        let (code, out, err) = env.run(&env.wt(), cmd);
        assert_eq!(code, Some(1), "{cmd}: {out}{err}");
        assert_eq!(err, t14, "{cmd}");
        assert_eq!(env.snapshot(), before, "{cmd}");
    }
}

#[test]
fn a_standards_only_project_with_an_armadai_link_out_is_refused_behind_a_broken_dot_git() {
    standards_only_behind_a_broken_dot_git(".armadai", &["unlink --dry-run", "unlink", "link"]);
}

#[test]
fn a_standards_only_project_with_a_claude_link_out_is_refused_behind_a_broken_dot_git() {
    standards_only_behind_a_broken_dot_git(".claude", &["unlink --dry-run", "unlink"]);
}
