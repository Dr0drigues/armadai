//! Calls to the Herdr.app binaries (`hc`, `herdr-app`). Third-party tool rule: the binary
//! comes from `PATH`, no shell, one argument each, and a failure is ignored by the caller.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

pub(crate) const HC: &str = "hc";
// Used by `herdr watch` (T4).
#[allow(dead_code)]
pub(crate) const HERDR_APP: &str = "herdr-app";

/// `HC_TERMINAL_ID`, when set and not empty: we run in a Herdr.app terminal.
pub(crate) fn terminal_id() -> Option<String> {
    std::env::var("HC_TERMINAL_ID")
        .ok()
        .filter(|s| !s.is_empty())
}

/// The first executable file named `name` in `path` (the value of `PATH`); an empty entry
/// is skipped, not read as the current directory.
pub(crate) fn find_bin_in(name: &str, path: &OsStr) -> Option<PathBuf> {
    std::env::split_paths(path)
        .filter(|d| !d.as_os_str().is_empty())
        .map(|d| d.join(name))
        .find(|p| is_executable(p))
}

/// `find_bin_in` over the `PATH` of this process.
pub(crate) fn find_bin(name: &str) -> Option<PathBuf> {
    find_bin_in(name, &std::env::var_os("PATH")?)
}

fn is_executable(p: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        p.is_file()
    }
}

/// `hc label <terminal> armadai.flow=<flow> armadai.step=<step>`, 2 s at most; every failure
/// (spawn, exit code, delay) is ignored, and nothing of the child reaches our streams.
pub(crate) fn label_step(hc: &Path, terminal: &str, flow: &str, step: &str) {
    let mut cmd = Command::new(hc);
    cmd.args([
        "label",
        terminal,
        &format!("armadai.flow={flow}"),
        &format!("armadai.step={step}"),
    ])
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::null());
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    if let Ok(mut child) = cmd.spawn() {
        let _ = crate::process::finish(&mut child, Duration::from_secs(2), false);
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn bin(dir: &Path, name: &str, mode: u32) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(mode)).unwrap();
        p
    }

    fn join(dirs: &[&Path]) -> std::ffi::OsString {
        std::env::join_paths(dirs).unwrap()
    }

    #[test]
    fn the_first_directory_of_the_path_wins() {
        let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let in_a = bin(a.path(), "hc", 0o755);
        bin(b.path(), "hc", 0o755);
        assert_eq!(find_bin_in("hc", &join(&[a.path(), b.path()])), Some(in_a));
    }

    #[test]
    fn a_file_without_the_x_bit_is_skipped() {
        let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        bin(a.path(), "hc", 0o644);
        let in_b = bin(b.path(), "hc", 0o755);
        assert_eq!(find_bin_in("hc", &join(&[a.path(), b.path()])), Some(in_b));
        assert_eq!(find_bin_in("hc", &join(&[a.path()])), None);
    }

    #[test]
    fn an_absent_binary_is_none_and_an_empty_entry_is_not_the_cwd() {
        let a = tempfile::tempdir().unwrap();
        assert_eq!(find_bin_in("hc", &join(&[a.path()])), None);
        // an empty PATH entry must not resolve against the current directory
        let cwd_file = std::env::current_dir()
            .unwrap()
            .join("cargo-test-herdr-bin");
        std::fs::write(&cwd_file, "").unwrap();
        std::fs::set_permissions(&cwd_file, std::fs::Permissions::from_mode(0o755)).unwrap();
        let found = find_bin_in("cargo-test-herdr-bin", OsStr::new(":"));
        std::fs::remove_file(&cwd_file).unwrap();
        assert_eq!(found, None);
    }
}
