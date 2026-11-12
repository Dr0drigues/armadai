//! `armadai pack test` and the validation of `tests:`, end to end.

use std::path::Path;

use assert_cmd::Command;

struct Env {
    dir: tempfile::TempDir,
}

impl Env {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("config")).unwrap();
        std::fs::create_dir_all(dir.path().join("home")).unwrap();
        Env { dir }
    }

    fn put(&self, rel: &str, text: &str) {
        let p = self.dir.path().join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    /// `(exit code, stdout, stderr)`, run from the scratch folder.
    fn run(&self, args: &[&str]) -> (Option<i32>, String, String) {
        let out = Command::cargo_bin("armadai")
            .unwrap()
            .current_dir(self.dir.path())
            .env("ARMADAI_CONFIG_DIR", self.dir.path().join("config"))
            .env("HOME", self.dir.path().join("home"))
            .env("NO_COLOR", "1")
            .args(args)
            .timeout(std::time::Duration::from_secs(60))
            .output()
            .unwrap();
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }
}

/// The durations (`0.4s`) of a result replaced, they vary from run to run.
fn untimed(text: &str) -> String {
    let timed = |w: &str| {
        w.strip_suffix('s')
            .and_then(|n| n.split_once('.'))
            .is_some_and(|(a, b)| {
                !a.is_empty()
                    && a.bytes().all(|c| c.is_ascii_digit())
                    && b.len() == 1
                    && b.bytes().all(|c| c.is_ascii_digit())
            })
    };
    text.split_inclusive('\n')
        .map(|line| {
            let body = line.trim_end_matches('\n');
            let words: Vec<&str> = body
                .split(' ')
                .map(|w| if timed(w) { "X.Xs" } else { w })
                .collect();
            words.join(" ") + &line[body.len()..]
        })
        .collect()
}

fn is_dir(p: &Path) -> bool {
    p.is_dir()
}

#[test]
fn two_tests_one_passing_one_failing_give_the_whole_text_and_exit_1() {
    let env = Env::new();
    env.put(
        "p/pack.yaml",
        "name: p\ndescription: d\ntests:\n  - tests/ok.sh\n  - tests/bad.sh\n",
    );
    env.put("p/tests/ok.sh", "exit 0\n");
    env.put("p/tests/bad.sh", "echo boom; exit 1\n");
    let (code, out, err) = env.run(&["pack", "test", "p"]);
    assert_eq!(
        (code, untimed(&out).as_str(), err.as_str()),
        (
            Some(1),
            "pack test  p\n\
             ok    tests/ok.sh  X.Xs\n\
             FAIL  tests/bad.sh  exit 1  X.Xs\n\
             \x20 | boom\n\
             1 passed, 1 failed\n",
            ""
        )
    );
}

#[test]
fn a_passing_pack_exits_0_and_a_pack_without_test_says_so() {
    let env = Env::new();
    env.put("p/pack.yaml", "name: p\ndescription: d\ntests:\n  - t.sh\n");
    env.put("p/t.sh", "exit 0\n");
    let (code, out, _) = env.run(&["pack", "test", "p"]);
    assert_eq!(
        (code, untimed(&out).as_str()),
        (
            Some(0),
            "pack test  p\nok    t.sh  X.Xs\n1 passed, 0 failed\n"
        )
    );
    env.put("q/pack.yaml", "name: q\ndescription: d\n");
    let (code, out, err) = env.run(&["pack", "test", "q"]);
    assert_eq!(
        (code, out.as_str(), err.as_str()),
        (
            Some(0),
            "pack q declares no test (tests: in pack.yaml)\n",
            ""
        )
    );
}

#[test]
fn a_test_writing_under_home_leaves_the_callers_home_alone() {
    let env = Env::new();
    env.put("p/pack.yaml", "name: p\ndescription: d\ntests:\n  - t.sh\n");
    let home = env.dir.path().join("home");
    env.put(
        "p/t.sh",
        &format!(
            "test \"$HOME\" != '{h}' || exit 3\necho x > \"$HOME/x\"\n",
            h = home.display()
        ),
    );
    let (code, out, _) = env.run(&["pack", "test", "p"]);
    assert_eq!(code, Some(0), "{out}");
    assert!(is_dir(&home));
    assert!(std::fs::read_dir(&home).unwrap().next().is_none());
    assert!(
        std::fs::read_dir(env.dir.path().join("config"))
            .unwrap()
            .next()
            .is_none()
    );
}

#[test]
fn validate_refuses_an_escaping_test_and_does_not_flag_a_listed_one() {
    let env = Env::new();
    env.put(
        "p/pack.yaml",
        "name: p\ndescription: d\nformat: 1\ntests:\n  - tests/ok.sh\n  - ../x\n",
    );
    env.put("p/tests/ok.sh", "exit 0\n");
    env.put("p/tests/other.sh", "exit 0\n");
    let (code, out, err) = env.run(&["validate", "p"]);
    let all = format!("{out}{err}");
    assert_eq!(code, Some(1), "{all}");
    assert!(
        all.contains("test `../x` must be a relative path inside the pack"),
        "{all}"
    );
    assert!(all.contains("pack-test-path-escapes"), "{all}");
    assert!(
        all.contains("write it as tests/<name>.sh, without .. or a leading /"),
        "{all}"
    );
    // only the file `tests:` does not list is "ignored"
    let ignored: Vec<&str> = all
        .lines()
        .filter(|l| l.contains("ignored-pack-file"))
        .collect();
    assert_eq!(ignored.len(), 1, "{all}");
    assert!(ignored[0].contains("other.sh"), "{all}");
}

#[test]
fn pack_test_refuses_a_folder_that_is_no_pack_and_an_escaping_path() {
    let env = Env::new();
    std::fs::create_dir_all(env.dir.path().join("empty")).unwrap();
    let (code, _, err) = env.run(&["pack", "test", "empty"]);
    assert_eq!(code, Some(1));
    assert!(err.contains("No pack.yaml found in"), "{err}");
    env.put("p/pack.yaml", "name: p\ndescription: d\ntests:\n  - ../x\n");
    let (code, _, err) = env.run(&["pack", "test", "p"]);
    assert_eq!(code, Some(1));
    assert!(
        err.contains("test `../x` must be a relative path inside the pack"),
        "{err}"
    );
}
