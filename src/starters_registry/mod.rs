//! Remote starter registries (B2 Lot B): fetch starter packs from git (Lot 1)
//! or archive (Lot 2) sources into a cache, discovered by the existing
//! `StarterPack` convention (+ an optional `armadai-starters.yaml` manifest).

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::core::registries::{RegistrySource, SourceKind, cache_key};

/// Root cache dir for synced starter registries.
pub fn starters_cache_dir() -> PathBuf {
    crate::core::config::registry_cache_dir().join("starters")
}

/// Cache dir for one source URL.
pub fn source_cache_dir(url: &str) -> PathBuf {
    starters_cache_dir().join(cache_key(url))
}

/// Optional registry manifest that enriches/restricts discovery.
#[derive(Debug, Deserialize, Default)]
struct StartersManifest {
    #[serde(default)]
    packs: Vec<ManifestPack>,
}
#[derive(Debug, Deserialize)]
struct ManifestPack {
    path: String,
}

pub trait StarterFetcher {
    fn fetch(&self, url: &str, dest: &Path) -> anyhow::Result<()>;
}

/// Git-backed fetcher: clone if absent, else pull. Shells out to `git`.
pub struct GitFetcher;

impl StarterFetcher for GitFetcher {
    fn fetch(&self, url: &str, dest: &Path) -> anyhow::Result<()> {
        if dest.join(".git").is_dir() {
            run_git(&[
                "-C",
                dest.to_str().unwrap_or("."),
                "pull",
                "--ff-only",
                "-q",
            ])?;
        } else {
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            run_git(&[
                "clone",
                "--depth",
                "1",
                "-q",
                url,
                dest.to_str().unwrap_or("."),
            ])?;
        }
        Ok(())
    }
}

fn run_git(args: &[&str]) -> anyhow::Result<()> {
    let out = std::process::Command::new("git").args(args).output()?;
    if !out.status.success() {
        anyhow::bail!(
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(())
}

/// Fetch one source into its cache dir. Git only in Lot 1; archive is Lot 2.
pub fn fetch_starter_source(source: &RegistrySource) -> anyhow::Result<PathBuf> {
    let dest = source_cache_dir(&source.url);
    match source.resolved_kind() {
        SourceKind::Git => {
            GitFetcher.fetch(&source.url, &dest)?;
            Ok(dest)
        }
        SourceKind::Archive => {
            anyhow::bail!(
                "archive starter source '{}' requires the `providers-api` feature (B2 Lot B Lot 2)",
                source.url
            )
        }
    }
}

/// Sync all sources; warn+continue on failure. Returns the dirs that synced OK.
pub fn sync_starters(sources: &[RegistrySource]) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for s in sources {
        match fetch_starter_source(s) {
            Ok(d) => dirs.push(d),
            Err(e) => eprintln!("  warn: failed to sync starter source {}: {e}", s.url),
        }
    }
    dirs
}

/// Discover pack directories under a fetched registry dir.
///
/// Hybrid: an `armadai-starters.yaml` at the root RESTRICTS to its listed
/// `path:`s; otherwise every directory containing a `pack.yaml` is a pack.
pub fn discover_packs(registry_dir: &Path) -> Vec<PathBuf> {
    let manifest_path = registry_dir.join("armadai-starters.yaml");
    if manifest_path.is_file()
        && let Ok(content) = std::fs::read_to_string(&manifest_path)
        && let Ok(manifest) = serde_yaml_ng::from_str::<StartersManifest>(&content)
        && !manifest.packs.is_empty()
    {
        return manifest
            .packs
            .iter()
            .map(|p| registry_dir.join(&p.path))
            .filter(|d| d.join("pack.yaml").is_file())
            .collect();
    }
    let mut out = Vec::new();
    scan_for_packs(registry_dir, 0, &mut out);
    out
}

fn scan_for_packs(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > 4 {
        return;
    }
    if dir.join("pack.yaml").is_file() {
        out.push(dir.to_path_buf());
        return; // a pack dir isn't itself scanned deeper
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() && !p.file_name().is_some_and(|n| n == ".git") {
            scan_for_packs(&p, depth + 1, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(p: &Path, s: &str) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, s).unwrap();
    }

    #[test]
    fn discover_by_convention_scans_pack_yaml() {
        let tmp = std::env::temp_dir().join(format!("armadai-st-conv-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        write(&tmp.join("rust-qa/pack.yaml"), "name: rust-qa\n");
        write(&tmp.join("nested/web/pack.yaml"), "name: web\n");
        write(&tmp.join("not-a-pack/readme.md"), "x");
        let packs = discover_packs(&tmp);
        let names: Vec<_> = packs
            .iter()
            .filter_map(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .collect();
        assert!(names.contains(&"rust-qa".to_string()));
        assert!(names.contains(&"web".to_string()));
        assert!(!names.iter().any(|n| n == "not-a-pack"));
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn discover_with_manifest_restricts_to_listed() {
        let tmp = std::env::temp_dir().join(format!("armadai-st-man-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        write(&tmp.join("rust-qa/pack.yaml"), "name: rust-qa\n");
        write(&tmp.join("hidden/pack.yaml"), "name: hidden\n");
        write(
            &tmp.join("armadai-starters.yaml"),
            "packs:\n  - path: rust-qa\n",
        );
        let packs = discover_packs(&tmp);
        assert_eq!(packs.len(), 1);
        assert_eq!(packs[0].file_name().unwrap().to_string_lossy(), "rust-qa");
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn fetch_archive_kind_errors_in_lot1() {
        let src = RegistrySource {
            url: "https://x/p.tar.gz".to_string(),
            kind: None,
        };
        assert!(fetch_starter_source(&src).is_err());
    }

    #[tokio::test]
    async fn git_fetcher_clones_local_repo() {
        // Build a local git repo containing a pack, fetch it, discover the pack.
        // (Skip gracefully if `git` is unavailable.)
        if std::process::Command::new("git")
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }
        let tmp = std::env::temp_dir().join(format!("armadai-st-git-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        let repo = tmp.join("origin");
        write(&repo.join("demo/pack.yaml"), "name: demo\n");
        for args in [
            vec!["init", "-q"],
            vec!["add", "-A"],
            vec![
                "-c",
                "user.email=t@t",
                "-c",
                "user.name=t",
                "commit",
                "-qm",
                "x",
            ],
        ] {
            std::process::Command::new("git")
                .current_dir(&repo)
                .args(&args)
                .output()
                .unwrap();
        }
        let dest = tmp.join("dest");
        GitFetcher.fetch(repo.to_str().unwrap(), &dest).unwrap();
        let packs = discover_packs(&dest);
        assert!(
            packs
                .iter()
                .any(|p| p.file_name().unwrap().to_string_lossy() == "demo")
        );
        let _ = fs::remove_dir_all(&tmp);
    }
}
