//! `.armadai/pack.lock`: the version of a referenced pack that `link` uses.
//! Only `pack update` advances it; the commit (or archive digest) is what counts.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::starters_registry::{git_failure, path_arg};

pub const LOCK: &str = ".armadai/pack.lock";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackLock {
    pub format: u32,
    pub pack: String,
    pub source: String,
    pub resolved: Resolved,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Resolved {
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
}

impl Resolved {
    /// The commit, else `sha256-<hex>` of the digest: what `link` reads (the tag is never
    /// trusted alone) and the snapshot folder's name.
    pub fn id(&self) -> String {
        self.commit
            .clone()
            .or_else(|| self.digest.as_ref().map(|d| d.replace(':', "-")))
            .unwrap_or_default()
    }
}

fn path(root: &Path) -> PathBuf {
    root.join(LOCK)
}

/// `None` when the lock is absent.
pub fn read(root: &Path) -> anyhow::Result<Option<PackLock>> {
    let p = path(root);
    let text = match std::fs::read_to_string(&p) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => anyhow::bail!("{}: {e}", p.display()),
    };
    armadai_model::yaml::from_str(&text)
        .map(Some)
        .map_err(|e| anyhow::anyhow!("{LOCK}: {e}"))
}

/// Written whole (`.tmp` then rename), never through a symbolic link.
pub fn write(root: &Path, lock: &PackLock) -> anyhow::Result<()> {
    let p = path(root);
    let tmp = p.with_extension("lock.tmp");
    for f in [&p, &tmp] {
        if std::fs::symlink_metadata(f).is_ok_and(|m| m.file_type().is_symlink()) {
            anyhow::bail!(
                "{} is a symbolic link; it is not written through",
                f.display()
            );
        }
    }
    let text = armadai_model::yaml::to_string(lock).map_err(|e| anyhow::anyhow!("{e}"))?;
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, &p)?;
    Ok(())
}

/// What became of the lock's tag in the clone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TagState {
    /// The tag now points to this commit, not the lock's.
    Moved(String),
    /// The tag is no longer in the clone.
    Gone,
}

/// A tag of the lock is always `<pack>-v<semver>`: nothing else becomes a `git` argument.
fn valid_tag(pack: &str, tag: &str) -> bool {
    let plain = !pack.is_empty()
        && !pack.starts_with('-')
        && pack
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    plain
        && tag
            .strip_prefix(pack)
            .and_then(|t| t.strip_prefix("-v"))
            .is_some_and(|v| semver::Version::parse(v).is_ok())
}

/// `None` when the tag still points to the lock's commit, or the lock has no tag or
/// commit (an archive). `pack sync` only.
pub fn tag_moved(
    clone: &Path,
    pack: &str,
    resolved: &Resolved,
) -> anyhow::Result<Option<TagState>> {
    let (Some(tag), Some(commit)) = (&resolved.tag, &resolved.commit) else {
        return Ok(None);
    };
    anyhow::ensure!(
        valid_tag(pack, tag),
        "pack {pack} has an invalid name or version id: {tag}"
    );
    let dir = path_arg(clone)?;
    let rev = format!("refs/tags/{tag}^{{commit}}");
    let args = [
        "-C",
        dir,
        "rev-parse",
        "--verify",
        "--quiet",
        "--end-of-options",
        &rev,
    ];
    let out = std::process::Command::new("git").args(args).output()?;
    match out.status.code() {
        Some(0) => {
            let now = String::from_utf8_lossy(&out.stdout).trim().to_string();
            Ok((now != *commit).then_some(TagState::Moved(now)))
        }
        Some(1) => Ok(Some(TagState::Gone)),
        _ => anyhow::bail!(git_failure(&args, &String::from_utf8_lossy(&out.stderr))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::starters_registry::git_output;

    fn sample() -> PackLock {
        PackLock {
            format: 1,
            pack: "demo".into(),
            source: "https://example.com/packs.git".into(),
            resolved: Resolved {
                version: "1.3.2".into(),
                tag: Some("demo-v1.3.2".into()),
                commit: Some("0123456789abcdef0123456789abcdef01234567".into()),
                digest: None,
            },
        }
    }

    #[test]
    fn lock_roundtrips_and_refuses_unknown_keys() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(read(tmp.path()).unwrap().is_none());
        write(tmp.path(), &sample()).unwrap();
        assert_eq!(read(tmp.path()).unwrap(), Some(sample()));
        let text = std::fs::read_to_string(tmp.path().join(LOCK)).unwrap();
        // an unknown key at the root, then one under `resolved:`
        for bad in [
            format!("{text}branch: main\n"),
            format!("{text}  branch: main\n"),
        ] {
            std::fs::write(tmp.path().join(LOCK), &bad).unwrap();
            let yaml = armadai_model::yaml::from_str::<PackLock>(&bad).unwrap_err();
            assert_eq!(
                read(tmp.path()).unwrap_err().to_string(),
                format!(".armadai/pack.lock: {yaml}")
            );
            assert!(yaml.to_string().contains("branch"), "{yaml}");
        }
    }

    #[test]
    fn lock_id_is_the_commit_not_the_tag() {
        let r = sample().resolved;
        assert_eq!(r.id(), r.commit.clone().unwrap());
        let archive = Resolved {
            version: "1.0.0".into(),
            tag: None,
            commit: None,
            digest: Some("sha256:ab".into()),
        };
        assert_eq!(archive.id(), "sha256-ab");
    }

    fn git(dir: &Path, args: &[&str]) -> String {
        let mut full = vec![
            "-C",
            dir.to_str().unwrap(),
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "tag.gpgsign=false",
        ];
        full.extend(args);
        git_output(&full).unwrap()
    }

    #[test]
    fn tag_moved_is_reported() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path();
        git(repo, &["init", "-q"]);
        std::fs::write(repo.join("a"), "1").unwrap();
        git(repo, &["add", "-A"]);
        git(repo, &["commit", "-qm", "one"]);
        git(repo, &["tag", "demo-v1.0.0"]);
        let first = git(repo, &["rev-parse", "HEAD"]);
        let resolved = Resolved {
            version: "1.0.0".into(),
            tag: Some("demo-v1.0.0".into()),
            commit: Some(first),
            digest: None,
        };
        assert_eq!(tag_moved(repo, "demo", &resolved).unwrap(), None);
        std::fs::write(repo.join("a"), "2").unwrap();
        git(repo, &["commit", "-qam", "two"]);
        git(repo, &["tag", "-f", "demo-v1.0.0"]);
        let second = git(repo, &["rev-parse", "HEAD"]);
        assert_eq!(
            tag_moved(repo, "demo", &resolved).unwrap(),
            Some(TagState::Moved(second))
        );
        git(repo, &["tag", "-d", "demo-v1.0.0"]);
        assert_eq!(
            tag_moved(repo, "demo", &resolved).unwrap(),
            Some(TagState::Gone)
        );
    }

    #[test]
    fn a_tag_that_is_an_option_is_an_error_never_moved() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path();
        git(repo, &["init", "-q"]);
        std::fs::write(repo.join("a"), "1").unwrap();
        git(repo, &["add", "-A"]);
        git(repo, &["commit", "-qm", "one"]);
        for tag in [
            "--contains=HEAD",
            "--points-at=HEAD",
            "-n",
            "--sort=refname",
            "*",
            "other-v1.0.0",
            "demo-v1.0.0\u{2e}\u{2e}x",
        ] {
            let resolved = Resolved {
                version: "1.0.0".into(),
                tag: Some(tag.into()),
                commit: Some("0".repeat(40)),
                digest: None,
            };
            let got = tag_moved(repo, "demo", &resolved);
            assert_eq!(
                got.unwrap_err().to_string(),
                format!("pack demo has an invalid name or version id: {tag}"),
            );
        }
    }

    #[test]
    fn an_empty_pack_name_never_makes_a_tag() {
        let tmp = tempfile::tempdir().unwrap();
        let resolved = Resolved {
            version: "1.0.0".into(),
            tag: Some("-v1.0.0".into()),
            commit: Some("0".repeat(40)),
            digest: None,
        };
        assert_eq!(
            tag_moved(tmp.path(), "", &resolved)
                .unwrap_err()
                .to_string(),
            "pack  has an invalid name or version id: -v1.0.0"
        );
    }

    #[test]
    fn a_failing_git_reports_git_s_own_message() {
        let tmp = tempfile::tempdir().unwrap(); // not a repository
        let resolved = Resolved {
            version: "1.0.0".into(),
            tag: Some("demo-v1.0.0".into()),
            commit: Some("0".repeat(40)),
            digest: None,
        };
        let dir = tmp.path().to_str().unwrap();
        let rev = "refs/tags/demo-v1.0.0^{commit}";
        let out = std::process::Command::new("git")
            .args([
                "-C",
                dir,
                "rev-parse",
                "--verify",
                "--quiet",
                "--end-of-options",
                rev,
            ])
            .output()
            .unwrap();
        let want = git_failure(
            &[
                "-C",
                dir,
                "rev-parse",
                "--verify",
                "--quiet",
                "--end-of-options",
                rev,
            ],
            &String::from_utf8_lossy(&out.stderr),
        );
        assert!(want.contains("not a git repository"), "{want}");
        let got = tag_moved(tmp.path(), "demo", &resolved).unwrap_err();
        assert_eq!(got.to_string(), want);
    }

    #[cfg(unix)]
    #[test]
    fn write_refuses_a_link() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("target");
        std::fs::write(&target, "keep").unwrap();
        std::fs::create_dir_all(tmp.path().join(".armadai")).unwrap();
        std::os::unix::fs::symlink(&target, tmp.path().join(LOCK)).unwrap();
        let err = write(tmp.path(), &sample()).unwrap_err();
        assert_eq!(
            err.to_string(),
            format!(
                "{} is a symbolic link; it is not written through",
                tmp.path().join(LOCK).display()
            )
        );
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "keep");
    }

    #[cfg(unix)]
    #[test]
    fn write_refuses_a_link_at_the_temporary_name() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("target");
        std::fs::write(&target, "keep").unwrap();
        std::fs::create_dir_all(tmp.path().join(".armadai")).unwrap();
        let tmp_name = tmp.path().join(".armadai/pack.lock.tmp");
        std::os::unix::fs::symlink(&target, &tmp_name).unwrap();
        let err = write(tmp.path(), &sample()).unwrap_err();
        assert_eq!(
            err.to_string(),
            format!(
                "{} is a symbolic link; it is not written through",
                tmp_name.display()
            )
        );
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "keep");
    }
}
