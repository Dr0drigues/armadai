# OH7 Lot 3 — Extraire `armadai-core` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extraire tout `src/core/` (types domaine + trait `Provider` + `EventLog`/`InMemoryLog` + parser + routing + moteur event-sourcé) vers un crate lib feuille `crates/armadai-core/`, avec les assets embarqués `skills/`/`starters/` qu'il possède, sans aucun changement de logique.

**Architecture:** `armadai-core` devient une **feuille pure featureless** (0 `#[cfg(feature)]`, 0 dépendance lourde : ni reqwest, ni rusqlite, ni ratatui). Le déplacement est **mécanique** — `core` est déjà cycle-free depuis le Lot 1 (0 `use crate::` sortant). `lib.rs` = le `mod.rs` actuel verbatim (les `pub mod X`), donc l'API publique reste `armadai_core::<module>::<Item>`, image exacte de `crate::core::<module>::<Item>`. **Aucune façade `pub use` curatée ce lot-ci** (YAGNI — reportée à OH2/plugin).

**Tech Stack:** Rust edition 2024, Cargo workspace (déjà en place depuis Lot 2c), `include_dir` (assets skills/starters), tokio/async-trait (trait Provider async), pulldown-cmark (parser).

## Global Constraints

- **Branche** : master-only. UNE PR pour ce lot (l'extraction est atomique — un déplacement partiel de core ne compile pas). Squash-merge, revue indépendante + CI verte (6 checks) avant merge. CI verte ≠ suffisante.
- **Gate CI** (le bin reste la référence, features propagées) :
  - `cargo fmt --all -- --check`
  - clippy 3 modes `-D warnings` : `--no-default-features --features tui` ; `… tui,providers-api` ; `… tui,web,storage`
  - `cargo test` 2 modes : `--no-default-features --features tui` ; `--no-default-features --features tui,storage`
  - **+ `cargo build -p armadai-core`** (cœur nu, featureless — garde-fou de portabilité, vérifié localement ; l'ajout au YAML CI est Lot 5).
- **Refactor pur** : aucun test réécrit sauf déplacement d'`use`/de fichier. Réconciliation des tests obligatoire : les tests de core migrent sous `cargo test -p armadai-core` ; `(bin_after + core_crate_tests)` doit égaler le total précédent (baseline bin : 1042 en `tui`, 1088 en `tui,storage`).
- **`rust-analyzer` non fiable** (stale/ABI) → **vérifier au compilateur** uniquement.
- **Code/commentaires/commits en anglais.** Conventional Commits, scope `oh7`. Finir le message de commit par `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`.
- **NE PAS `git add -A`** : le working tree contient beaucoup d'untracked pré-existant (docs/superpowers plans, graphify-out/, examples/*/.claude/, etc.). Stager explicitement les fichiers du lot.
- **`fake-claude`** (`src/bin/fake-claude.rs`) : reste dans le bin ; s'il référence `crate::core`, il est repointé comme le reste du bin.

---

## File Structure

Après le Lot 3 :

```
Cargo.toml                    # racine : [workspace] + [package]. Ajout dep armadai-core ; retrait rien d'autre.
src/
  main.rs                     # `mod core;` RETIRÉ ; imports `crate::core::` inchangés SAUF réécrits en `armadai_core::`
  es_log.rs, db.rs, ...       # ~50 fichiers bin : `crate::core::` -> `armadai_core::`
  providers/                  # (reste dans le bin ce lot ; 7 sites `crate::core::` -> `armadai_core::`) — extrait au Lot 4
crates/
  armadai-core/
    Cargo.toml                # NOUVEAU : featureless, deps légères
    skills/                   # git mv depuis la racine (embarqué via include_dir!)
    starters/                 # git mv depuis la racine (embarqué via include_dir!)
    src/
      lib.rs                  # = ex-src/core/mod.rs verbatim (pub mod agent; … pub mod starter;)
      agent.rs, config.rs, provider.rs, routing.rs, model_*.rs, events.rs, ...
      orchestration/          # + orchestration/es/ (moteur + 4 patterns)
      parser/
```

> **Racine dépouillée** : `skills/` et `starters/` quittent la racine du repo (elles vont dans le crate). `web/ui/dist` reste (embarqué par `src/web`, bin-side).

---

## Task 3: Extraire `armadai-core`

Extraction atomique de `src/core/` → `crates/armadai-core/`, avec `skills/`+`starters/`, repointage bidirectionnel des chemins, et câblage du bin.

**Files:**
- Create: `crates/armadai-core/Cargo.toml`
- Move (dir): `src/core/` → `crates/armadai-core/src/` (avec `mod.rs` renommé `lib.rs`)
- Move (dirs): `skills/` → `crates/armadai-core/skills/` ; `starters/` → `crates/armadai-core/starters/`
- Modify: `Cargo.toml` racine (ajout dépendance `armadai-core`)
- Modify: `src/main.rs` (retrait `mod core;`)
- Modify (imports): ~50 fichiers bin + `src/providers/*` — `crate::core::` → `armadai_core::`
- Modify (internes): fichiers déplacés dans le crate — `crate::core::` → `crate::`

**Interfaces:**
- `armadai-core` — Produces: l'API publique `armadai_core::<module>::<Item>` pour chaque `pub mod` de l'ancien `core/mod.rs` (agent, config, dependency_resolver, events, model_aliases, model_resolution, model_updater, orchestration, pack_validation, parser, project, project_registry, prompt, provider, registries, routing, skill, starter). Notamment : `armadai_core::provider::{Provider, CompletionRequest, CompletionResponse, ProviderMetadata, TokenStream, ChatMessage}`, `armadai_core::orchestration::es::{…}`, `armadai_core::agent::{Agent, AgentMetadata}`, `armadai_core::parser::parse_agent_file`.
- Consumed by bin + (au Lot 3) le module `providers` encore dans le bin.

- [ ] **Step 1: Créer la branche**

```bash
cd "$(git rev-parse --show-toplevel)"
git checkout master && git pull --ff-only
git checkout -b feat/oh7-3-extract-core
```

- [ ] **Step 2: Déplacer le répertoire core + renommer mod.rs → lib.rs**

```bash
mkdir -p crates/armadai-core
git mv src/core crates/armadai-core/src
git mv crates/armadai-core/src/mod.rs crates/armadai-core/src/lib.rs
```

- [ ] **Step 3: Déplacer les assets embarqués skills/ + starters/ dans le crate**

```bash
git mv skills crates/armadai-core/skills
git mv starters crates/armadai-core/starters
```

Vérifier que les chemins `include_dir!` du crate résolvent maintenant en interne :
```bash
grep -rn 'include_dir!' crates/armadai-core/src/skill.rs crates/armadai-core/src/starter.rs
# doivent rester include_dir!("$CARGO_MANIFEST_DIR/skills") / "$CARGO_MANIFEST_DIR/starters"
ls crates/armadai-core/skills crates/armadai-core/starters   # existent
```

> `$CARGO_MANIFEST_DIR` du crate `armadai-core` = `crates/armadai-core/`, donc `$CARGO_MANIFEST_DIR/skills` résout dans le crate. Les fallbacks runtime CWD (`./starters`) et projet (`.armadai/starters`) sont inchangés (relatifs à l'exécution, pas au manifest).

- [ ] **Step 4: Réécrire les auto-références INTERNES au crate (`crate::core::` → `crate::`)**

Dans le crate déplacé, `crate::` désigne désormais la racine du crate core (plus le bin). Tous les `crate::core::X` internes deviennent `crate::X` :

```bash
grep -rl "crate::core::" crates/armadai-core/src/ \
  | xargs sed -i '' 's/crate::core::/crate::/g'
# Vérifier qu'il ne reste aucun crate::core:: dans le crate
grep -rn "crate::core" crates/armadai-core/src/ && echo "!! RESTE" || echo "OK: crate core auto-cohérent"
```
Attendu : `OK`. (Note : les mentions `crate::core` en **doc-comment** deviennent aussi `crate::` — acceptable ; si une mention prose devient incorrecte, la corriger en texte comme au Lot 2d.)

- [ ] **Step 5: Créer `crates/armadai-core/Cargo.toml`**

```toml
[package]
name = "armadai-core"
version = "1.0.0-rc.5"
edition.workspace = true
license.workspace = true
repository.workspace = true
description = "ArmadAI — reusable orchestration core: domain types, Provider/EventLog traits, parser, event-sourced engine."

[dependencies]
anyhow = "1"
thiserror = "2"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
serde_yaml_ng = "0.10"
async-trait = "0.1"
futures-util = { version = "0.3", default-features = false, features = ["std", "async-await"] }
tokio = { version = "1", features = ["time", "macros", "rt", "rt-multi-thread", "sync"] }
tokio-stream = { version = "0.1", features = ["io-util"] }
pulldown-cmark = "0.13"
include_dir = "0.7"
```

> **Réconcilier au compilateur** : lancer `cargo build -p armadai-core` puis `cargo test -p armadai-core`. Ajouter/retirer une dépendance selon les erreurs `unresolved import` / warnings `unused`. Les 11 crates ci-dessus proviennent de l'audit des `use` de core ; `tokio` features à ajuster si un `unresolved` apparaît (core utilise `tokio::time::sleep` + `tokio::test`). AUCUNE dep lourde (reqwest/rusqlite/ratatui/axum) ne doit apparaître — si l'une est requise, STOP et signale (ce serait une fuite de couplage inattendue).

- [ ] **Step 6: Câbler la dépendance dans le bin + retirer `mod core;`**

Dans `Cargo.toml` racine, section `[dependencies]`, ajouter :
```toml
armadai-core = { path = "crates/armadai-core" }
```
Dans `src/main.rs`, retirer la ligne `mod core;`.

- [ ] **Step 7: Repointer le bin (`crate::core::` → `armadai_core::`)**

```bash
# tout src/ SAUF le crate déjà déplacé (src/core n'existe plus)
grep -rl "crate::core::" src/ \
  | xargs sed -i '' 's/crate::core::/armadai_core::/g'
# forme sans :: finale (rare, ex. `use crate::core;` ou `crate::core}`)
grep -rl "crate::core\b" src/ \
  | xargs sed -i '' 's/crate::core\b/armadai_core/g'
# Vérifier plus aucun crate::core dans le bin
grep -rn "crate::core\|mod core" src/ && echo "!! RESTE (vérifier faux positifs 'mod core_xxx')" || echo "OK: bin repointé"
```
Attendu : `OK` (ou uniquement des faux positifs `mod core_something` — vérifier au cas par cas ; il ne doit rester aucun `crate::core::` ni `mod core;` réel).

- [ ] **Step 8: Vérifier tests/ (integration e2e) ne référence pas core en interne**

```bash
grep -rn "crate::core\|armadai::core" tests/ 2>/dev/null && echo "vérifier" || echo "OK: tests e2e n'accèdent pas aux internes (assert_cmd/binaire)"
```

- [ ] **Step 9: Build du cœur nu (garde-fou portabilité)**

```bash
cargo build -p armadai-core 2>&1 | tail -3
```
Attendu : `Finished`. Aucune dep lourde tirée (vérif optionnelle : `cargo tree -p armadai-core | grep -E 'reqwest|rusqlite|ratatui|axum'` → vide).

- [ ] **Step 10: Gate CI complète**

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --no-default-features --features tui -- -D warnings
cargo clippy --all-targets --no-default-features --features tui,providers-api -- -D warnings
cargo clippy --all-targets --no-default-features --features tui,web,storage -- -D warnings
cargo test --no-default-features --features tui 2>&1 | grep "test result:"
cargo test --no-default-features --features tui,storage 2>&1 | grep "test result:"
cargo test -p armadai-core 2>&1 | grep "test result:"
```
Attendu : fmt clean ; clippy 3 modes clean ; réconciliation des tests OK (voir Step 11).

- [ ] **Step 11: Réconcilier les comptes de tests**

Relever : les tests lib du bin en mode `tui` et `tui,storage`, + les 8 fake-claude + 30 e2e (constants), + le compte `cargo test -p armadai-core`. Vérifier :
`(lib_bin_tui,storage + 8 + 30) + core_crate_tests == 1088` (le total baseline avant extraction). Le bin lib doit chuter exactement du nombre de tests qui vivaient dans `core/*` (ils tournent désormais sous `armadai-core`). Documenter les nombres exacts dans le rapport. AUCUN test perdu, uniquement relocalisé.

- [ ] **Step 12: Commit**

```bash
git add crates/armadai-core Cargo.toml Cargo.lock src/
# NE PAS git add -A. Vérifier le staging :
git status --short | grep -v '^??'
git commit -m "$(cat <<'MSG'
refactor(oh7): Lot 3 — extract armadai-core leaf crate (#252)

Move src/core/* (+ skills/ + starters/ embedded assets) into
crates/armadai-core/ as a featureless leaf crate. Pure mechanical
extraction (core was already cycle-free since Lot 1):
- lib.rs = old core/mod.rs verbatim (public API = armadai_core::<module>::<Item>)
- internal crate::core:: -> crate:: ; bin crate::core:: -> armadai_core::
- skills/ + starters/ moved into the crate so include_dir!($CARGO_MANIFEST_DIR/...)
  resolves; core stays self-contained for OH2/plugin reuse
- no heavy deps in core (no reqwest/rusqlite/ratatui)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
MSG
)"
```

---

## Invariant de fin de Lot 3

- `crates/armadai-core/` extrait ; `cargo build -p armadai-core` vert ; `cargo tree -p armadai-core` ne contient ni reqwest, ni rusqlite, ni ratatui, ni axum.
- `grep -rn "crate::core\|mod core;" src/` → vide (hors faux positifs `core_xxx`).
- `grep -rn "crate::core" crates/armadai-core/src/` → vide (auto-cohérent).
- `skills/` et `starters/` ne sont plus à la racine du repo ; ils sont dans `crates/armadai-core/` et embarqués via `include_dir!`.
- Gate verte 3 modes clippy + 2 modes test ; réconciliation des tests exacte (aucun perdu).
- `include_dir!` du bin (`web/ui/dist`) inchangé ; extraction/install skills+starters fonctionnelle (tests `starter.rs`/`skill.rs` passent sous `armadai-core`).

## Hors périmètre (rappel)

- Extraction de `armadai-providers` (Lot 4) — `providers` reste dans le bin ce lot, simplement repointé vers `armadai_core`.
- Façade `pub use` curatée de `armadai-core` (ergonomie `armadai_core::Agent`) — YAGNI, reportée (OH2/plugin définiront les vrais besoins). `lib.rs` = `pub mod` actuels.
- Ajout du `cargo build -p armadai-core` au YAML CI (`.github/workflows/ci.yml`) — Lot 5 (vérifié localement ici).

## Self-Review (rempli à l'écriture)

- **Couverture spec** : déplacement core+parser+Provider+EventLog+ES ✓ ; skills/starters (Option A) ✓ ; featureless + deps légères ✓ ; façade YAGNI documentée ✓ ; garde-fou cœur nu (local, YAML au Lot 5) ✓.
- **Placeholders** : le manifeste a un step de réconciliation-au-compilateur explicite (Step 5). Pas de « TBD ».
- **Cohérence chemins** : repointage bidirectionnel explicité (interne `crate::core::`→`crate::` AVANT bin `crate::core::`→`armadai_core::`), avec vérifications grep à chaque sens.
- **Piège assets** : `$CARGO_MANIFEST_DIR` déplacé → skills/starters déplacés en Step 3 pour que l'`include_dir!` résolve ; fallbacks runtime CWD/projet inchangés.
- **Piège tests** : réconciliation obligatoire (Step 11) — les tests de core migrent, le total doit être conservé.
- **Piège atomicité** : un déplacement partiel ne compile pas → tout en une PR/un commit ; pas de sous-lots verts intermédiaires possibles.
