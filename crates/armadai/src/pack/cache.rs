//! The versioned index of a pack source and `versions.yaml`, what `pack sync`
//! records so that every other surface reads versions without the network or `git`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use armadai_core::registries::{RegistrySource, SourceKind, cache_key};
use armadai_core::starter::discover_packs;
use serde::{Deserialize, Serialize};

use crate::linker::manifest::digest_of;
use crate::starters_registry::{
    ArchiveFetcher, EXTRACT_LIMITS, GitFetcher, StarterFetcher, complete_clone, extract_archive,
    git_output, path_arg, private_tempdir, reject_escaping_links,
};

const INDEX_FILE: &str = "armadai-packs.yaml";
const VERSIONS_FILE: &str = "versions.yaml";

/// `<source>/armadai-packs.yaml`: the packs of a source and their current version.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Index {
    pub format: u32,
    #[serde(default)]
    pub packs: Vec<IndexPack>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexPack {
    pub name: String,
    /// Folder of the pack inside the source.
    pub path: String,
    /// Version of the content at the current commit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

/// `<source>/armadai-packs.yaml`, or `None` (absent, or not readable: a warning).
pub fn read_index(source_dir: &Path) -> Option<Index> {
    let path = source_dir.join(INDEX_FILE);
    let text = std::fs::read_to_string(&path).ok()?;
    match armadai_model::yaml::from_str::<Index>(&text) {
        Ok(index) => Some(index),
        Err(e) => {
            eprintln!("  warn: {} is not readable: {e}", path.display());
            None
        }
    }
}

/// `versions.yaml`: what the last `pack sync` of a source found.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Versions {
    pub format: u32,
    pub packs: BTreeMap<String, PackVersions>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackVersions {
    pub path: String,
    #[serde(default)]
    pub versions: Vec<Available>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Available {
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
}

impl Available {
    /// The commit, else `sha256-<hex>` of the digest: the snapshot folder name.
    pub fn id(&self) -> String {
        match (&self.commit, &self.digest) {
            (Some(commit), _) => commit.clone(),
            (None, Some(digest)) => digest.replace(':', "-"),
            (None, None) => String::new(),
        }
    }
}

/// `<registry>/packs/<cache_key(url)>`.
pub fn store_dir_in(registry: &Path, url: &str) -> PathBuf {
    registry.join("packs").join(cache_key(url))
}

pub fn store_dir(url: &str) -> PathBuf {
    store_dir_in(&armadai_core::config::registry_cache_dir(), url)
}

/// `<store>/<pack>/<id>`: one pack at one version, written once, never modified.
pub fn snapshot_dir_in(registry: &Path, url: &str, pack: &str, id: &str) -> PathBuf {
    store_dir_in(registry, url).join(pack).join(id)
}

/// The snapshot of `pack` at `available`, extracted when absent: git, `git archive
/// <commit>:<path>` from the local clone through `extract_archive` and
/// `reject_escaping_links`; archive, a copy of the cache's pack folder, refused when
/// its `tree_digest` differs from `available.digest`. Built in a staging folder and
/// renamed into place, so a failed one is never visible. No network.
pub fn ensure_snapshot_in(
    registry: &Path,
    source: &RegistrySource,
    pack: &str,
    path: &str,
    available: &Available,
) -> anyhow::Result<PathBuf> {
    let id = available.id();
    anyhow::ensure!(
        !id.is_empty(),
        "pack {pack} has a version without commit or digest"
    );
    // Both become a path and `git` arguments: only plain names and hex ids pass.
    anyhow::ensure!(
        valid_name(pack),
        "pack {pack} has an invalid name or version id: {pack}"
    );
    anyhow::ensure!(
        valid_id(&id),
        "pack {pack} has an invalid name or version id: {id}"
    );
    let dest = snapshot_dir_in(registry, &source.url, pack, &id);
    if dest.is_dir() {
        return Ok(dest);
    }
    let parent = dest.parent().expect("a snapshot has a parent");
    std::fs::create_dir_all(parent)?;
    let staging = private_tempdir(".staging-", parent)?;
    let tree = staging.path().join("tree");
    let clone = registry.join("starters").join(cache_key(&source.url));
    let label = format!("{pack} {}", available.version);
    match (source.resolved_kind(), &available.commit) {
        (SourceKind::Git, Some(commit)) => {
            let tar = staging.path().join("pack.tar.gz");
            git_output(&[
                "-C",
                path_arg(&clone)?,
                "archive",
                "--format=tar.gz",
                "-o",
                path_arg(&tar)?,
                &format!("{commit}:{path}"),
            ])?;
            extract_archive(&tar, &tree, &label, EXTRACT_LIMITS)?;
        }
        (SourceKind::Archive, _) => {
            copy_tree(&clone.join(path), &tree)?;
            let found = tree_digest(&tree)?;
            anyhow::ensure!(
                available.digest.as_deref() == Some(found.as_str()),
                "the cache no longer holds pack {label} (digest {found}): run `armadai pack sync`"
            );
        }
        (SourceKind::Git, None) => anyhow::bail!("pack {label} has no commit"),
    }
    reject_escaping_links(&tree, &label)?;
    make_read_only(&tree)?;
    // Two callers may race to the same snapshot: the one that lost finds it in place.
    if let Err(e) = std::fs::rename(&tree, &dest)
        && !dest.is_dir()
    {
        return Err(e.into());
    }
    Ok(dest)
}

/// One plain path component, not an option for `git`.
pub(crate) fn valid_name(name: &str) -> bool {
    let mut parts = Path::new(name).components();
    matches!(parts.next(), Some(std::path::Component::Normal(_)))
        && parts.next().is_none()
        && !name.contains(['/', '\\', ':'])
        && !name.starts_with('-')
}

/// A commit (40 or 64 lowercase hex) or `sha256-<64 hex>`.
pub(crate) fn valid_id(id: &str) -> bool {
    let hex = |s: &str, n: &[usize]| {
        n.contains(&s.len()) && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    };
    hex(id, &[40, 64]) || id.strip_prefix("sha256-").is_some_and(|h| hex(h, &[64]))
}

/// Copy files and links, kept as they are (the digest covers links).
fn copy_tree(from: &Path, to: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let (src, dst) = (e.path(), to.join(e.file_name()));
        let kind = e.file_type()?;
        if kind.is_dir() {
            copy_tree(&src, &dst)?;
        } else if kind.is_symlink() {
            #[cfg(unix)]
            std::os::unix::fs::symlink(std::fs::read_link(&src)?, &dst)?;
        } else {
            std::fs::copy(&src, &dst)?;
        }
    }
    Ok(())
}

/// Files lose their write bits; folders keep theirs so that the cache can still be
/// deleted (shortcut: a file can be added by hand, none edited; make folders read-only
/// too once the cache is removed by a command that restores write bits first).
fn make_read_only(dir: &Path) -> anyhow::Result<()> {
    for e in std::fs::read_dir(dir)? {
        let e = e?;
        let kind = e.file_type()?;
        if kind.is_dir() {
            make_read_only(&e.path())?;
        } else if kind.is_file() {
            let mut perm = e.metadata()?.permissions();
            perm.set_readonly(true);
            std::fs::set_permissions(e.path(), perm)?;
        }
    }
    Ok(())
}

/// `versions.yaml` of a source: no network, no git.
pub fn versions_in(registry: &Path, url: &str) -> Option<Versions> {
    let text = std::fs::read_to_string(store_dir_in(registry, url).join(VERSIONS_FILE)).ok()?;
    armadai_model::yaml::from_str(&text).ok()
}

/// Refresh one source now, whatever its cache holds: git (clone if absent, else
/// `pull --ff-only`, then the whole history and the tags), archive (downloaded
/// again); then write `versions.yaml` and return it.
pub fn sync_source_in(registry: &Path, source: &RegistrySource) -> anyhow::Result<Versions> {
    match source.resolved_kind() {
        SourceKind::Git => sync_source_with(registry, source, &GitFetcher),
        SourceKind::Archive => sync_source_with(registry, source, &ArchiveFetcher),
    }
}

/// [`sync_source_in`] with the fetcher given (tests pass a local one).
pub fn sync_source_with(
    registry: &Path,
    source: &RegistrySource,
    fetcher: &dyn StarterFetcher,
) -> anyhow::Result<Versions> {
    let url = &source.url;
    let clone = registry.join("starters").join(cache_key(url));
    let previous = versions_in(registry, url);
    fetcher.fetch(url, &clone)?;
    let (packs, indexed) = listed_packs(&clone);
    let versions = match source.resolved_kind() {
        SourceKind::Git => {
            complete_clone(&clone, url)?;
            git_versions(&clone, packs, indexed)?
        }
        SourceKind::Archive => archive_versions(&clone, packs, previous)?,
    };
    if source.resolved_kind() == SourceKind::Archive {
        // The next sync replaces the cache: freeze what it holds now.
        for (name, pv) in &versions.packs {
            let Ok(digest) = tree_digest(&clone.join(&pv.path)) else {
                continue;
            };
            if let Some(a) = pv
                .versions
                .iter()
                .find(|a| a.digest.as_ref() == Some(&digest))
            {
                ensure_snapshot_in(registry, source, name, &pv.path, a)?;
            }
        }
    }
    let store = store_dir_in(registry, url);
    std::fs::create_dir_all(&store)?;
    let tmp = store.join(format!("{VERSIONS_FILE}.tmp"));
    std::fs::write(&tmp, armadai_model::yaml::to_string(&versions)?)?;
    std::fs::rename(&tmp, store.join(VERSIONS_FILE))?;
    Ok(versions)
}

/// The packs of a fetched source: the index (`true`), else what `discover_packs` finds
/// (`false`, no version). An index entry needs its `pack.yaml`, as in `discover_packs`.
fn listed_packs(dir: &Path) -> (Vec<IndexPack>, bool) {
    if let Some(index) = read_index(dir) {
        let safe = |p: &str| {
            !Path::new(p).components().any(|c| {
                matches!(
                    c,
                    std::path::Component::ParentDir | std::path::Component::RootDir
                )
            })
        };
        if !index.packs.is_empty() {
            let packs = index
                .packs
                .into_iter()
                .filter(|p| {
                    valid_name(&p.name)
                        && safe(&p.path)
                        && dir.join(&p.path).join("pack.yaml").is_file()
                })
                .collect();
            return (packs, true);
        }
    }
    let found = discover_packs(dir)
        .into_iter()
        .filter_map(|d| {
            let path = d.strip_prefix(dir).ok()?.to_string_lossy().into_owned();
            Some(IndexPack {
                name: d.file_name()?.to_string_lossy().into_owned(),
                path,
                version: None,
            })
        })
        .collect();
    (found, false)
}

/// Tags `<pack>-vX.Y.Z` of the clone, plus the index version at HEAD when no tag carries it.
fn git_versions(clone: &Path, packs: Vec<IndexPack>, indexed: bool) -> anyhow::Result<Versions> {
    let dir = path_arg(clone)?;
    let mut out = BTreeMap::new();
    for pack in packs {
        let mut found: Vec<(semver::Version, Available)> = Vec::new();
        // A pack of a source without index has no version: its tags are not read.
        if indexed {
            let prefix = format!("{}-v", pack.name);
            let tags = git_output(&["-C", dir, "tag", "-l", &format!("{prefix}*")])?;
            for tag in tags.lines() {
                let Some(version) = tag
                    .strip_prefix(&prefix)
                    .and_then(|s| semver::Version::parse(s).ok())
                else {
                    continue;
                };
                let commit = git_output(&["-C", dir, "rev-parse", &format!("{tag}^{{commit}}")])?;
                found.push((
                    version.clone(),
                    Available {
                        version: version.to_string(),
                        tag: Some(tag.to_string()),
                        commit: Some(commit),
                        digest: None,
                    },
                ));
            }
        }
        if let Some(head) = pack
            .version
            .as_deref()
            .and_then(|v| semver::Version::parse(v).ok())
            && !found.iter().any(|(v, _)| *v == head)
        {
            let commit = git_output(&["-C", dir, "rev-parse", "HEAD"])?;
            found.push((
                head.clone(),
                Available {
                    version: head.to_string(),
                    tag: None,
                    commit: Some(commit),
                    digest: None,
                },
            ));
        }
        found.sort_by(|a, b| a.0.cmp(&b.0));
        out.insert(
            pack.name,
            PackVersions {
                path: pack.path,
                versions: found.into_iter().map(|(_, a)| a).collect(),
            },
        );
    }
    Ok(Versions {
        format: 1,
        packs: out,
    })
}

/// The index version and digest of each pack, after the entries of earlier syncs
/// (the next sync replaces the cache: only `versions.yaml` remembers them).
fn archive_versions(
    clone: &Path,
    packs: Vec<IndexPack>,
    previous: Option<Versions>,
) -> anyhow::Result<Versions> {
    let mut out = previous.map(|p| p.packs).unwrap_or_default();
    for pack in packs {
        let entry = out.entry(pack.name).or_insert_with(|| PackVersions {
            path: pack.path.clone(),
            versions: Vec::new(),
        });
        entry.path = pack.path.clone();
        let Some(version) = pack.version else {
            continue;
        };
        let digest = tree_digest(&clone.join(&pack.path))?;
        if !entry
            .versions
            .iter()
            .any(|a| a.digest.as_ref() == Some(&digest))
        {
            entry.versions.push(Available {
                version,
                tag: None,
                commit: None,
                digest: Some(digest),
            });
        }
    }
    Ok(Versions {
        format: 1,
        packs: out,
    })
}

/// `sha256:<hex>` of a pack folder: relative paths sorted, each
/// `path NUL mode-exec NUL len NUL bytes` (a link: its target, mode `l`).
pub fn tree_digest(dir: &Path) -> anyhow::Result<String> {
    let mut files = Vec::new();
    collect(dir, dir, &mut files)?;
    files.sort();
    let mut buf = Vec::new();
    for rel in files {
        let path = dir.join(&rel);
        let meta = std::fs::symlink_metadata(&path)?;
        let (mode, bytes) = if meta.file_type().is_symlink() {
            (
                "l",
                std::fs::read_link(&path)?
                    .to_string_lossy()
                    .into_owned()
                    .into_bytes(),
            )
        } else {
            (
                if is_exec(&meta) { "1" } else { "0" },
                std::fs::read(&path)?,
            )
        };
        buf.extend_from_slice(rel.as_bytes());
        buf.extend_from_slice(format!("\0{mode}\0{}\0", bytes.len()).as_bytes());
        buf.extend_from_slice(&bytes);
    }
    Ok(digest_of(&buf))
}

fn collect(root: &Path, dir: &Path, out: &mut Vec<String>) -> anyhow::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if std::fs::symlink_metadata(&path)?.is_dir() {
            collect(root, &path, out)?;
        } else {
            out.push(
                path.strip_prefix(root)?
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        }
    }
    Ok(())
}

#[cfg(unix)]
fn is_exec(meta: &std::fs::Metadata) -> bool {
    std::os::unix::fs::PermissionsExt::mode(&meta.permissions()) & 0o111 != 0
}

#[cfg(not(unix))]
fn is_exec(_: &std::fs::Metadata) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

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

    /// A local origin repo with `demo/pack.yaml` and (optionally) an index.
    fn origin(tmp: &Path) -> PathBuf {
        let repo = tmp.join("origin");
        fs::create_dir_all(repo.join("demo")).unwrap();
        git(&repo, &["init", "-q"]);
        repo
    }

    fn commit(repo: &Path, files: &[(&str, &str)]) -> String {
        for (rel, body) in files {
            let p = repo.join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, body).unwrap();
        }
        git(repo, &["add", "-A"]);
        git(repo, &["commit", "-qm", "x"]);
        git(repo, &["rev-parse", "HEAD"])
    }

    fn file_url(repo: &Path) -> RegistrySource {
        RegistrySource {
            url: format!("file://{}", repo.display()),
            kind: None,
        }
    }

    fn index(version: &str) -> String {
        format!("format: 1\npacks:\n  - name: demo\n    path: demo\n    version: {version}\n")
    }

    #[test]
    fn sync_refreshes_a_cache_that_already_holds_the_pack() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = origin(tmp.path());
        commit(
            &repo,
            &[("demo/pack.yaml", "name: demo\ndescription: v1\n")],
        );
        let registry = tmp.path().join("registry");
        let source = file_url(&repo);
        sync_source_in(&registry, &source).unwrap();
        commit(
            &repo,
            &[("demo/pack.yaml", "name: demo\ndescription: v2\n")],
        );
        sync_source_in(&registry, &source).unwrap();
        let cached = registry
            .join("starters")
            .join(cache_key(&source.url))
            .join("demo/pack.yaml");
        assert_eq!(
            fs::read_to_string(cached).unwrap(),
            "name: demo\ndescription: v2\n"
        );
    }

    /// Plays the archive download: puts a local folder where the cache goes.
    struct Local(PathBuf);
    impl StarterFetcher for Local {
        fn fetch(&self, _url: &str, dest: &Path) -> anyhow::Result<()> {
            let _ = fs::remove_dir_all(dest);
            fs::create_dir_all(dest.join("demo"))?;
            for rel in ["demo/pack.yaml", INDEX_FILE] {
                fs::copy(self.0.join(rel), dest.join(rel))?;
            }
            Ok(())
        }
    }

    #[test]
    fn sync_refreshes_an_archive_source() {
        let tmp = tempfile::tempdir().unwrap();
        let registry = tmp.path().join("registry");
        let source = RegistrySource {
            url: "https://example.com/packs.tar.gz".into(),
            kind: None,
        };
        let mut digests = Vec::new();
        for (name, version, body) in [
            ("a", "1.0.0", "name: demo\n"),
            ("b", "1.1.0", "name: demo\nx: 1\n"),
        ] {
            let dir = tmp.path().join(name);
            fs::create_dir_all(dir.join("demo")).unwrap();
            fs::write(dir.join("demo/pack.yaml"), body).unwrap();
            fs::write(dir.join(INDEX_FILE), index(version)).unwrap();
            let v = sync_source_with(&registry, &source, &Local(dir)).unwrap();
            digests = v.packs["demo"]
                .versions
                .iter()
                .map(|a| (a.version.clone(), a.digest.clone().unwrap()))
                .collect();
        }
        assert_eq!(digests.len(), 2);
        assert_eq!(digests[0].0, "1.0.0");
        assert_eq!(digests[1].0, "1.1.0");
        assert_ne!(digests[0].1, digests[1].1);
        let a = &versions_in(&registry, &source.url).unwrap().packs["demo"].versions[0];
        assert_eq!(a.id(), digests[0].1.replace(':', "-"));
        // what is on disk is what was returned
        let on_disk = versions_in(&registry, &source.url).unwrap();
        assert_eq!(on_disk.packs["demo"].versions.len(), 2);
    }

    #[test]
    fn versions_from_tags_and_index() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = origin(tmp.path());
        let c1 = commit(&repo, &[("demo/pack.yaml", "name: demo\n"), ("f", "1")]);
        git(&repo, &["tag", "demo-v1.0.0"]);
        let c2 = commit(&repo, &[("f", "2")]);
        git(&repo, &["tag", "demo-v1.1.0"]);
        git(&repo, &["tag", "other-v9.0.0"]);
        git(&repo, &["tag", "demo-vbad"]);
        let head = commit(&repo, &[(INDEX_FILE, &index("1.2.0"))]);
        let registry = tmp.path().join("registry");
        let v = sync_source_in(&registry, &file_url(&repo)).unwrap();
        assert_eq!(v.packs.keys().collect::<Vec<_>>(), ["demo"]);
        let got: Vec<_> = v.packs["demo"]
            .versions
            .iter()
            .map(|a| (a.version.as_str(), a.tag.as_deref(), a.commit.as_deref()))
            .collect();
        assert_eq!(
            got,
            [
                ("1.0.0", Some("demo-v1.0.0"), Some(c1.as_str())),
                ("1.1.0", Some("demo-v1.1.0"), Some(c2.as_str())),
                ("1.2.0", None, Some(head.as_str())),
            ]
        );
    }

    #[test]
    fn a_shallow_clone_gets_its_history() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = origin(tmp.path());
        let first = commit(&repo, &[("demo/pack.yaml", "name: demo\n"), ("f", "1")]);
        git(&repo, &["tag", "demo-v1.0.0"]);
        // No tag points here: only the history, not the tags, brings it in.
        let untagged = commit(&repo, &[("f", "2")]);
        commit(&repo, &[("f", "3"), (INDEX_FILE, &index("1.1.0"))]);
        git(&repo, &["tag", "demo-v1.1.0"]);
        let registry = tmp.path().join("registry");
        let source = file_url(&repo);
        let clone = registry.join("starters").join(cache_key(&source.url));
        fs::create_dir_all(clone.parent().unwrap()).unwrap();
        git_output(&[
            "clone",
            "--depth",
            "1",
            "-q",
            &source.url,
            clone.to_str().unwrap(),
        ])
        .unwrap();
        assert!(
            clone.join(".git/shallow").exists(),
            "the fixture is shallow"
        );
        sync_source_in(&registry, &source).unwrap();
        git(&clone, &["cat-file", "-e", &untagged]);
        git(&clone, &["cat-file", "-e", &first]);
    }

    /// Two tagged versions of `demo`, `a.md` holding `A1` then `A2`.
    fn two_versions(tmp: &Path) -> (RegistrySource, PathBuf, Versions) {
        let repo = origin(tmp);
        commit(
            &repo,
            &[
                ("demo/pack.yaml", "name: demo\n"),
                ("demo/a.md", "A1"),
                (INDEX_FILE, &index("1.0.0")),
            ],
        );
        git(&repo, &["tag", "demo-v1.0.0"]);
        commit(&repo, &[("demo/a.md", "A2"), (INDEX_FILE, &index("1.1.0"))]);
        git(&repo, &["tag", "demo-v1.1.0"]);
        let registry = tmp.join("registry");
        let source = file_url(&repo);
        let v = sync_source_in(&registry, &source).unwrap();
        (source, registry, v)
    }

    #[test]
    fn two_versions_live_side_by_side() {
        let tmp = tempfile::tempdir().unwrap();
        let (source, registry, v) = two_versions(tmp.path());
        let got: Vec<_> = v.packs["demo"]
            .versions
            .iter()
            .map(|a| {
                let dir = ensure_snapshot_in(&registry, &source, "demo", "demo", a).unwrap();
                assert_eq!(
                    dir,
                    snapshot_dir_in(&registry, &source.url, "demo", &a.id())
                );
                fs::read_to_string(dir.join("a.md")).unwrap()
            })
            .collect();
        assert_eq!(got, ["A1", "A2"]);
    }

    #[test]
    fn a_snapshot_is_never_rewritten() {
        let tmp = tempfile::tempdir().unwrap();
        let (source, registry, v) = two_versions(tmp.path());
        let a = &v.packs["demo"].versions[0];
        let dir = ensure_snapshot_in(&registry, &source, "demo", "demo", a).unwrap();
        let file = dir.join("a.md");
        let mut perm = fs::metadata(&file).unwrap().permissions();
        assert!(perm.readonly(), "a snapshot file is read-only");
        #[allow(clippy::permissions_set_readonly_false)]
        perm.set_readonly(false);
        fs::set_permissions(&file, perm).unwrap();
        fs::write(&file, "edited").unwrap();
        ensure_snapshot_in(&registry, &source, "demo", "demo", a).unwrap();
        assert_eq!(fs::read_to_string(file).unwrap(), "edited");
    }

    #[cfg(unix)]
    #[test]
    fn an_interrupted_snapshot_is_never_visible() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = origin(tmp.path());
        std::os::unix::fs::symlink("/etc/hosts", repo.join("demo/out")).unwrap();
        let commit = commit(&repo, &[("demo/a.md", "A1"), (INDEX_FILE, &index("1.0.0"))]);
        let registry = tmp.path().join("registry");
        let source = file_url(&repo);
        sync_source_in(&registry, &source).unwrap();
        let a = Available {
            version: "1.0.0".into(),
            tag: None,
            commit: Some(commit),
            digest: None,
        };
        let err = ensure_snapshot_in(&registry, &source, "demo", "demo", &a).unwrap_err();
        assert!(err.to_string().contains("points outside"), "{err}");
        let left: Vec<_> = fs::read_dir(store_dir_in(&registry, &source.url).join("demo"))
            .unwrap()
            .collect();
        assert!(left.is_empty(), "nothing left behind: {left:?}");
    }

    #[test]
    fn an_archive_snapshot_checks_its_digest() {
        let tmp = tempfile::tempdir().unwrap();
        let registry = tmp.path().join("registry");
        let source = RegistrySource {
            url: "https://example.com/packs.tar.gz".into(),
            kind: None,
        };
        let sync = |body: &str, version: &str| {
            let dir = tmp.path().join(version);
            fs::create_dir_all(dir.join("demo")).unwrap();
            fs::write(dir.join("demo/pack.yaml"), body).unwrap();
            fs::write(dir.join(INDEX_FILE), index(version)).unwrap();
            sync_source_with(&registry, &source, &Local(dir)).unwrap()
        };
        let v1 = sync("name: demo\n", "1.0.0");
        sync("name: demo\nx: 1\n", "1.1.0");
        // the first version outlived the replacement of the cache
        let first = &v1.packs["demo"].versions[0];
        let dir = snapshot_dir_in(&registry, &source.url, "demo", &first.id());
        assert_eq!(
            fs::read_to_string(dir.join("pack.yaml")).unwrap(),
            "name: demo\n"
        );
        // a digest the cache does not hold: refused, no snapshot
        let wrong = Available {
            digest: Some(format!("sha256:{}", "0".repeat(64))),
            ..first.clone()
        };
        let err = ensure_snapshot_in(&registry, &source, "demo", "demo", &wrong).unwrap_err();
        let found = tree_digest(
            &registry
                .join("starters")
                .join(cache_key(&source.url))
                .join("demo"),
        )
        .unwrap();
        assert_eq!(
            err.to_string(),
            format!(
                "the cache no longer holds pack demo 1.0.0 (digest {found}): run `armadai pack sync`"
            )
        );
        assert!(!snapshot_dir_in(&registry, &source.url, "demo", &wrong.id()).exists());
    }

    fn demo_versions(v: &Versions) -> Vec<&str> {
        v.packs["demo"]
            .versions
            .iter()
            .map(|a| a.version.as_str())
            .collect()
    }

    #[test]
    fn an_index_pack_without_version_still_has_its_tags() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = origin(tmp.path());
        commit(
            &repo,
            &[
                ("demo/pack.yaml", "name: demo\n"),
                (
                    INDEX_FILE,
                    "format: 1\npacks:\n  - name: demo\n    path: demo\n",
                ),
            ],
        );
        git(&repo, &["tag", "demo-v1.0.0"]);
        let v = sync_source_in(&tmp.path().join("registry"), &file_url(&repo)).unwrap();
        assert_eq!(demo_versions(&v), ["1.0.0"]);
    }

    #[test]
    fn a_pack_of_a_source_without_index_has_no_version() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = origin(tmp.path());
        commit(&repo, &[("demo/pack.yaml", "name: demo\n")]);
        git(&repo, &["tag", "demo-v1.0.0"]);
        let v = sync_source_in(&tmp.path().join("registry"), &file_url(&repo)).unwrap();
        assert_eq!(demo_versions(&v), Vec::<&str>::new());
    }

    #[test]
    fn an_index_entry_must_stay_inside_the_source_and_hold_a_pack() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = origin(tmp.path());
        // `../outside` resolves, from the clone, to a real pack next to it.
        let registry = tmp.path().join("registry");
        fs::create_dir_all(registry.join("starters/outside")).unwrap();
        fs::write(
            registry.join("starters/outside/pack.yaml"),
            "name: outside\n",
        )
        .unwrap();
        let index = "format: 1\npacks:\n  - { name: demo, path: demo, version: 1.0.0 }\n  \
                     - { name: outside, path: ../outside, version: 1.0.0 }\n  \
                     - { name: ghost, path: ghost, version: 1.0.0 }\n";
        commit(
            &repo,
            &[("demo/pack.yaml", "name: demo\n"), (INDEX_FILE, index)],
        );
        let v = sync_source_in(&registry, &file_url(&repo)).unwrap();
        assert_eq!(v.packs.keys().collect::<Vec<_>>(), ["demo"]);
    }

    fn commit_id() -> String {
        "a".repeat(40)
    }

    #[test]
    fn a_name_or_id_that_is_not_plain_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let registry = tmp.path().join("registry");
        let source = RegistrySource {
            url: "https://example.com/p.git".into(),
            kind: Some(SourceKind::Git),
        };
        let with = |commit: String| Available {
            version: "1.0.0".into(),
            tag: None,
            commit: Some(commit),
            digest: None,
        };
        let refused = |pack: &str, a: &Available, shown: &str| {
            let err = ensure_snapshot_in(&registry, &source, pack, "demo", a).unwrap_err();
            assert_eq!(
                err.to_string(),
                format!("pack {pack} has an invalid name or version id: {shown}")
            );
        };
        for name in ["../escaped", "a/b", "a\\b", "-x", "..", "/abs"] {
            refused(name, &with(commit_id()), name);
        }
        for id in [
            "../../../../victim",
            "--remote=/nonexistent",
            &"A".repeat(40),
            "abc",
        ] {
            refused("demo", &with(id.to_string()), id);
        }
        // an id equal to the pack name is still an id, not a name
        refused("master", &with("master".into()), "master");
        refused("a:b", &with(commit_id()), "a:b");
        assert!(!registry.exists(), "nothing was created");
    }

    #[test]
    fn a_remote_pack_name_cannot_leave_the_store() {
        let tmp = tempfile::tempdir().unwrap();
        let registry = tmp.path().join("registry");
        let source = RegistrySource {
            url: "https://example.com/packs.tar.gz".into(),
            kind: None,
        };
        let dir = tmp.path().join("src");
        fs::create_dir_all(dir.join("demo")).unwrap();
        fs::write(dir.join("demo/pack.yaml"), "name: demo\n").unwrap();
        fs::write(
            dir.join(INDEX_FILE),
            "format: 1\npacks:\n  - name: ../../../escaped\n    path: demo\n    version: 1.0.0\n",
        )
        .unwrap();
        let v = sync_source_with(&registry, &source, &Local(dir)).unwrap();
        assert!(v.packs.is_empty());
        assert!(!tmp.path().join("escaped").exists());
    }

    #[test]
    fn the_snapshot_errors_say_what_is_wrong() {
        let tmp = tempfile::tempdir().unwrap();
        let (source, registry, v) = two_versions(tmp.path());
        let none = Available {
            version: "1.0.0".into(),
            tag: None,
            commit: None,
            digest: None,
        };
        let say = |a: &Available| {
            ensure_snapshot_in(&registry, &source, "demo", "demo", a)
                .unwrap_err()
                .to_string()
        };
        assert_eq!(
            say(&none),
            "pack demo has a version without commit or digest"
        );
        let digest_only = Available {
            digest: Some(format!("sha256:{}", "0".repeat(64))),
            ..none
        };
        assert_eq!(say(&digest_only), "pack demo 1.0.0 has no commit");
        drop(v);
    }

    #[test]
    fn a_file_in_a_snapshot_place_is_not_a_snapshot() {
        let tmp = tempfile::tempdir().unwrap();
        let (source, registry, v) = two_versions(tmp.path());
        let a = &v.packs["demo"].versions[0];
        let dest = snapshot_dir_in(&registry, &source.url, "demo", &a.id());
        fs::create_dir_all(dest.parent().unwrap()).unwrap();
        fs::write(&dest, "not a folder").unwrap();
        // refused at the entry, and again when the rename finds the file
        assert!(ensure_snapshot_in(&registry, &source, "demo", "demo", a).is_err());
        assert!(dest.is_file());
    }

    #[test]
    fn tree_digest_sees_content_and_exec_bit() {
        let tmp = tempfile::tempdir().unwrap();
        let make = |name: &str, body: &str, exec: bool| {
            let d = tmp.path().join(name);
            fs::create_dir_all(&d).unwrap();
            let f = d.join("run.sh");
            fs::write(&f, body).unwrap();
            #[cfg(unix)]
            fs::set_permissions(
                &f,
                std::os::unix::fs::PermissionsExt::from_mode(if exec { 0o755 } else { 0o644 }),
            )
            .unwrap();
            tree_digest(&d).unwrap()
        };
        let base = make("a", "echo 1", false);
        let other_byte = make("b", "echo 2", false);
        let exec = make("c", "echo 1", true);
        assert_ne!(base, other_byte);
        assert_ne!(base, exec);
        assert_ne!(other_byte, exec);
        assert!(base.starts_with("sha256:"));
    }
}
