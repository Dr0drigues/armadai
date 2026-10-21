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
        self.run_env(args, &[])
    }

    fn run_env(&self, args: &[&str], vars: &[(&str, &str)]) -> (bool, String) {
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
            .envs(vars.iter().copied())
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

fn agent_files(dir: &Path) -> Vec<String> {
    let mut got: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    got.sort();
    got
}

/// Each kind of project source, alone, carrying an embedded pack's name: the embedded
/// pack is installed. A relative starter folder, which would be read from the current
/// directory, is ignored with a warning.
#[test]
fn an_embedded_pack_wins_over_every_project_source() {
    let want = agent_files(
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../armadai-core/starters/code-analysis-rust/agents"),
    );
    let name = "code-analysis-rust";
    let ignored = |from: &str| {
        format!(
            "warning: starter folder 'starters' ({from}) is a relative path: ignored, give an absolute path\n"
        )
    };
    for kind in [
        "registry",
        ".armadai/starters",
        "starters",
        ".",
        "starters_dirs",
        "ARMADAI_STARTERS_DIRS",
    ] {
        let env = Env::new();
        let mut vars = vec![];
        let mut want_err = String::new();
        match kind {
            "registry" => {
                env.project_sources(&[A]);
                write_pack(&env.cache(A).join(name), name, "FROM-PROJECT");
            }
            "starters_dirs" | "ARMADAI_STARTERS_DIRS" => {
                write_pack(
                    &env.path("repo/sub/starters").join(name),
                    name,
                    "FROM-PROJECT",
                );
                if kind == "starters_dirs" {
                    std::fs::write(
                        env.path("config/config.yaml"),
                        "starters_dirs:\n  - starters\n",
                    )
                    .unwrap();
                } else {
                    vars.push(("ARMADAI_STARTERS_DIRS", "starters"));
                }
                want_err = ignored(kind);
            }
            dir => write_pack(
                &env.path("repo/sub").join(dir).join(name),
                name,
                "FROM-PROJECT",
            ),
        }
        let (ok, err) = env.run_env(&["init", "--pack", name], &vars);
        assert!(ok, "{kind}: {err}");
        assert_eq!(
            agent_files(&env.path("repo/sub/.armadai/agents")),
            want,
            "{kind}: {err}"
        );
        assert_eq!(err, want_err, "{kind}");
    }
}

/// A path names a local folder, a bare name never does.
#[test]
fn only_a_path_names_a_folder_of_the_current_directory() {
    let env = Env::new();
    let name = "code-analysis-rust";
    write_pack(&env.path("repo/sub").join(name), name, "FROM-LOCAL");
    let (ok, err) = env.run(&["init", "--pack", "./code-analysis-rust"]);
    assert!(ok, "{err}");
    assert_eq!(env.installed(), agent("FROM-LOCAL"));
    assert_eq!(err, "");
}

/// Two project packs, and one of the user's sources declared but not synced yet: the
/// sync runs, and the user's pack wins. Without it, the ambiguity stays an error.
#[test]
fn a_project_ambiguity_still_lets_an_unsynced_user_source_win() {
    for with_user in [true, false] {
        let env = Env::new();
        env.project_sources(&[B]);
        write_pack(&env.cache(B).join("shared"), "shared", "FROM-PROJREG");
        env.archive("b.example", "shared", "FROM-PROJREG");
        write_pack(
            &env.path("repo/sub/.armadai/starters/shared"),
            "shared",
            "FROM-LOCAL",
        );
        if with_user {
            env.user_sources(&[A]);
            env.archive("a.example", "shared", "FROM-USER");
        }
        let (ok, err) = env.run(&["init", "--pack", "shared"]);
        if with_user {
            assert!(ok, "{err}");
            assert_eq!(env.installed(), agent("FROM-USER"), "{err}");
        } else {
            assert!(!ok);
            let first = err.lines().find(|l| l.starts_with("Error:"));
            let want = "Error: starter pack 'shared' is provided by several project sources:";
            assert_eq!(first, Some(want), "{err}");
            assert!(!env.path("repo/sub/.armadai/agents").exists());
        }
    }
}

/// A URL read from a project never drives the terminal: its control characters are
/// escaped in the note.
#[test]
fn the_note_escapes_control_characters() {
    for esc_root in [false, true] {
        note_escapes_control_characters(esc_root);
    }
}

/// `esc_root`: the project root's real name holds an escape character (`repo` is a
/// link to it, the current directory resolves to it).
fn note_escapes_control_characters(esc_root: bool) {
    let env = Env::new();
    if esc_root {
        std::fs::rename(env.path("repo"), env.path("re\u{1b}po")).unwrap();
        std::os::unix::fs::symlink(env.path("re\u{1b}po"), env.path("repo")).unwrap();
    }
    let url = "https://p.example/x\u{1b}[31mRED\u{7}/s.tar.gz";
    std::fs::create_dir_all(env.path("repo/.armadai")).unwrap();
    let yaml =
        "registries:\n  starters:\n    - url: \"https://p.example/x\\e[31mRED\\a/s.tar.gz\"\n";
    std::fs::write(env.path("repo/.armadai/config.yaml"), yaml).unwrap();
    env.archive("p.example", "shared", "FROM-PROJECT");
    write_pack(&env.cache(url).join("shared"), "shared", "FROM-PROJECT");
    let (ok, err) = env.run(&["init", "--pack", "shared"]);
    assert!(ok, "{err}");
    let note = format!(
        "note: pack 'shared' comes from a project source, not from your own folders or \
         sources: https://p.example/x\\u{{1b}}[31mRED\\u{{7}}/s.tar.gz (from {})",
        (env.path("repo")
            .canonicalize()
            .unwrap()
            .display()
            .to_string())
        .replace('\u{1b}', "\\u{1b}")
    );
    let notes: Vec<&str> = err.lines().filter(|l| l.starts_with("note:")).collect();
    assert_eq!(notes, [note.as_str()], "{err}");
}

/// A pack name matches a folder name case included, whatever the file system: `Shared`
/// is not the user's `shared`, nor the project's.
#[test]
fn a_pack_name_matches_folders_case_included() {
    let env = Env::new();
    env.user_sources(&[A]);
    write_pack(&env.cache(A).join("shared"), "shared", "FROM-USER");
    env.archive("a.example", "shared", "FROM-USER");
    write_pack(
        &env.path("repo/sub/.armadai/starters/shared"),
        "shared",
        "FROM-PROJECT",
    );
    let (ok, err) = env.run(&["init", "--pack", "Shared"]);
    assert!(!ok);
    let first = err.lines().find(|l| l.starts_with("Error:"));
    assert_eq!(
        first.map(|l| l.split('.').next().unwrap()),
        Some("Error: Starter pack 'Shared' not found"),
        "{err}"
    );
    assert!(!env.path("repo/sub/.armadai/agents").exists());
}

#[test]
fn a_relative_starter_folder_is_not_added() {
    let env = Env::new();
    let (ok, err) = env.run(&["config", "starters-dir", "add", "starters"]);
    assert!(!ok);
    assert_eq!(
        err,
        "Error: starters is a relative path: give an absolute one\n"
    );
    assert!(!env.path("config/config.yaml").exists());
}

/// A user source that fails to sync may hold the pack: a project pack is refused then.
/// One that syncs without it lets the project pack in, announced.
#[test]
fn a_user_source_that_fails_to_sync_refuses_a_project_pack() {
    for user_syncs in [false, true] {
        let env = Env::new();
        env.user_sources(&[A]);
        if user_syncs {
            env.archive("a.example", "other", "FROM-USER");
        } // else: no archive, the fake curl fails
        env.project_sources(&[B]);
        write_pack(&env.cache(B).join("shared"), "shared", "FROM-PROJECT");
        env.archive("b.example", "shared", "FROM-PROJECT");
        let (ok, err) = env.run(&["init", "--pack", "shared"]);
        if user_syncs {
            assert!(ok, "{err}");
            assert_eq!(env.installed(), agent("FROM-PROJECT"));
            continue;
        }
        assert!(!ok);
        let error: Vec<&str> = err
            .lines()
            .skip_while(|l| !l.starts_with("Error:"))
            .collect();
        let folder = env.cache(B).join("shared");
        let want = format!(
            "Error: pack 'shared' is found only in project sources, and these sources of \
             yours did not sync (one of them may hold it):\n  {A}\nretry once they can be \
             reached, or install the project's pack by its folder: `armadai init --pack {}`",
            folder.display()
        );
        assert_eq!(error.join("\n"), want, "{err}");
        assert!(!env.path("repo/sub/.armadai/agents").exists());
    }
}

/// `~` and `~/...` are the home directory; without HOME, a clear error.
#[test]
fn a_leading_tilde_is_the_home_directory() {
    let env = Env::new();
    write_pack(&env.path("home-p"), "home-p", "FROM-HOME");
    let (ok, err) = env.run(&["init", "--pack", "~/home-p"]);
    assert!(ok, "{err}");
    assert_eq!(env.installed(), agent("FROM-HOME"));
    let env = Env::new();
    let (ok, err) = env.run_env(&["init", "--pack", "~/home-p"], &[("HOME", "")]);
    assert!(!ok);
    assert_eq!(
        err,
        "Error: cannot expand `~` in '~/home-p': HOME is not set\n"
    );
}

#[test]
fn an_empty_pack_name_is_refused() {
    let env = Env::new();
    write_pack(&env.path("repo/sub"), "here", "FROM-CWD");
    let (ok, err) = env.run(&["init", "--pack", ""]);
    assert!(!ok);
    assert_eq!(
        err,
        "Error: empty pack name: give a pack name or a pack folder\n"
    );
}

/// `.`, `../x` and `x/` are paths: each names its folder.
#[test]
fn dot_parent_and_trailing_slash_are_paths() {
    for (arg, at) in [(".", "repo/sub"), ("../x", "repo/x"), ("p/", "repo/sub/p")] {
        let env = Env::new();
        write_pack(&env.path(at), "local", "FROM-LOCAL");
        let (ok, err) = env.run(&["init", "--pack", arg]);
        assert!(ok, "{arg}: {err}");
        assert_eq!(env.installed(), agent("FROM-LOCAL"), "{arg}");
    }
}

/// The "Available packs" list shows a folder name's control characters escaped.
#[test]
fn the_available_packs_list_escapes_control_characters() {
    let env = Env::new();
    write_pack(&env.path("repo/sub/.armadai/starters/x\u{1b}y"), "x", "X");
    let mut names: Vec<String> =
        std::fs::read_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../armadai-core/starters"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
    names.push("x\u{1b}y".into());
    names.sort();
    let (ok, err) = env.run(&["init", "--pack", "nothere"]);
    assert!(!ok);
    let want = format!(
        "Error: Starter pack 'nothere' not found. Available packs: {}\n",
        names.join(", ").replace('\u{1b}', "\\u{1b}")
    );
    assert_eq!(err, want);
}
