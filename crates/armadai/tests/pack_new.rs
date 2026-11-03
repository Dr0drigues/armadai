//! `armadai pack new`: the minimal pack, the capture of a project, the refusals.

use std::path::{Path, PathBuf};

use assert_cmd::Command;

struct Env {
    dir: tempfile::TempDir,
}

struct Run {
    code: Option<i32>,
    out: String,
    err: String,
}

impl Env {
    fn new() -> Env {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("project/.git")).unwrap();
        std::fs::create_dir_all(dir.path().join("config")).unwrap();
        Env { dir }
    }

    fn root(&self) -> PathBuf {
        // canonical: the binary prints its cwd as the system reports it (/private/var on macOS)
        self.dir.path().canonicalize().unwrap().join("project")
    }

    fn put(&self, rel: &str, text: &str) {
        let p = self.root().join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    fn run(&self, args: &[&str]) -> Run {
        let out = Command::cargo_bin("armadai")
            .unwrap()
            .current_dir(self.root())
            .env("ARMADAI_CONFIG_DIR", self.dir.path().join("config"))
            .env("HOME", self.dir.path())
            .env("NO_COLOR", "1")
            .args(args)
            .output()
            .unwrap();
        Run {
            code: out.status.code(),
            out: String::from_utf8_lossy(&out.stdout).into_owned(),
            err: String::from_utf8_lossy(&out.stderr).into_owned(),
        }
    }
}

/// Every file under `dir`, relative, sorted; folders listed with a trailing `/`.
fn tree(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut todo = vec![dir.to_path_buf()];
    while let Some(d) = todo.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            let rel = p.strip_prefix(dir).unwrap().display().to_string();
            if p.is_dir() {
                out.push(format!("{rel}/"));
                todo.push(p);
            } else {
                out.push(rel);
            }
        }
    }
    out.sort();
    out
}

const AGENT: &str = "---\ndescription: Reviews code\n---\nYou review.\n";

#[test]
fn pack_new_empty_validates() {
    let env = Env::new();
    let r = env.run(&[
        "pack",
        "new",
        "demo",
        "--empty",
        "--description",
        "Demo pack",
    ]);
    assert_eq!(r.code, Some(0), "{}{}", r.out, r.err);
    let dir = env.root().join("demo");
    assert!(r.out.lines().any(|l| l
        == format!(
            "Created pack demo in {}: pack.yaml, project.yaml, agents/, skills/, flows/, automations/",
            dir.display()
        )));
    assert_eq!(
        std::fs::read_to_string(dir.join("pack.yaml")).unwrap(),
        "format: 1\nname: demo\ndescription: \"Demo pack\"\n"
    );
    let pack = armadai_core::starter::StarterPack::load(&dir).unwrap();
    assert_eq!(pack.format, Some(1));
    let v = env.run(&["validate", dir.to_str().unwrap()]);
    assert_eq!(v.code, Some(0), "{}{}", v.out, v.err);
    assert_eq!(v.out.trim(), "0 error(s), 0 warning(s)");
    for d in ["agents", "skills", "flows", "automations"] {
        assert!(dir.join(d).join(".gitkeep").is_file(), "{d}");
    }
    // installing it: no ignored-pack-file, no .gitkeep copied into the project
    let i = env.run(&["init", "--pack", dir.to_str().unwrap()]);
    assert_eq!(i.code, Some(0), "{}{}", i.out, i.err);
    assert!(!i.out.contains("ignored-pack-file") && !i.err.contains("ignored-pack-file"));
    assert!(
        tree(&env.root().join(".armadai"))
            .iter()
            .all(|f| !f.ends_with(".gitkeep"))
    );
}

#[test]
fn pack_new_empty_default_description() {
    let env = Env::new();
    assert_eq!(env.run(&["pack", "new", "demo", "--empty"]).code, Some(0));
    let text = std::fs::read_to_string(env.root().join("demo/pack.yaml")).unwrap();
    assert_eq!(
        text,
        "format: 1\nname: demo\ndescription: \"A starter pack\"\n"
    );
}

#[test]
fn pack_new_from_project_copies_the_white_list_only() {
    let env = Env::new();
    env.put(
        ".armadai/project.yaml",
        "schema_version: 1\ntargets: [claude]\n",
    );
    env.put(".armadai/agents/rev.md", AGENT);
    env.put(
        ".armadai/skills/tool/SKILL.md",
        "---\nname: tool\ndescription: A tool\n---\nUse it.\n",
    );
    env.put(
        ".armadai/skills/tool/scripts/run.sh",
        "#!/bin/sh\necho hi\n",
    );
    env.put(".armadai/link-manifest.yaml", "version: 1\n");
    env.put(".armadai/state/x", "state");
    env.put(".armadai/vars.local.yaml", "who: secret-local-value\n");
    env.put(".armadai/vars.yaml", "who: shared\n");
    env.put(".armadai/pack.lock", "format: 1\n");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let p = env.root().join(".armadai/skills/tool/scripts/run.sh");
        std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let r = env.run(&[
        "pack",
        "new",
        "cap",
        "--from-project",
        "--description",
        "Captured",
    ]);
    assert_eq!(r.code, Some(0), "{}{}", r.out, r.err);
    let dir = env.root().join("cap");
    assert!(r.out.lines().any(|l| l
        == format!(
            "Created pack cap in {} from .armadai/: 1 agent(s), 1 skill(s), 0 flow(s), \
             0 automation(s), 0 template(s), 0 command(s) \
             (link-manifest.yaml, state/, pack.lock and the vars files left out)",
            dir.display()
        )));
    assert_eq!(
        tree(&dir),
        [
            "agents/",
            "agents/rev.md",
            "pack.yaml",
            "project.yaml",
            "skills/",
            "skills/tool/",
            "skills/tool/SKILL.md",
            "skills/tool/scripts/",
            "skills/tool/scripts/run.sh",
        ]
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let m = std::fs::metadata(dir.join("skills/tool/scripts/run.sh")).unwrap();
        assert_ne!(
            m.permissions().mode() & 0o111,
            0,
            "the script stays executable"
        );
    }
    for f in tree(&dir) {
        if dir.join(&f).is_file() {
            let text = std::fs::read_to_string(dir.join(&f)).unwrap();
            assert!(!text.contains("secret-local-value"), "{f}");
        }
    }
    let v = env.run(&["validate", dir.to_str().unwrap()]);
    assert_eq!(v.code, Some(0), "{}{}", v.out, v.err);
}

#[test]
fn pack_new_refuses_an_existing_folder() {
    let env = Env::new();
    env.put("demo/keep.txt", "mine");
    let r = env.run(&["pack", "new", "demo", "--empty"]);
    assert_eq!(r.code, Some(1));
    let want = format!(
        "Error: {} already exists: choose another name, or --dir",
        env.root().join("demo").display()
    );
    assert!(r.err.lines().any(|l| l == want), "{}", r.err);
    assert_eq!(tree(&env.root().join("demo")), ["keep.txt"]);
}

#[test]
fn pack_new_from_a_reference_project_refuses() {
    let env = Env::new();
    env.put(
        ".armadai/project.yaml",
        "schema_version: 1\npack: { name: demo, source: 'https://example.com/p.git', version: latest }\n",
    );
    let r = env.run(&["pack", "new", "cap", "--from-project"]);
    assert_eq!(r.code, Some(1));
    assert!(
        r.err.lines().any(|l| l
            == "Error: this project references pack demo: its .armadai/ is an overlay, not a pack; \
                create the pack from its source instead"),
        "{}",
        r.err
    );
    assert!(!env.root().join("cap").exists());
}

#[test]
fn pack_new_without_a_flag_needs_a_terminal() {
    let env = Env::new();
    let r = env.run(&["pack", "new", "demo"]);
    assert_eq!(r.code, Some(1));
    assert!(
        r.err.lines().any(|l| l
            == "Error: interactive mode needs a terminal; pass --empty or --from-project instead (armadai pack new --help)"),
        "{}",
        r.err
    );
    assert!(!env.root().join("demo").exists());
}

#[test]
fn pack_new_dir_is_the_parent_folder() {
    let env = Env::new();
    let r = env.run(&["pack", "new", "demo", "--empty", "--dir", "out/packs"]);
    assert_eq!(r.code, Some(0), "{}{}", r.out, r.err);
    assert!(env.root().join("out/packs/demo/pack.yaml").is_file());
}

#[test]
fn an_invalid_pack_writes_nothing_and_blames_the_project_file() {
    let env = Env::new();
    env.put(
        ".armadai/project.yaml",
        "schema_version: 1\ntargets: [claude]\n",
    );
    env.put(
        ".armadai/agents/bad.md",
        "---\ndescription: [unclosed\n---\nBody.\n",
    );
    let r = env.run(&["pack", "new", "cap", "--from-project", "--dir", "out/x"]);
    assert_eq!(r.code, Some(1), "{}{}", r.out, r.err);
    assert!(
        r.err
            .lines()
            .any(|l| l == "Error: pack 'cap' is invalid; nothing was written"),
        "{}",
        r.err
    );
    let source = env.root().join(".armadai/agents/bad.md");
    let blame = format!("{}:", source.display());
    assert!(r.err.lines().any(|l| l.starts_with(&blame)), "{}", r.err);
    assert!(!env.root().join("out").exists(), "no folder left behind");
}

#[cfg(unix)]
#[test]
fn a_link_in_the_project_is_refused() {
    let env = Env::new();
    env.put(".armadai/project.yaml", "schema_version: 1\n");
    env.put("elsewhere.md", AGENT);
    std::fs::create_dir_all(env.root().join(".armadai/agents")).unwrap();
    std::os::unix::fs::symlink(
        env.root().join("elsewhere.md"),
        env.root().join(".armadai/agents/a.md"),
    )
    .unwrap();
    let r = env.run(&["pack", "new", "cap", "--from-project"]);
    assert_eq!(r.code, Some(1), "{}{}", r.out, r.err);
    let want = format!(
        "Error: {} is a symbolic link, refused",
        env.root().join(".armadai/agents/a.md").display()
    );
    assert!(r.err.lines().any(|l| l == want), "{}", r.err);
    assert!(!env.root().join("cap").exists());
}
