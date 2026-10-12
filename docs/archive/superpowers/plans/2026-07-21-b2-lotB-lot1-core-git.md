# B2 Lot B — Lot 1 (cœur + fetcher git) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`).

**Goal:** Fetcher et découvrir des starter packs depuis des registres **git** distants, derrière une abstraction agnostique du livrable, et les rendre visibles au système de starters existant. (L'archive fetcher + la CLI sont le Lot 2.)

**Architecture:** `RegistrySource` gagne un `kind` (git|archive) inféré/explicite ; `RegistryKind::Starters` + `resolved_sources`. Nouveau module `starters_registry` : cache par source, `StarterFetcher` trait + `GitFetcher` (clone/pull), découverte hybride (scan `pack.yaml` + `armadai-starters.yaml` optionnel), `sync_starters`. `core/starter.rs` inclut le cache distant dans `all_starters_dirs`/`find_pack_dir`/`load_all_packs`.

**Tech Stack:** Rust edition 2024, git (shell-out), serde.

## Global Constraints

- Base = `origin/release/1.0.0`. Branche `feat/b2-lotB-lot1-core`, PR vers `release/1.0.0`.
- **Tout est compilé sans `providers-api`** (git shell-out, pas de HTTP). Clippy 2 modes CI `-D warnings` : `--no-default-features --features tui` ET `--features tui,providers-api`. `cargo fmt -- --check`. `cargo test`.
- Réutiliser : `core::registries::{RegistrySource, RegistriesConfig, RegistryKind, resolved_sources, load_user_registries, cache_key}` ; le pattern git de `skills_registry::sync` (`clone`/`pull`) ; `core::starter::{StarterPack, all_starters_dirs, find_pack_dir, load_all_packs}`.
- Spec : `docs/superpowers/specs/2026-07-21-b2-lotB-remote-starters-design.md`.
- **Rétro-compat** : sans `registries.starters`, aucun cache distant → comportement starters inchangé.

---

### Task 1: `SourceKind` + `kind` sur `RegistrySource` + `RegistryKind::Starters`

**Files:** Modify `src/core/registries.rs`.

**Interfaces produces:**
- `pub enum SourceKind { Git, Archive }` (Deserialize/Serialize, `#[serde(rename_all="lowercase")]`, Debug/Clone/Copy/PartialEq/Eq).
- `RegistrySource` gagne `pub kind: Option<SourceKind>` (`#[serde(default)]`).
- `pub fn RegistrySource::resolved_kind(&self) -> SourceKind` — `self.kind` sinon inférence depuis `url`.
- `RegistryKind::Starters` variant + arm dans `sources()`.

- [ ] **Step 1: Write the failing tests** (module `#[cfg(test)]` de `registries.rs`)

```rust
#[test]
fn test_source_kind_deserialize_and_infer() {
    let yaml = r#"
starters:
  - url: "https://github.com/me/starters.git"
  - url: "https://x.com/p.tar.gz"
  - url: "https://interne/p"
    kind: archive
"#;
    let c: RegistriesConfig = serde_yaml_ng::from_str(yaml).unwrap();
    assert_eq!(c.starters[0].resolved_kind(), SourceKind::Git);      // .git → git
    assert_eq!(c.starters[1].resolved_kind(), SourceKind::Archive);  // .tar.gz → archive
    assert_eq!(c.starters[2].resolved_kind(), SourceKind::Archive);  // explicit override
    assert_eq!(c.starters[2].kind, Some(SourceKind::Archive));
}

#[test]
fn test_infer_defaults_to_git() {
    let s = RegistrySource { url: "https://host/repo".to_string(), kind: None };
    assert_eq!(s.resolved_kind(), SourceKind::Git);
}

#[test]
fn test_resolved_sources_starters_union() {
    let user = RegistriesConfig {
        starters: vec![RegistrySource { url: "u".into(), kind: None }],
        ..Default::default()
    };
    let proj = RegistriesConfig {
        starters: vec![RegistrySource { url: "p".into(), kind: None }],
        ..Default::default()
    };
    let out = resolved_sources(RegistryKind::Starters, &[], &user, Some(&proj));
    assert_eq!(out, vec!["u".to_string(), "p".to_string()]);
}
```

- [ ] **Step 2: Run to verify fail**

Run: `cargo test --no-default-features --features tui -p armadai registries`
Expected: FAIL (`SourceKind`, `kind`, `resolved_kind`, `RegistryKind::Starters` undefined).

- [ ] **Step 3: Add `SourceKind` + `kind` field + inference**

Dans `registries.rs`, après les imports :

```rust
/// Delivery kind of a registry source (transport-agnostic fetch).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceKind {
    Git,
    Archive,
}
```

`RegistrySource` :

```rust
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct RegistrySource {
    pub url: String,
    /// Delivery kind. Absent = inferred from the URL (see `resolved_kind`).
    #[serde(default)]
    pub kind: Option<SourceKind>,
}

impl RegistrySource {
    /// The effective delivery kind: explicit `kind`, else inferred from the URL.
    pub fn resolved_kind(&self) -> SourceKind {
        if let Some(k) = self.kind {
            return k;
        }
        let u = self.url.to_lowercase();
        if u.ends_with(".tar.gz") || u.ends_with(".tgz") || u.ends_with(".zip") {
            SourceKind::Archive
        } else {
            SourceKind::Git
        }
    }
}
```

**Fix existing `RegistrySource { .. }` literals** (they now miss `kind`): `rg -n "RegistrySource \{" src/` and add `kind: None` to each (notably `registry/sync.rs` / `skills_registry` / `model_registry` if any construct it, and CLI). Compile to verify.

- [ ] **Step 4: Add `RegistryKind::Starters`**

Enum `RegistryKind` : add `Starters` variant. `sources()` match : add `RegistryKind::Starters => &config.starters,`. Update the enum's doc comment (remove the "starters has no variant yet" note).

- [ ] **Step 5: Run tests + clippy 2 modes + fmt**

Run: `cargo test --no-default-features --features tui -p armadai registries`
Expected: PASS.
Run: `cargo clippy --all-targets --no-default-features --features tui -- -D warnings && cargo clippy --all-targets --no-default-features --features tui,providers-api -- -D warnings && cargo fmt -- --check`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add src/core/registries.rs
git commit -m "feat(registries): SourceKind + RegistrySource.kind + RegistryKind::Starters"
```

---

### Task 2: Module `starters_registry` (cache + GitFetcher + découverte hybride + sync)

**Files:** Create `src/starters_registry/mod.rs`; modify `src/main.rs` (`mod starters_registry;`).

**Interfaces:**
- Consumes (Task 1): `SourceKind`, `RegistrySource`, `cache_key`.
- Produces:
  - `pub fn starters_cache_dir() -> PathBuf` (racine du cache, mirror de `skills_registry::sync::repos_dir` — sous `crate::core::config::registry_cache_dir()`).
  - `pub fn source_cache_dir(url: &str) -> PathBuf` (= `starters_cache_dir().join(cache_key(url))`).
  - `pub trait StarterFetcher { fn fetch(&self, url: &str, dest: &Path) -> anyhow::Result<()>; }` + `pub struct GitFetcher;` (clone si `dest/.git` absent, sinon pull — mirror `skills_registry` clone/pull).
  - `pub fn fetch_starter_source(source: &RegistrySource) -> anyhow::Result<PathBuf>` — dispatch selon `resolved_kind()`: `Git` → `GitFetcher` ; `Archive` → renvoie une erreur « archive fetch requires providers-api (Lot 2) » (Lot 1 = git seulement).
  - `pub fn sync_starters(sources: &[RegistrySource]) -> Vec<PathBuf>` — fetch chaque source (warn+continue sur erreur), renvoie les dirs OK.
  - `pub fn discover_packs(registry_dir: &Path) -> Vec<PathBuf>` — **découverte hybride** : si `registry_dir/armadai-starters.yaml` existe, ne renvoyer que les `path:` listés (résolus + valides) ; sinon scanner récursivement (profondeur raisonnable) les dossiers contenant un `pack.yaml`.

- [ ] **Step 1: Write the failing tests** (dans `starters_registry/mod.rs` `#[cfg(test)]`)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(p: &Path, s: &str) { fs::create_dir_all(p.parent().unwrap()).unwrap(); fs::write(p, s).unwrap(); }

    #[test]
    fn discover_by_convention_scans_pack_yaml() {
        let tmp = std::env::temp_dir().join(format!("armadai-st-conv-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        write(&tmp.join("rust-qa/pack.yaml"), "name: rust-qa\n");
        write(&tmp.join("nested/web/pack.yaml"), "name: web\n");
        write(&tmp.join("not-a-pack/readme.md"), "x");
        let packs = discover_packs(&tmp);
        let names: Vec<_> = packs.iter().filter_map(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).collect();
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
        write(&tmp.join("armadai-starters.yaml"), "packs:\n  - path: rust-qa\n");
        let packs = discover_packs(&tmp);
        assert_eq!(packs.len(), 1);
        assert_eq!(packs[0].file_name().unwrap().to_string_lossy(), "rust-qa");
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn fetch_archive_kind_errors_in_lot1() {
        let src = RegistrySource { url: "https://x/p.tar.gz".to_string(), kind: None };
        assert!(fetch_starter_source(&src).is_err());
    }

    #[tokio::test]
    async fn git_fetcher_clones_local_repo() {
        // Build a local git repo containing a pack, fetch it, discover the pack.
        // (Skip gracefully if `git` is unavailable.)
        if std::process::Command::new("git").arg("--version").output().is_err() { return; }
        let tmp = std::env::temp_dir().join(format!("armadai-st-git-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        let repo = tmp.join("origin");
        write(&repo.join("demo/pack.yaml"), "name: demo\n");
        for args in [vec!["init","-q"], vec!["add","-A"], vec!["-c","user.email=t@t","-c","user.name=t","commit","-qm","x"]] {
            std::process::Command::new("git").current_dir(&repo).args(&args).output().unwrap();
        }
        let dest = tmp.join("dest");
        GitFetcher.fetch(repo.to_str().unwrap(), &dest).unwrap();
        let packs = discover_packs(&dest);
        assert!(packs.iter().any(|p| p.file_name().unwrap().to_string_lossy() == "demo"));
        let _ = fs::remove_dir_all(&tmp);
    }
}
```

- [ ] **Step 2: Run to verify fail**

Run: `cargo test --no-default-features --features tui -p armadai starters_registry`
Expected: FAIL (module absent).

- [ ] **Step 3: Implement `starters_registry/mod.rs`**

```rust
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
            run_git(&["-C", dest.to_str().unwrap_or("."), "pull", "--ff-only", "-q"])?;
        } else {
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            run_git(&["clone", "--depth", "1", "-q", url, dest.to_str().unwrap_or(".")])?;
        }
        Ok(())
    }
}

fn run_git(args: &[&str]) -> anyhow::Result<()> {
    let out = std::process::Command::new("git").args(args).output()?;
    if !out.status.success() {
        anyhow::bail!("git {:?} failed: {}", args, String::from_utf8_lossy(&out.stderr));
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
```

Register in `src/main.rs`: `mod starters_registry;` (near `mod skills_registry;`). If `skills_registry` is gated, match its gating; otherwise leave ungated.

- [ ] **Step 4: Run tests + clippy 2 modes + fmt**

Run: `cargo test --no-default-features --features tui -p armadai starters_registry`
Expected: PASS (4 tests; the git one skips if `git` missing).
Run: `cargo clippy --all-targets --no-default-features --features tui -- -D warnings && cargo clippy --all-targets --no-default-features --features tui,providers-api -- -D warnings && cargo fmt -- --check`
Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add src/starters_registry/mod.rs src/main.rs
git commit -m "feat(starters): remote starter registry — GitFetcher + hybrid discovery + cache"
```

---

### Task 3: Intégrer le cache distant au système starters

**Files:** Modify `src/core/starter.rs`.

**Interfaces consumes (Task 2):** `starters_registry::{starters_cache_dir, discover_packs}`.

- [ ] **Step 1: Write the failing test** (dans `starter.rs` `#[cfg(test)]`)

```rust
#[test]
fn remote_cache_packs_are_discovered() {
    // A pack under the starters cache dir should be found by find_pack_dir.
    // Isolate the cache via a temp registry_cache_dir if overridable; else
    // assert discover_packs surfaces the cache packs through
    // remote_starter_pack_dirs().
    use std::fs;
    let tmp = std::env::temp_dir().join(format!("armadai-remote-{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp);
    let pack = tmp.join("src1/remote-demo/pack.yaml");
    fs::create_dir_all(pack.parent().unwrap()).unwrap();
    fs::write(&pack, "name: remote-demo\n").unwrap();
    // remote_starter_pack_dirs scans a given cache root for packs:
    let dirs = remote_starter_pack_dirs_in(&tmp);
    assert!(dirs.iter().any(|d| d.file_name().unwrap().to_string_lossy() == "remote-demo"));
    let _ = fs::remove_dir_all(&tmp);
}
```

- [ ] **Step 2: Run to verify fail**

Run: `cargo test --no-default-features --features tui -p armadai remote_cache_packs`
Expected: FAIL (`remote_starter_pack_dirs_in` undefined).

- [ ] **Step 3: Add remote-cache discovery + wire into find_pack_dir/load_all_packs**

Dans `starter.rs`, ajouter :

```rust
/// Pack directories discovered under the remote starters cache (each
/// synced source is a subdir; packs are found by `discover_packs`).
pub fn remote_starter_pack_dirs() -> Vec<PathBuf> {
    remote_starter_pack_dirs_in(&crate::starters_registry::starters_cache_dir())
}

/// Testable core: scan a cache root's per-source subdirs for packs.
fn remote_starter_pack_dirs_in(cache_root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(sources) = std::fs::read_dir(cache_root) else {
        return out;
    };
    for src in sources.flatten() {
        if src.path().is_dir() {
            out.extend(crate::starters_registry::discover_packs(&src.path()));
        }
    }
    out
}
```

- `find_pack_dir(name)` : après la boucle sur `all_starters_dirs()`, si `found` est None, chercher un pack distant dont le dossier s'appelle `name` (priorité local > distant : ne remplace pas un `found` local) :

```rust
pub fn find_pack_dir(name: &str) -> Option<PathBuf> {
    let mut found: Option<PathBuf> = None;
    for dir in all_starters_dirs() {
        let candidate = dir.join(name);
        if candidate.is_dir() && candidate.join("pack.yaml").is_file() {
            found = Some(candidate);
        }
    }
    if found.is_none() {
        found = remote_starter_pack_dirs()
            .into_iter()
            .find(|d| d.file_name().is_some_and(|n| n == name));
    }
    found
}
```

- `load_all_packs()` : après avoir chargé les packs des `all_starters_dirs()`, charger aussi les packs distants (`remote_starter_pack_dirs()` → `StarterPack::load(dir)`), en n'écrasant PAS un pack local du même nom (le `HashMap` existant garde le premier / gère la dédup — insérer les distants seulement si le nom n'est pas déjà présent). Suivre la logique de dédup existante de `load_all_packs`.

- [ ] **Step 4: Run tests + clippy 2 modes + fmt**

Run: `cargo test --no-default-features --features tui -p armadai starter`
Expected: PASS (nouveau test + existants).
Run: `cargo clippy --all-targets --no-default-features --features tui -- -D warnings && cargo clippy --all-targets --no-default-features --features tui,providers-api -- -D warnings && cargo fmt -- --check`
Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add src/core/starter.rs
git commit -m "feat(starters): include remote registry cache in pack discovery (local wins)"
```

---

## Notes pour l'implémenteur

- **Lot 1 = git only.** L'archive fetcher (providers-api), la CLI (`registry sources ... starters`, `registry sync`) et l'auto-on-miss d'`init --pack` sont le **Lot 2** — ne pas les faire ici.
- `starters_cache_dir` doit être cohérent avec l'emplacement de cache existant (`registry_cache_dir()` / `skills_registry::repos_dir` base) — vérifier `repos_dir()` et mirrorer la base.
- Priorité **local > distant** partout (find_pack_dir + load_all_packs dédup).
- `discover_packs` : un dossier avec `pack.yaml` est un pack et n'est pas re-scanné en profondeur (évite les packs imbriqués fantômes). Cap profondeur 4.
- Le test `git_fetcher_clones_local_repo` skip proprement si `git` absent (CI a git).
- Vérifier le gating de `mod skills_registry` dans main.rs et aligner `mod starters_registry`.
