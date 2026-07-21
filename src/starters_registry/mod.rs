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

// Not yet wired outside tests: real callers land in Lot 2 (`registry sync`
// CLI + archive fetcher).
#[allow(dead_code)]
pub trait StarterFetcher {
    fn fetch(&self, url: &str, dest: &Path) -> anyhow::Result<()>;
}

/// Git-backed fetcher: clone if absent, else pull. Shells out to `git`.
#[allow(dead_code)]
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

#[allow(dead_code)]
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

/// Extract a downloaded archive (.tar.gz/.tgz or .zip) into `dest` by shelling
/// out to system `tar`/`unzip` (no extra crate). `dest` is created.
// Only called by `ArchiveFetcher`, which is gated `providers-api`; without
// that feature this helper is unused (still exercised directly by tests).
#[cfg_attr(not(feature = "providers-api"), allow(dead_code))]
fn extract_archive(archive: &Path, dest: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dest)?;
    let name = archive.to_string_lossy().to_lowercase();
    let status = if name.ends_with(".zip") {
        std::process::Command::new("unzip")
            .args([
                "-q",
                "-o",
                archive.to_str().unwrap_or(""),
                "-d",
                dest.to_str().unwrap_or(""),
            ])
            .status()?
    } else {
        // .tar.gz / .tgz
        std::process::Command::new("tar")
            .args([
                "-xzf",
                archive.to_str().unwrap_or(""),
                "-C",
                dest.to_str().unwrap_or(""),
            ])
            .status()?
    };
    if !status.success() {
        anyhow::bail!("failed to extract archive {}", archive.display());
    }
    Ok(())
}

/// Archive-backed fetcher: download via reqwest, then extract via `tar`/`unzip`.
#[cfg(feature = "providers-api")]
pub struct ArchiveFetcher;

#[cfg(feature = "providers-api")]
impl StarterFetcher for ArchiveFetcher {
    fn fetch(&self, url: &str, dest: &Path) -> anyhow::Result<()> {
        // Download to a temp file, then extract into `dest`.
        let bytes = download_bytes(url)?;
        let tmp = std::env::temp_dir().join(format!("armadai-dl-{}", cache_key(url)));
        std::fs::write(&tmp, &bytes)?;
        // Preserve the extension so extract_archive picks tar vs unzip.
        let ext_tmp = tmp.with_extension(archive_ext(url));
        std::fs::rename(&tmp, &ext_tmp).ok();
        let src = if ext_tmp.exists() { ext_tmp } else { tmp };
        let _ = std::fs::remove_dir_all(dest); // fresh extract
        extract_archive(&src, dest)?;
        let _ = std::fs::remove_file(&src);
        Ok(())
    }
}

#[cfg(feature = "providers-api")]
fn archive_ext(url: &str) -> &'static str {
    let u = url.to_lowercase();
    if u.ends_with(".zip") {
        "zip"
    } else if u.ends_with(".tgz") {
        "tgz"
    } else {
        "tar.gz"
    }
}

#[cfg(feature = "providers-api")]
fn download_bytes(url: &str) -> anyhow::Result<Vec<u8>> {
    // `fetch` is a sync trait method but is called from WITHIN an async runtime
    // (`registry sync`, `init`). Creating a tokio runtime inline would panic
    // ("cannot start a runtime from within a runtime"). Run the async reqwest
    // download on a dedicated OS thread with its own current-thread runtime.
    let url = url.to_string();
    std::thread::spawn(move || -> anyhow::Result<Vec<u8>> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        rt.block_on(async {
            let resp = reqwest::get(&url).await?;
            if !resp.status().is_success() {
                anyhow::bail!("download {url} failed: HTTP {}", resp.status());
            }
            Ok(resp.bytes().await?.to_vec())
        })
    })
    .join()
    .map_err(|_| anyhow::anyhow!("download thread panicked"))?
}

/// Fetch one source into its cache dir. Git only in Lot 1; archive is Lot 2.
// Not yet wired outside tests: real caller is `sync_starters`, itself a
// Lot 2 (CLI `registry sync`) entry point.
#[allow(dead_code)]
pub fn fetch_starter_source(source: &RegistrySource) -> anyhow::Result<PathBuf> {
    let dest = source_cache_dir(&source.url);
    match source.resolved_kind() {
        SourceKind::Git => {
            GitFetcher.fetch(&source.url, &dest)?;
            Ok(dest)
        }
        SourceKind::Archive => {
            #[cfg(feature = "providers-api")]
            {
                ArchiveFetcher.fetch(&source.url, &dest)?;
                Ok(dest)
            }
            #[cfg(not(feature = "providers-api"))]
            {
                anyhow::bail!(
                    "archive starter source '{}' requires the `providers-api` feature (B2 Lot B Lot 2)",
                    source.url
                )
            }
        }
    }
}

/// Sync all sources; warn+continue on failure. Returns the dirs that synced OK.
// Not yet wired outside tests: real caller is the Lot 2 `registry sync` CLI
// command.
#[allow(dead_code)]
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
            .filter(|p| {
                let path = std::path::Path::new(&p.path);
                !path.components().any(|c| {
                    matches!(
                        c,
                        std::path::Component::ParentDir | std::path::Component::RootDir
                    )
                })
            })
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
    #[cfg(not(feature = "providers-api"))]
    fn fetch_archive_kind_errors_without_providers_api() {
        // Without `providers-api`, archive sources have no fetcher wired and
        // must fail fast with a clear error (no network attempted).
        let src = RegistrySource {
            url: "https://x/p.tar.gz".to_string(),
            kind: None,
        };
        assert!(fetch_starter_source(&src).is_err());
    }

    #[test]
    fn discover_rejects_parent_traversal_in_manifest() {
        let tmp = std::env::temp_dir().join(format!("armadai-trav-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(tmp.join("ok")).unwrap();
        fs::write(tmp.join("ok/pack.yaml"), "name: ok\n").unwrap();
        fs::write(
            tmp.join("armadai-starters.yaml"),
            "packs:\n  - path: ../escape\n  - path: ok\n",
        )
        .unwrap();
        let packs = discover_packs(&tmp);
        // The `../escape` entry is rejected; only `ok` survives.
        assert_eq!(packs.len(), 1);
        assert!(packs[0].ends_with("ok"));
        let _ = fs::remove_dir_all(&tmp);
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

#[cfg(all(test, feature = "providers-api"))]
mod archive_tests {
    use super::*;
    use std::fs;

    fn write(p: &std::path::Path, s: &str) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, s).unwrap();
    }

    // Extract a local .tar.gz built with system `tar` and confirm packs discovered.
    #[test]
    fn archive_fetcher_extracts_local_targz() {
        if std::process::Command::new("tar")
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }
        let tmp = std::env::temp_dir().join(format!("armadai-arc-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        // Build a source tree with a pack, tar it.
        let srcroot = tmp.join("srcroot");
        write(&srcroot.join("rust-qa/pack.yaml"), "name: rust-qa\n");
        let archive = tmp.join("packs.tar.gz");
        std::process::Command::new("tar")
            .args([
                "-czf",
                archive.to_str().unwrap(),
                "-C",
                srcroot.to_str().unwrap(),
                ".",
            ])
            .output()
            .unwrap();
        // Extract via the fetcher's extract helper (file:// or local path).
        let dest = tmp.join("dest");
        extract_archive(&archive, &dest).unwrap();
        let packs = discover_packs(&dest);
        assert!(
            packs
                .iter()
                .any(|p| p.file_name().unwrap().to_string_lossy() == "rust-qa")
        );
        let _ = fs::remove_dir_all(&tmp);
    }
}
