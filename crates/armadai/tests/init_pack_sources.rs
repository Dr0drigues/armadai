//! `armadai init --pack <name>` across starter sources (#490): a trusted source (the
//! embedded packs, the user's folders and `registries.yaml` sources) always wins over a
//! project source (a legacy project config's `registries.starters`, its
//! `.armadai/starters/`, a `starters/` folder of the current directory); two sources of
//! one level holding the same pack are refused; a project pack is announced.
#![cfg(unix)]

use std::path::{Path, PathBuf};

use armadai_core::registries::cache_key;
use assert_cmd::Command;

struct Env {
    dir: tempfile::TempDir,
}

impl Env {
    /// A project `repo/` (legacy config holding project sources goes at its root), run
    /// from `repo/sub`, with a fake `curl` serving `arch/<host>.tar.gz`.
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        for d in ["config", "repo/.git", "repo/sub", "fake", "arch"] {
            std::fs::create_dir_all(dir.path().join(d)).unwrap();
        }
        let curl = dir.path().join("fake/curl");
        std::fs::write(
            &curl,
            "#!/bin/sh\nwhile [ $# -gt 1 ]; do [ \"$1\" = -o ] && out=$2; shift; done\n\
             host=$(printf '%s' \"$1\" | sed -E 's#https://([^@/]*@)?([^/]*)/.*#\\2#')\n\
             cp \"$ARCH/$host.tar.gz\" \"$out\"\n",
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&curl, std::fs::Permissions::from_mode(0o755)).unwrap();
        Env { dir }
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.dir.path().join(rel)
    }

    fn cache(&self, url: &str) -> PathBuf {
        self.path("config/registry/starters").join(cache_key(url))
    }

    fn user_sources(&self, urls: &[&str]) {
        let list: String = urls.iter().map(|u| format!("  - url: {u}\n")).collect();
        std::fs::write(
            self.path("config/registries.yaml"),
            format!("starters:\n{list}"),
        )
        .unwrap();
    }

    fn project_sources(&self, urls: &[&str]) {
        let list: String = urls.iter().map(|u| format!("    - url: {u}\n")).collect();
        let yaml = format!("registries:\n  starters:\n{list}");
        // `.armadai/config.yaml`: the other legacy name, `armadai.yaml`, prints a hint
        std::fs::create_dir_all(self.path("repo/.armadai")).unwrap();
        std::fs::write(self.path("repo/.armadai/config.yaml"), yaml).unwrap();
    }

    /// The archive a source URL on `host` serves, holding `name` marked `marker`.
    fn archive(&self, host: &str, name: &str, marker: &str) {
        let src = self.path(&format!("src-{host}"));
        write_pack(&src.join(name), name, marker);
        let out = self.path(&format!("arch/{host}.tar.gz"));
        let ok = std::process::Command::new("tar")
            .arg("-czf")
            .arg(&out)
            .arg("-C")
            .arg(&src)
            .arg(".")
            .status()
            .unwrap();
        assert!(ok.success());
    }

    fn run(&self, args: &[&str]) -> (bool, String) {
        let out = Command::cargo_bin("armadai")
            .unwrap()
            .current_dir(self.path("repo/sub"))
            .env_clear()
            .env(
                "PATH",
                format!("{}:/usr/bin:/bin", self.path("fake").display()),
            )
            .env("ARMADAI_CONFIG_DIR", self.path("config"))
            .env("HOME", self.dir.path())
            .env("ARCH", self.path("arch"))
            .env("NO_COLOR", "1")
            .args(args)
            .output()
            .unwrap();
        let err = String::from_utf8_lossy(&out.stderr).into_owned();
        (out.status.success(), err)
    }

    fn installed(&self) -> String {
        std::fs::read_to_string(self.path("repo/sub/.armadai/agents/who.md")).unwrap()
    }
}

fn write_pack(dir: &Path, name: &str, marker: &str) {
    std::fs::create_dir_all(dir.join("agents")).unwrap();
    let pack = format!("format: 1\nname: {name}\ndescription: {marker}\n");
    std::fs::write(dir.join("pack.yaml"), pack).unwrap();
    std::fs::write(dir.join("project.yaml"), "schema_version: 1\n").unwrap();
    std::fs::write(dir.join("agents/who.md"), agent(marker)).unwrap();
}

fn agent(marker: &str) -> String {
    format!("---\ndescription: {marker}\n---\n{marker}\n")
}

const A: &str = "https://a.example/s.tar.gz";
const B: &str = "https://b.example/s.tar.gz";

/// Whatever the URLs, hence the names of the cache folders and the order `read_dir`
/// lists them in, and whatever the order they were created in, the user's pack wins.
#[test]
fn a_user_source_wins_over_a_project_source_whatever_the_cache_order() {
    for (user, project) in [(A, B), (B, A)] {
        for user_first in [true, false] {
            let env = Env::new();
            env.user_sources(&[user]);
            env.project_sources(&[project]);
            let mut order = [(user, "FROM-USER"), (project, "FROM-PROJECT")];
            if !user_first {
                order.reverse();
            }
            for (url, marker) in order {
                // built aside then renamed in: each folder appears at once, in this order
                let aside = env.path(&format!("aside-{marker}"));
                write_pack(&aside.join("shared"), "shared", marker);
                std::fs::create_dir_all(env.cache(url).parent().unwrap()).unwrap();
                std::fs::rename(&aside, env.cache(url)).unwrap();
            }
            let (ok, err) = env.run(&["init", "--pack", "shared"]);
            let case = format!("user {user}, user first: {user_first}\n{err}");
            assert!(ok, "{case}");
            assert_eq!(env.installed(), agent("FROM-USER"), "{case}");
            // found in a trusted source: no sync, no note
            assert_eq!(err, "", "{case}");
        }
    }
}

/// A user source declared but not synced yet is synced before a project pack is
/// taken, and wins.
#[test]
fn a_user_source_not_synced_yet_still_wins_over_a_cached_project_pack() {
    let env = Env::new();
    env.user_sources(&[A]);
    env.project_sources(&[B]);
    write_pack(&env.cache(B).join("shared"), "shared", "FROM-PROJECT");
    env.archive("a.example", "shared", "FROM-USER");
    env.archive("b.example", "shared", "FROM-PROJECT");
    let (ok, err) = env.run(&["init", "--pack", "shared"]);
    assert!(ok, "{err}");
    assert_eq!(env.installed(), agent("FROM-USER"), "{err}");
}

#[test]
fn two_user_sources_holding_the_same_pack_are_refused() {
    let env = Env::new();
    env.user_sources(&[A, B]);
    write_pack(&env.cache(A).join("shared"), "shared", "FROM-A");
    write_pack(&env.cache(B).join("shared"), "shared", "FROM-B");
    let (ok, err) = env.run(&["init", "--pack", "shared"]);
    assert!(!ok);
    let expected = format!(
        "Error: starter pack 'shared' is provided by several of your sources:\n  \
         {A}: {}\n  {B}: {}\ninstall one by its folder (`armadai init --pack <folder>`), \
         or remove the other sources\n",
        env.cache(A).join("shared").display(),
        env.cache(B).join("shared").display()
    );
    assert_eq!(err, expected);
    assert!(!env.path("repo/sub/.armadai").exists());
}

#[test]
fn a_pack_only_in_a_project_source_is_installed_and_announced() {
    let env = Env::new();
    let url = "https://tok:secret@p.example/s.tar.gz?sig=0123456789";
    env.project_sources(&[url]);
    env.archive("p.example", "only", "FROM-PROJECT");
    write_pack(&env.cache(url).join("only"), "only", "FROM-PROJECT");
    let (ok, err) = env.run(&["init", "--pack", "only"]);
    assert!(ok, "{err}");
    assert_eq!(env.installed(), agent("FROM-PROJECT"));
    let note = format!(
        "note: pack 'only' comes from a project source, not from your own folders or \
         sources: https://***@p.example/s.tar.gz?*** (from {})",
        // the current directory, hence the project root, is the resolved path
        env.path("repo").canonicalize().unwrap().display()
    );
    let notes: Vec<&str> = err.lines().filter(|l| l.starts_with("note:")).collect();
    assert_eq!(notes, [note.as_str()], "{err}");
    assert!(
        !err.contains("secret") && !err.contains("0123456789"),
        "{err}"
    );
}

/// Each kind of project source, alone, carrying an embedded pack's name: the embedded
/// pack is installed.
#[test]
fn an_embedded_pack_wins_over_every_project_source() {
    let embedded = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../armadai-core/starters/code-analysis-rust/agents");
    let mut want: Vec<String> = std::fs::read_dir(embedded)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    want.sort();
    let name = "code-analysis-rust";
    for kind in ["registry", ".armadai/starters", "starters"] {
        let env = Env::new();
        match kind {
            "registry" => {
                env.project_sources(&[A]);
                write_pack(&env.cache(A).join(name), name, "FROM-PROJECT");
            }
            dir => write_pack(
                &env.path("repo/sub").join(dir).join(name),
                name,
                "FROM-PROJECT",
            ),
        }
        let (ok, err) = env.run(&["init", "--pack", name]);
        assert!(ok, "{kind}: {err}");
        let mut got: Vec<String> = std::fs::read_dir(env.path("repo/sub/.armadai/agents"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        got.sort();
        assert_eq!(got, want, "{kind}: {err}");
    }
}
