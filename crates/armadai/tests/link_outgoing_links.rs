//! `link` and a `.claude/` or `.armadai/` that is a symbolic link leaving the checkout:
//! refused under a pack pin (the other checkout is left untouched), a note otherwise.

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
        let o = Command::cargo_bin("armadai")
            .unwrap()
            .current_dir(self.wt())
            .env("HOME", self.0.path().join("home"))
            .env("ARMADAI_CONFIG_DIR", self.0.path().join("config"))
            .arg("link")
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
    }

    fn link_to(&self, name: &str, target: &Path) {
        std::os::unix::fs::symlink(target, self.wt().join(name)).unwrap();
    }
}

fn t9(name: &str, target: &Path) -> String {
    format!(
        "Error: link refused: {name} is a symbolic link to {}, outside this checkout, and this checkout pins its pack; give the worktree its own {name}/ or run armadai pack unpin\n",
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
    assert_eq!(err, t9(".claude", &target));
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
    assert_eq!(err, t9(".armadai", &target));
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
