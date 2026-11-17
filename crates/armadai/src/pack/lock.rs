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

/// What `.armadai/pack.lock` holds: the team pack's entry, and one per standards pack.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LockFile {
    pub team: Option<PackLock>,
    pub standards: Vec<PackLock>,
}

impl LockFile {
    /// The team entry, else the first standards entry.
    pub fn first(&self) -> Option<&PackLock> {
        self.team.as_ref().or(self.standards.first())
    }

    pub fn standards_entry(&self, name: &str) -> Option<&PackLock> {
        self.standards.iter().find(|e| e.pack == name)
    }
}

impl From<PackLock> for LockFile {
    fn from(team: PackLock) -> Self {
        LockFile {
            team: Some(team),
            standards: Vec::new(),
        }
    }
}

/// The on-disk shape; field order is the file's order, so an rc.13 lock is the same text.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLock {
    format: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pack: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    resolved: Option<Resolved>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    standards: Vec<RawEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEntry {
    pack: String,
    source: String,
    resolved: Resolved,
}

impl RawLock {
    fn into_file(self, shown: &str) -> anyhow::Result<LockFile> {
        let format = self.format;
        let team = match (self.pack, self.source, self.resolved) {
            (Some(pack), Some(source), Some(resolved)) => Some(PackLock {
                format,
                pack,
                source,
                resolved,
            }),
            (None, None, None) => None,
            _ => anyhow::bail!("{shown}: pack, source and resolved go together"),
        };
        let standards = self
            .standards
            .into_iter()
            .map(|e| PackLock {
                format,
                pack: e.pack,
                source: e.source,
                resolved: e.resolved,
            })
            .collect();
        Ok(LockFile { team, standards })
    }

    fn from_file(lock: &LockFile) -> Self {
        let format = lock.first().map_or(1, |l| l.format);
        RawLock {
            format,
            pack: lock.team.as_ref().map(|t| t.pack.clone()),
            source: lock.team.as_ref().map(|t| t.source.clone()),
            resolved: lock.team.as_ref().map(|t| t.resolved.clone()),
            standards: lock
                .standards
                .iter()
                .map(|e| RawEntry {
                    pack: e.pack.clone(),
                    source: e.source.clone(),
                    resolved: e.resolved.clone(),
                })
                .collect(),
        }
    }
}

fn path(root: &Path) -> PathBuf {
    root.join(LOCK)
}

/// Where this checkout's pin lives: `<git dir>/armadai/pack.lock`; `Ok(None)` outside git;
/// `Err(T14)` when the nearest `.git` does not lead to a git directory
/// (`worktree::git_dir_checked`): the pin is never looked for in another checkout.
/// Git never lists that folder, and `git worktree remove` takes it with the worktree.
pub fn pin_path(root: &Path) -> anyhow::Result<Option<PathBuf>> {
    match crate::worktree::git_dir_checked(root) {
        Ok(gd) => Ok(gd.map(|g| g.join("armadai/pack.lock"))),
        Err(dot_git) => anyhow::bail!(
            "{} does not lead to a git directory, where this checkout keeps its pack pin; repair it (git worktree repair) and run the command again",
            dot_git.display()
        ),
    }
}

fn read_file(p: &Path, shown: &str) -> anyhow::Result<Option<LockFile>> {
    let text = match std::fs::read_to_string(p) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => anyhow::bail!("{}: {e}", p.display()),
    };
    let raw: RawLock =
        armadai_model::yaml::from_str(&text).map_err(|e| anyhow::anyhow!("{shown}: {e}"))?;
    raw.into_file(shown).map(Some)
}

/// The pin of this checkout, if any. A symbolic link (on the file or on `armadai/`) is
/// refused, not read through.
pub fn read_pin(root: &Path) -> anyhow::Result<Option<LockFile>> {
    let Some(p) = pin_path(root)? else {
        return Ok(None);
    };
    for f in [Some(&*p), p.parent()].into_iter().flatten() {
        if std::fs::symlink_metadata(f).is_ok_and(|m| m.file_type().is_symlink()) {
            anyhow::bail!("{} is a symbolic link; it is not read through", f.display());
        }
    }
    read_file(&p, "the pack pin of this checkout")
}

/// `.armadai/pack.lock` only; `None` when it is absent.
pub fn read_committed(root: &Path) -> anyhow::Result<Option<LockFile>> {
    read_file(&path(root), LOCK)
}

/// The lock this checkout uses: its pin when it has one, else `.armadai/pack.lock`.
pub fn read(root: &Path) -> anyhow::Result<Option<LockFile>> {
    match read_pin(root)? {
        Some(pin) => Ok(Some(pin)),
        None => read_committed(root),
    }
}

/// T7: what `pack update` and `pack add` say when the checkout is pinned.
pub fn pinned_refusal(pin: &LockFile) -> String {
    let (kind, l) = match (&pin.team, pin.standards.first()) {
        (Some(t), _) => ("pack", Some(t)),
        (None, s) => ("standards", s),
    };
    let (name, version) = l.map_or(("", ""), |l| (&l.pack, &l.resolved.version));
    format!(
        "refused: this checkout pins {kind} {name} {version} (armadai pack pin); run armadai pack unpin first"
    )
}

/// Refused while this checkout pins its pack (T7): the lock a pin masks is not rewritten.
pub fn write(root: &Path, lock: &LockFile) -> anyhow::Result<()> {
    if let Some(pin) = read_pin(root)? {
        anyhow::bail!("{}", pinned_refusal(&pin));
    }
    write_file(&path(root), lock)
}

/// Written whole (`.tmp` then rename), never through a symbolic link.
fn write_file(p: &Path, lock: &LockFile) -> anyhow::Result<()> {
    let tmp = p.with_extension("lock.tmp");
    for f in [p, &tmp] {
        if std::fs::symlink_metadata(f).is_ok_and(|m| m.file_type().is_symlink()) {
            anyhow::bail!(
                "{} is a symbolic link; it is not written through",
                f.display()
            );
        }
    }
    let text = armadai_model::yaml::to_string(&RawLock::from_file(lock))
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, p)?;
    Ok(())
}

/// `pack pin`: copies `.armadai/pack.lock` to this checkout's pin (T1 to T4); the project
/// references a pack (`pack:`), standards packs (`standards:`) or both.
pub fn pin(root: &Path) -> anyhow::Result<LockFile> {
    let team = super::effective::pack_ref(root);
    let standards = super::effective::standards_locked_refs(root);
    if team.is_none() && standards.is_empty() {
        anyhow::bail!(
            "this project references no pack (pack: in .armadai/project.yaml); a pack installed by copy is already in the checkout and needs no pin"
        );
    }
    let Some(p) = pin_path(root).map_err(|e| anyhow::anyhow!("pack pin refused: {e}"))? else {
        anyhow::bail!(
            "pack pin needs a git checkout: the pin is kept in its git directory, and there is no .git above {}",
            root.display()
        );
    };
    if let Some(old) = read_pin(root)? {
        let (kind, l) = match (&old.team, old.standards.first()) {
            (Some(t), _) => ("pack", Some(t)),
            (None, s) => ("standards", s),
        };
        anyhow::bail!(
            "this checkout already pins {kind} {} {}; run armadai pack unpin first",
            l.map_or("", |l| &l.pack),
            l.map_or("", |l| &l.resolved.version)
        );
    }
    let committed = read_committed(root)?;
    if let Some((name, _)) = &team
        && committed.as_ref().is_none_or(|l| l.team.is_none())
    {
        anyhow::bail!("{}", super::effective::lock_missing(name));
    }
    for (name, ..) in &standards {
        if committed
            .as_ref()
            .is_none_or(|l| l.standards_entry(name).is_none())
        {
            anyhow::bail!(
                "{}",
                super::effective::standards_missing(name, committed.is_some())
            );
        }
    }
    let lock = committed.expect("a referenced pack has a lock");
    write_file(&p, &lock)?;
    Ok(lock)
}

/// `pack unpin`: removes the pin (the folder around it stays); the pin it held, if any.
pub fn unpin(root: &Path) -> anyhow::Result<Option<LockFile>> {
    pin_path(root).map_err(|e| anyhow::anyhow!("pack unpin refused: {e}"))?;
    let Some(old) = read_pin(root)? else {
        return Ok(None);
    };
    if let Some(p) = pin_path(root)? {
        std::fs::remove_file(p)?;
    }
    Ok(Some(old))
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

    fn sample() -> LockFile {
        entry().into()
    }

    fn entry() -> PackLock {
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
            let yaml = armadai_model::yaml::from_str::<RawLock>(&bad).unwrap_err();
            assert_eq!(
                read(tmp.path()).unwrap_err().to_string(),
                format!(".armadai/pack.lock: {yaml}")
            );
            assert!(yaml.to_string().contains("branch"), "{yaml}");
        }
    }

    #[test]
    fn lock_id_is_the_commit_not_the_tag() {
        let r = entry().resolved;
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

    // ---- the worktree pin ----

    /// A main checkout `m` (a `.git` folder) and a linked worktree `wt` of it, both
    /// referencing pack `demo`; returns (main, worktree).
    fn checkouts(t: &Path) -> (PathBuf, PathBuf) {
        let (m, wt) = (t.join("m"), t.join("wt"));
        let gd = m.join(".git/worktrees/wt");
        std::fs::create_dir_all(&gd).unwrap();
        std::fs::write(wt_dot_git(&wt), format!("gitdir: {}\n", gd.display())).unwrap();
        for r in [&m, &wt] {
            std::fs::create_dir_all(r.join(".armadai")).unwrap();
            std::fs::write(
                r.join(".armadai/project.yaml"),
                "schema_version: 1\npack: { name: demo, source: 'https://example.com/packs.git', version: latest }\n",
            )
            .unwrap();
        }
        (m, wt)
    }

    fn wt_dot_git(wt: &Path) -> PathBuf {
        std::fs::create_dir_all(wt).unwrap();
        wt.join(".git")
    }

    fn at(version: &str, commit: char) -> LockFile {
        let mut l = entry();
        l.resolved.version = version.into();
        l.resolved.tag = Some(format!("demo-v{version}"));
        l.resolved.commit = Some(commit.to_string().repeat(40));
        l.into()
    }

    #[test]
    fn a_pin_wins_over_the_committed_lock_and_lives_in_the_worktree_git_dir() {
        let t = tempfile::tempdir().unwrap();
        let (m, wt) = checkouts(t.path());
        write(&wt, &at("1.3.0", '3')).unwrap();
        assert_eq!(pin(&wt).unwrap(), at("1.3.0", '3'));
        assert_eq!(
            pin_path(&wt).unwrap(),
            Some(m.join(".git/worktrees/wt/armadai/pack.lock"))
        );
        // the main checkout has no pin of its own
        assert_eq!(read_pin(&m).unwrap(), None);
        // the committed lock moves on, as a `pack update` of the main checkout would
        write_file(&path(&wt), &at("1.4.0", '4')).unwrap();
        assert_eq!(read(&wt).unwrap(), Some(at("1.3.0", '3')));
        assert_eq!(read_pin(&wt).unwrap(), Some(at("1.3.0", '3')));
        assert_eq!(read_committed(&wt).unwrap(), Some(at("1.4.0", '4')));
        assert_eq!(unpin(&wt).unwrap(), Some(at("1.3.0", '3')));
        assert_eq!(read(&wt).unwrap(), Some(at("1.4.0", '4')));
    }

    #[test]
    fn write_is_refused_under_a_pin_and_leaves_the_committed_lock() {
        let t = tempfile::tempdir().unwrap();
        let (_, wt) = checkouts(t.path());
        write(&wt, &at("1.3.0", '3')).unwrap();
        pin(&wt).unwrap();
        let err = write(&wt, &at("1.4.0", '4')).unwrap_err();
        assert_eq!(
            err.to_string(),
            "refused: this checkout pins pack demo 1.3.0 (armadai pack pin); run armadai pack unpin first"
        );
        assert_eq!(read_committed(&wt).unwrap(), Some(at("1.3.0", '3')));
    }

    #[test]
    fn pin_refusals_are_the_planned_texts() {
        let t = tempfile::tempdir().unwrap();
        let (m, wt) = checkouts(t.path());
        // a lock is missing
        assert_eq!(
            pin(&wt).unwrap_err().to_string(),
            ".armadai/project.yaml references pack demo but .armadai/pack.lock is missing: run `armadai pack update`"
        );
        write(&wt, &at("1.3.0", '3')).unwrap();
        pin(&wt).unwrap();
        assert_eq!(
            pin(&wt).unwrap_err().to_string(),
            "this checkout already pins pack demo 1.3.0; run armadai pack unpin first"
        );
        // a copy-mode project
        std::fs::write(m.join(".armadai/project.yaml"), "schema_version: 1\n").unwrap();
        assert_eq!(
            pin(&m).unwrap_err().to_string(),
            "this project references no pack (pack: in .armadai/project.yaml); a pack installed by copy is already in the checkout and needs no pin"
        );
        assert_eq!(unpin(&m).unwrap(), None);
    }

    #[test]
    fn a_pin_outside_git_is_refused_by_name() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("plain");
        std::fs::create_dir_all(root.join(".armadai")).unwrap();
        std::fs::write(
            root.join(".armadai/project.yaml"),
            "schema_version: 1\npack: { name: demo, source: 'https://example.com/packs.git', version: latest }\n",
        )
        .unwrap();
        assert_eq!(
            pin_path(&root).unwrap(),
            None,
            "the temporary folder sits inside a git repository"
        );
        assert_eq!(read(&root).unwrap(), None);
        assert_eq!(
            pin(&root).unwrap_err().to_string(),
            format!(
                "pack pin needs a git checkout: the pin is kept in its git directory, and there is no .git above {}",
                root.display()
            )
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_pin_that_is_a_link_is_refused_not_read_through() {
        let t = tempfile::tempdir().unwrap();
        let (_, wt) = checkouts(t.path());
        let real = t.path().join("elsewhere");
        std::fs::write(&real, "format: 1\n").unwrap();
        let p = pin_path(&wt).unwrap().unwrap();
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&real, &p).unwrap();
        let want = format!("{} is a symbolic link; it is not read through", p.display());
        assert_eq!(read_pin(&wt).unwrap_err().to_string(), want);
        assert_eq!(read(&wt).unwrap_err().to_string(), want);
        assert_eq!(unpin(&wt).unwrap_err().to_string(), want);
        // the folder around the pin as a link
        std::fs::remove_file(&p).unwrap();
        let dir = p.parent().unwrap().to_path_buf();
        std::fs::remove_dir(&dir).unwrap();
        let other = t.path().join("other");
        std::fs::create_dir_all(&other).unwrap();
        std::os::unix::fs::symlink(&other, &dir).unwrap();
        write_file(&path(&wt), &sample()).unwrap();
        let want = format!(
            "{} is a symbolic link; it is not read through",
            dir.display()
        );
        assert_eq!(pin(&wt).unwrap_err().to_string(), want);
        assert!(!other.join("pack.lock").exists());
    }

    #[test]
    fn a_lock_written_before_the_pin_existed_still_reads() {
        let t = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(t.path().join(".armadai")).unwrap();
        std::fs::write(
            t.path().join(LOCK),
            "format: 1\npack: demo\nsource: 'https://example.com/packs.git'\nresolved:\n  version: 1.3.2\n  tag: demo-v1.3.2\n  commit: 0123456789abcdef0123456789abcdef01234567\n",
        )
        .unwrap();
        assert_eq!(read(t.path()).unwrap(), Some(sample()));
    }

    const RC13: &str = "format: 1\npack: demo\nsource: https://example.com/packs.git\nresolved:\n  version: \"1.3.2\"\n  tag: demo-v1.3.2\n  commit: 0123456789abcdef0123456789abcdef01234567\n";

    fn with_dir(text: &str) -> tempfile::TempDir {
        let t = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(t.path().join(".armadai")).unwrap();
        std::fs::write(t.path().join(LOCK), text).unwrap();
        t
    }

    #[test]
    fn an_rc13_lock_is_rewritten_byte_for_byte() {
        let t = with_dir(RC13);
        let lock = read_committed(t.path()).unwrap().unwrap();
        write(t.path(), &lock).unwrap();
        assert_eq!(std::fs::read_to_string(t.path().join(LOCK)).unwrap(), RC13);
    }

    fn std_entry(name: &str, version: &str, c: char) -> PackLock {
        let mut e = entry();
        e.pack = name.into();
        e.source = format!("https://example.com/{name}.git");
        e.resolved.version = version.into();
        e.resolved.tag = Some(format!("{name}-v{version}"));
        e.resolved.commit = Some(c.to_string().repeat(40));
        e
    }

    #[test]
    fn a_lock_with_standards_roundtrips() {
        let t = tempfile::tempdir().unwrap();
        let lock = LockFile {
            team: Some(entry()),
            standards: vec![
                std_entry("rust-std", "2.0.1", 'a'),
                std_entry("web-std", "0.4.0", 'b'),
            ],
        };
        write(t.path(), &lock).unwrap();
        assert_eq!(read(t.path()).unwrap(), Some(lock.clone()));
        assert_eq!(
            lock.standards_entry("web-std").unwrap().resolved.version,
            "0.4.0"
        );
        assert_eq!(lock.first().unwrap().pack, "demo");
    }

    #[test]
    fn a_standards_only_lock_reads() {
        let t = with_dir(
            "format: 1\nstandards:\n- pack: rust-std\n  source: https://example.com/rust-std.git\n  resolved:\n    version: 2.0.1\n    commit: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n",
        );
        let lock = read(t.path()).unwrap().unwrap();
        assert_eq!(lock.team, None);
        assert_eq!(lock.standards.len(), 1);
        assert_eq!(lock.standards[0].pack, "rust-std");
        assert_eq!(lock.standards[0].format, 1);
        assert_eq!(lock.first().unwrap().pack, "rust-std");
    }

    #[test]
    fn a_partial_team_is_refused() {
        let t = with_dir("format: 1\npack: demo\nsource: https://example.com/p.git\n");
        assert_eq!(
            read(t.path()).unwrap_err().to_string(),
            ".armadai/pack.lock: pack, source and resolved go together"
        );
    }

    #[test]
    fn an_unknown_key_in_a_standards_entry_is_refused() {
        let t = with_dir(
            "format: 1\nstandards:\n- pack: rust-std\n  source: https://example.com/rust-std.git\n  branch: main\n  resolved:\n    version: 2.0.1\n    commit: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n",
        );
        let e = read(t.path()).unwrap_err().to_string();
        assert!(e.starts_with(".armadai/pack.lock: "), "{e}");
        assert!(e.contains("branch"), "{e}");
    }

    #[test]
    fn a_pin_copies_the_whole_file_and_unpin_returns_it() {
        let t = tempfile::tempdir().unwrap();
        let (_, wt) = checkouts(t.path());
        let lock = LockFile {
            team: Some(entry()),
            standards: vec![std_entry("rust-std", "2.0.1", 'a')],
        };
        write(&wt, &lock).unwrap();
        assert_eq!(pin(&wt).unwrap(), lock);
        assert_eq!(read_pin(&wt).unwrap(), Some(lock.clone()));
        assert_eq!(unpin(&wt).unwrap(), Some(lock));
    }

    #[test]
    fn a_standards_only_pin_is_refused_with_its_own_text() {
        let pin = LockFile {
            team: None,
            standards: vec![std_entry("rust-std", "2.0.1", 'a')],
        };
        assert_eq!(
            pinned_refusal(&pin),
            "refused: this checkout pins standards rust-std 2.0.1 (armadai pack pin); run armadai pack unpin first"
        );
    }

    /// A project referencing pack `demo`, in `dir`.
    fn project_in(dir: &Path) {
        std::fs::create_dir_all(dir.join(".armadai")).unwrap();
        std::fs::write(
            dir.join(".armadai/project.yaml"),
            "schema_version: 1\npack: { name: demo, source: 'https://example.com/packs.git', version: latest }\n",
        )
        .unwrap();
    }

    fn broken(dot_git: &Path) -> String {
        format!(
            "{} does not lead to a git directory, where this checkout keeps its pack pin; repair it (git worktree repair) and run the command again",
            dot_git.display()
        )
    }

    #[cfg(unix)]
    #[test]
    fn a_dangling_dot_git_never_borrows_the_pin_of_the_checkout_above() {
        let t = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(t.path()).unwrap();
        let m = root.join("main");
        project_in(&m);
        std::fs::create_dir_all(m.join(".git")).unwrap();
        write(&m, &at("1.3.0", '3')).unwrap();
        pin(&m).unwrap();
        let main_pin = std::fs::read(m.join(".git/armadai/pack.lock")).unwrap();
        let dang = m.join("dang");
        project_in(&dang);
        std::os::unix::fs::symlink(root.join("nowhere"), dang.join(".git")).unwrap();
        let want = broken(&dang.join(".git"));
        assert_eq!(read(&dang).unwrap_err().to_string(), want);
        assert_eq!(
            unpin(&dang).unwrap_err().to_string(),
            format!("pack unpin refused: {want}")
        );
        assert_eq!(
            std::fs::read(m.join(".git/armadai/pack.lock")).unwrap(),
            main_pin
        );
    }

    #[test]
    fn a_gitdir_to_a_missing_folder_is_refused_and_creates_nothing() {
        let t = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(t.path()).unwrap();
        let gone = root.join("main/gone");
        project_in(&gone);
        let absent = root.join("absent");
        write_file(&path(&gone), &at("1.3.0", '3')).unwrap();
        std::fs::write(
            gone.join(".git"),
            format!("gitdir: {}\n", absent.join(".git/worktrees/gone").display()),
        )
        .unwrap();
        assert_eq!(
            pin(&gone).unwrap_err().to_string(),
            format!("pack pin refused: {}", broken(&gone.join(".git")))
        );
        assert!(!absent.exists());
    }
}
