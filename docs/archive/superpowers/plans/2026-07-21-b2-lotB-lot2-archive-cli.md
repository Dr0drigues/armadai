# B2 Lot B — Lot 2 (archive fetcher + CLI + auto-on-miss) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`).

**Goal:** Compléter les starters distants : fetcher **archive** (download + extract), l'exposer en **CLI** (`registry sources … starters`, `registry sync`), et **auto-sync-on-miss** dans `init --pack`. + follow-ups de la revue Lot 1.

**Architecture:** `ArchiveFetcher` (gated `providers-api`) = download via reqwest + extraction par shell-out `tar`/`unzip` (zéro nouvelle dep). CLI `SourceKind` gagne `Starters` ; `registry sync` synchronise aussi les starters. `init::resolve_pack_dir` tente un sync des sources starters si le pack est introuvable, puis re-cherche.

**Tech Stack:** Rust edition 2024, reqwest (gated), git/tar/unzip (shell-out).

## Global Constraints

- Base = `origin/release/1.0.0` (@ `6d1ebc6`, après Lot 1). Branche `feat/b2-lotB-lot2-cli`, PR vers `release/1.0.0`.
- **`ArchiveFetcher` gated `#[cfg(feature = "providers-api")]`** (reqwest). Sans `providers-api`, `fetch_starter_source` sur `Archive` renvoie déjà une erreur (Lot 1) — la garder. Clippy CI 2 modes `-D warnings` (`tui` ET `tui,providers-api`) + `cargo fmt -- --check` + `cargo test`. Vérifier AUSSI `--features tui,storage,providers-api` si init/starter touchent storage.
- **Zéro nouvelle dépendance** : extraction par shell-out `tar -xzf` / `unzip` (comme `GitFetcher` shelle `git`).
- Réutiliser Lot 1 : `starters_registry::{fetch_starter_source, sync_starters, source_cache_dir, StarterFetcher}`, `core::registries::{RegistrySource, SourceKind, RegistryKind, resolved_sources, load_user_registries}`.
- Spec : `docs/superpowers/specs/2026-07-21-b2-lotB-remote-starters-design.md`.

---

### Task 1: `ArchiveFetcher` (download + extract) + follow-ups

**Files:** Modify `src/starters_registry/mod.rs`.

**Interfaces:**
- Consumes: `RegistrySource`, `SourceKind`, `StarterFetcher`, `source_cache_dir`.
- Produces: `#[cfg(feature = "providers-api")] pub struct ArchiveFetcher;` impl `StarterFetcher`. `fetch_starter_source` (archive arm) dispatch vers `ArchiveFetcher` quand `providers-api`, sinon l'erreur existante. Remove the item-level `#[allow(dead_code)]` on the items now used (fetch_starter_source/sync_starters are wired in Task 2/3; keep allow only if still trult unused in a given build).

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(all(test, feature = "providers-api"))]
mod archive_tests {
    use super::*;
    use std::fs;

    fn write(p: &std::path::Path, s: &str) { fs::create_dir_all(p.parent().unwrap()).unwrap(); fs::write(p, s).unwrap(); }

    // Extract a local .tar.gz built with system `tar` and confirm packs discovered.
    #[test]
    fn archive_fetcher_extracts_local_targz() {
        if std::process::Command::new("tar").arg("--version").output().is_err() { return; }
        let tmp = std::env::temp_dir().join(format!("armadai-arc-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        // Build a source tree with a pack, tar it.
        let srcroot = tmp.join("srcroot");
        write(&srcroot.join("rust-qa/pack.yaml"), "name: rust-qa\n");
        let archive = tmp.join("packs.tar.gz");
        std::process::Command::new("tar")
            .args(["-czf", archive.to_str().unwrap(), "-C", srcroot.to_str().unwrap(), "."])
            .output().unwrap();
        // Extract via the fetcher's extract helper (file:// or local path).
        let dest = tmp.join("dest");
        extract_archive(&archive, &dest).unwrap();
        let packs = discover_packs(&dest);
        assert!(packs.iter().any(|p| p.file_name().unwrap().to_string_lossy() == "rust-qa"));
        let _ = fs::remove_dir_all(&tmp);
    }
}

// (non-gated) manifest path traversal is rejected
#[test]
fn discover_rejects_parent_traversal_in_manifest() {
    use std::fs;
    let tmp = std::env::temp_dir().join(format!("armadai-trav-{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(tmp.join("ok")).unwrap();
    fs::write(tmp.join("ok/pack.yaml"), "name: ok\n").unwrap();
    fs::write(tmp.join("armadai-starters.yaml"), "packs:\n  - path: ../escape\n  - path: ok\n").unwrap();
    let packs = discover_packs(&tmp);
    // The `../escape` entry is rejected; only `ok` survives.
    assert_eq!(packs.len(), 1);
    assert!(packs[0].ends_with("ok"));
    let _ = fs::remove_dir_all(&tmp);
}
```

- [ ] **Step 2: Run to verify fail**

Run: `cargo test --no-default-features --features tui,providers-api -p armadai starters_registry`
Expected: FAIL (`extract_archive` undefined ; traversal not yet rejected).

- [ ] **Step 3: Reject `..` in manifest paths (follow-up)**

Dans `discover_packs`, la branche manifest : filtrer les `path:` contenant un composant `..` (ou absolus) avant de joindre :

```rust
        return manifest
            .packs
            .iter()
            .filter(|p| {
                let path = std::path::Path::new(&p.path);
                !path.components().any(|c| matches!(c, std::path::Component::ParentDir | std::path::Component::RootDir))
            })
            .map(|p| registry_dir.join(&p.path))
            .filter(|d| d.join("pack.yaml").is_file())
            .collect();
```

- [ ] **Step 4: Add `extract_archive` + `ArchiveFetcher`**

```rust
/// Extract a downloaded archive (.tar.gz/.tgz or .zip) into `dest` by shelling
/// out to system `tar`/`unzip` (no extra crate). `dest` is created.
fn extract_archive(archive: &std::path::Path, dest: &std::path::Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dest)?;
    let name = archive.to_string_lossy().to_lowercase();
    let status = if name.ends_with(".zip") {
        std::process::Command::new("unzip")
            .args(["-q", "-o", archive.to_str().unwrap_or(""), "-d", dest.to_str().unwrap_or("")])
            .status()?
    } else {
        // .tar.gz / .tgz
        std::process::Command::new("tar")
            .args(["-xzf", archive.to_str().unwrap_or(""), "-C", dest.to_str().unwrap_or("")])
            .status()?
    };
    if !status.success() {
        anyhow::bail!("failed to extract archive {}", archive.display());
    }
    Ok(())
}

#[cfg(feature = "providers-api")]
pub struct ArchiveFetcher;

#[cfg(feature = "providers-api")]
impl StarterFetcher for ArchiveFetcher {
    fn fetch(&self, url: &str, dest: &std::path::Path) -> anyhow::Result<()> {
        // Download to a temp file, then extract into `dest`.
        let bytes = download_bytes(url)?;
        let tmp = std::env::temp_dir().join(format!("armadai-dl-{}", crate::core::registries::cache_key(url)));
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
    if u.ends_with(".zip") { "zip" } else if u.ends_with(".tgz") { "tgz" } else { "tar.gz" }
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
```

Wire `fetch_starter_source` archive arm:
```rust
        SourceKind::Archive => {
            #[cfg(feature = "providers-api")]
            {
                ArchiveFetcher.fetch(&source.url, &dest)?;
                return Ok(dest);
            }
            #[cfg(not(feature = "providers-api"))]
            {
                anyhow::bail!(
                    "archive starter source '{}' requires the `providers-api` feature",
                    source.url
                )
            }
        }
```
(Check `reqwest` is the API used by `model_registry::fetch` and mirror its blocking/async pattern; if a shared runtime helper exists, reuse it instead of `Runtime::new`.)

- [ ] **Step 5: Run tests + clippy 2 modes + fmt**

Run: `cargo test --no-default-features --features tui,providers-api -p armadai starters_registry`
Expected: PASS. Also `cargo test --no-default-features --features tui -p armadai starters_registry` (archive tests cfg'd out, traversal test runs).
Run: `cargo clippy --all-targets --no-default-features --features tui -- -D warnings && cargo clippy --all-targets --no-default-features --features tui,providers-api -- -D warnings && cargo fmt -- --check`
Expected: clean.

- [ ] **Step 6: Commit** `git commit -m "feat(starters): ArchiveFetcher (download+extract via tar/unzip) + reject manifest '..'"`

---

### Task 2: CLI — `registry sources … starters` + `registry sync`

**Files:** Modify `src/cli/registry.rs`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn cli_source_kind_starters_maps_to_registry_kind() {
    assert_eq!(RegistryKind::from(SourceKind::Starters), crate::core::registries::RegistryKind::Starters);
}
```

- [ ] **Step 2: Run to verify fail** — `cargo test --no-default-features --features tui -p armadai cli_source_kind_starters` → FAIL (no `Starters`).

- [ ] **Step 3: Add `Starters` to the CLI `SourceKind`**

In `src/cli/registry.rs`, the `#[derive(ValueEnum)] enum SourceKind` → add `Starters`. `From<SourceKind> for RegistryKind` → add `SourceKind::Starters => RegistryKind::Starters`. This makes `armadai registry sources add|remove|list starters <url>` work through the existing `sources_add/remove/list` (they already take a `SourceKind`/`RegistryKind`).

- [ ] **Step 4: Extend `registry sync` to sync starters**

`RegistryAction::Sync` currently syncs the agent community registry only. Extend it to ALSO sync starter sources:
- In `cmd_sync()`, after the agent sync, resolve starter sources: `let starters = crate::core::registries::resolved_sources(RegistryKind::Starters, &[], &load_user_registries(), project_registries_opt);` — build `RegistrySource`s from these URLs (or resolve to `Vec<RegistrySource>` — since `sync_starters` takes `&[RegistrySource]`, either add a `resolved_sources_typed` returning `Vec<RegistrySource>` for starters, OR reconstruct `RegistrySource { url, kind: None }` from the URLs so `resolved_kind()` infers). Reconstructing from URLs loses an explicit `kind` — acceptable for v1 (inference covers .git/.tar.gz), OR pass the typed sources directly by reading `load_user_registries().starters` + project. Prefer: gather typed sources (user.starters ∪ project.starters, dedup by url) and call `starters_registry::sync_starters(&sources)`.
- Print a summary (`Syncing N starter source(s)…`). Keep the existing agent-registry sync behavior unchanged.

- [ ] **Step 5: Run tests + clippy 2 modes + fmt + commit**

Run: `cargo test --no-default-features --features tui,providers-api -p armadai registry cli_source_kind`
Expected: PASS.
Clippy 2 modes + fmt clean. Commit: `git commit -m "feat(cli): registry sources/sync support starters kind"`

---

### Task 3: Auto-sync-on-miss dans `init --pack` + lookup par nom (follow-up)

**Files:** Modify `src/cli/init.rs` (auto-on-miss), `src/core/starter.rs` (name-field lookup follow-up).

- [ ] **Step 1: Write the failing test** (dans `starter.rs` — name-field lookup)

```rust
#[test]
fn find_remote_pack_by_manifest_name_not_dir() {
    // A remote pack whose dir differs from its `name:` should still resolve by name.
    use std::fs;
    let tmp = std::env::temp_dir().join(format!("armadai-nm-{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp);
    let d = tmp.join("some-hash-dir/inner");
    fs::create_dir_all(&d).unwrap();
    fs::write(d.join("pack.yaml"), "name: cool-pack\n").unwrap();
    let found = find_remote_pack_by_name_in(&tmp, "cool-pack");
    assert!(found.is_some());
    let _ = fs::remove_dir_all(&tmp);
}
```

- [ ] **Step 2: Run to verify fail** — undefined `find_remote_pack_by_name_in`.

- [ ] **Step 3: Add name-based remote lookup** (`starter.rs`)

```rust
/// Find a remote pack by its `pack.yaml` `name:` (not just its dir basename),
/// so single-pack repos whose dir != name still resolve. Testable core.
fn find_remote_pack_by_name_in(cache_root: &Path, name: &str) -> Option<PathBuf> {
    for dir in remote_starter_pack_dirs_in(cache_root) {
        if let Ok(pack) = StarterPack::load(&dir)
            && pack.name == name
        {
            return Some(dir);
        }
    }
    None
}

pub fn find_remote_pack_by_name(name: &str) -> Option<PathBuf> {
    find_remote_pack_by_name_in(&crate::starters_registry::starters_cache_dir(), name)
}
```
Extend `find_pack_dir(name)`: after the existing basename remote lookup returns None, also try `find_remote_pack_by_name(name)` (still local-wins overall).

- [ ] **Step 4: Auto-sync-on-miss in `init`** (`init.rs`)

In `resolve_pack_dir(name)`: if the local candidate + `find_pack_dir(name)` both miss, sync starter sources then retry once:
```rust
pub(crate) fn resolve_pack_dir(name: &str) -> Option<std::path::PathBuf> {
    let candidate = std::path::PathBuf::from(name);
    if candidate.join("pack.yaml").is_file() {
        return Some(candidate);
    }
    if let Some(dir) = find_pack_dir(name) {
        return Some(dir);
    }
    // Auto-sync-on-miss: fetch remote starter sources, then retry once.
    let user = crate::core::registries::load_user_registries();
    let sources: Vec<_> = user.starters.clone(); // (+ project sources if resolvable here)
    if !sources.is_empty() {
        eprintln!("Pack '{name}' not found locally — syncing remote starter registries…");
        let _ = crate::starters_registry::sync_starters(&sources);
        return find_pack_dir(name);
    }
    None
}
```
(If project-level starter sources are readily resolvable in this context, include them; otherwise user-level is acceptable for v1 — document.)

- [ ] **Step 5: Run tests + clippy (2 modes + storage) + fmt + build**

Run: `cargo test --no-default-features --features tui,providers-api -p armadai starter init`
Expected: PASS.
Run: `cargo clippy --all-targets --no-default-features --features tui -- -D warnings && cargo clippy --all-targets --no-default-features --features tui,providers-api -- -D warnings && cargo clippy --no-default-features --features tui,web,storage -- -D warnings && cargo fmt -- --check && cargo build --release`
Expected: clean.

- [ ] **Step 6: Commit** `git commit -m "feat(starters): auto-sync-on-miss in init --pack + resolve remote pack by name"`

---

## Notes pour l'implémenteur
- Archive: **shell-out `tar`/`unzip`** (pas de crate) ; download via reqwest **gated providers-api**. Sans providers-api, archive → erreur claire (comportement Lot 1 conservé). `download_bytes` : réutiliser le pattern runtime de `model_registry::fetch` si présent plutôt que `Runtime::new` ad hoc.
- Ne PAS ré-introduire un `#[allow(dead_code)]` module-level ; retirer les allows item-level devenus inutiles (items désormais câblés par la CLI/init) et garder seulement ceux encore réellement non utilisés dans un mode donné.
- Auto-on-miss : réseau UNIQUEMENT sur miss (pas à chaque `init`).
- Tests archive/git skip proprement si `tar`/`unzip`/`git` absents (CI les a).
