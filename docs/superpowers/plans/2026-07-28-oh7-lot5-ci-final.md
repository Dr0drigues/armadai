# OH7 Lot 5 — Bin final + CI workspace-aware Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rendre la CI *workspace-aware* — exécuter les tests et clippy des 4 crates membres (que l'extraction a sortis du périmètre du `cargo test` racine) — et ajouter le garde-fou de portabilité `cargo build -p armadai-core` (cœur nu). Dernier lot d'OH7.

**Architecture:** Le job `test` de la CI lance `cargo test --no-default-features --features tui[,storage]` **à la racine**, qui ne teste QUE le package bin. Depuis les Lots 3/4, les tests de `armadai-core` (470), `armadai-providers` (52 sans `api` / 66 avec) et `armadai-storage` (22) vivent dans des crates membres et **ne sont plus exécutés en CI** (gap silencieux : ils passent en local, vérifiés à chaque lot, mais la CI ne les lance pas). Ce lot ferme le gap via des steps `-p` explicites, et ajoute le build cœur-nu. Aucun changement de code Rust — **uniquement `.github/workflows/ci.yml`**.

**Tech Stack:** GitHub Actions, Cargo workspace (resolver 3).

## Global Constraints

- **Branche** : master-only. UNE PR. Squash-merge, revue indépendante + CI verte (la CI de cette PR **est** la validation du changement — les nouveaux steps doivent tourner vert sur GitHub).
- **Aucun changement de code Rust** : seul `.github/workflows/ci.yml` est modifié. Si un nouveau step révèle un lint/test cassé dans un crate (jamais exécuté en CI jusqu'ici), STOP et signale — ce serait un vrai défaut à traiter séparément, pas à corriger ici en douce.
- **`RUSTFLAGS: "-D warnings"`** est global dans la CI → tous les `cargo test`/`clippy`/`build` ajoutés héritent de `-D warnings`. Vérifier que chaque crate est warning-clean sous ce flag.
- **Vérif locale** : chaque commande ajoutée doit tourner vert en local (`RUSTFLAGS="-D warnings" cargo …`) avant commit. La validation finale = CI verte sur la PR.
- **Commit anglais**, Conventional Commits scope `oh7`/`ci`. Finir par `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`.
- **NE PAS `git add -A`** (untracked pré-existant). Stager `.github/workflows/ci.yml` explicitement.

## File Structure

Seul fichier modifié : `.github/workflows/ci.yml` (jobs `clippy`, `test`, `build`).

---

## Task 5: CI workspace-aware + garde-fou cœur nu

**Files:**
- Modify: `.github/workflows/ci.yml`

- [ ] **Step 1: Branche**

```bash
cd "$(git rev-parse --show-toplevel)"
git checkout master && git pull --ff-only
git checkout -b feat/oh7-5-ci-workspace
```

- [ ] **Step 2: Vérifier localement que chaque commande membre est verte sous `-D warnings`**

```bash
export RUSTFLAGS="-D warnings"
cargo build -p armadai-core                                   # cœur nu featureless
cargo clippy -p armadai-core --all-targets -- -D warnings
cargo clippy -p armadai-secrets --all-targets -- -D warnings
cargo clippy -p armadai-storage --all-targets -- -D warnings
cargo clippy -p armadai-providers --all-targets --features api -- -D warnings
cargo test -p armadai-core
cargo test -p armadai-secrets
cargo test -p armadai-storage
cargo test -p armadai-providers --features api
unset RUSTFLAGS
```
Attendu : tout vert. Si un lint/test casse dans un crate → STOP + signale (défaut réel jamais vu en Cci). Noter les comptes de tests : core, secrets (0), storage (22), providers `--features api` (66).

- [ ] **Step 3: Ajouter les steps clippy des crates membres au job `clippy`**

Dans `.github/workflows/ci.yml`, job `clippy`, APRÈS la ligne `cargo clippy --all-targets --no-default-features --features tui,web,storage -- -D warnings`, ajouter :

```yaml
      # Workspace member crates lint their own test targets too (the bin
      # clippy above compiles them as path deps but does not lint their
      # `#[cfg(test)]` targets). armadai-providers needs `api` to cover its
      # HTTP providers + model_registry online fetch.
      - run: cargo clippy -p armadai-core --all-targets -- -D warnings
      - run: cargo clippy -p armadai-secrets --all-targets -- -D warnings
      - run: cargo clippy -p armadai-storage --all-targets -- -D warnings
      - run: cargo clippy -p armadai-providers --all-targets --features api -- -D warnings
```

- [ ] **Step 4: Ajouter les steps test des crates membres au job `test`**

Dans le job `test`, APRÈS `- run: cargo test --no-default-features --features tui,storage` (et avant le step « Upload e2e report »), ajouter :

```yaml
      # Extracted workspace member crates are NOT covered by the root-package
      # `cargo test` above (they live in crates/*, outside the bin package).
      # Run their suites explicitly so armadai-core (domain + ES engine),
      # armadai-providers (incl. the `api`-gated HTTP providers), armadai-storage
      # and armadai-secrets stay exercised in CI after the OH7 extraction.
      - run: cargo test -p armadai-core
      - run: cargo test -p armadai-secrets
      - run: cargo test -p armadai-storage
      - run: cargo test -p armadai-providers --features api
```

- [ ] **Step 5: Ajouter le garde-fou build cœur-nu au job `build`**

Dans le job `build`, APRÈS `- run: cargo build --release --no-default-features --features tui,storage`, ajouter :

```yaml
      # Portability guard (OH7): the reusable core must compile standalone,
      # featureless, with no heavy deps (no reqwest/rusqlite/ratatui/axum) —
      # this is what OH2/the Claude Code plugin will depend on. A dep leaking
      # into core (e.g. via unintended feature unification) fails here.
      - run: cargo build -p armadai-core
```

- [ ] **Step 6: Valider la syntaxe YAML localement**

```bash
python3 -c "import yaml,sys; yaml.safe_load(open('.github/workflows/ci.yml')); print('YAML OK')"
```
Attendu : `YAML OK`.

- [ ] **Step 7: Commit**

```bash
git add .github/workflows/ci.yml
git status --short | grep -v '^??'
git commit -m "$(cat <<'MSG'
ci(oh7): run workspace member crate suites + bare-core build guard (#252)

After the OH7 extraction, the root-package `cargo test` no longer covers
the crates/* members. Add explicit -p test + clippy steps for
armadai-core/providers/storage/secrets (providers with `api`), and a bare
`cargo build -p armadai-core` portability guard (featureless, no heavy deps).

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
MSG
)"
```

---

## Invariant de fin de Lot 5 (et d'OH7)

- La CI exécute les suites des 4 crates membres (`-p armadai-core/secrets/storage/providers --features api`) en plus des 2 modes bin → plus de gap de couverture post-extraction.
- La CI clippy les 4 crates membres (`--all-targets`) → leurs test-targets sont lintés.
- La CI build `armadai-core` nu (garde-fou portabilité) → une fuite de dep dans le cœur échoue en CI.
- Aucun changement de code Rust ; seul `ci.yml` modifié.
- **OH7 complet** : `armadai-core` (feuille featureless réutilisable), `armadai-providers`, `armadai-storage`, `armadai-secrets` extraits ; bin = interfaces + adaptateurs ; CI workspace-aware.

## Hors périmètre / suivi

- **`[workspace.dependencies]`** (centraliser les versions partagées anyhow/serde/tokio/… entre les 4 crates) : YAGNI/cosmétique, non fait ; follow-up si la dérive de versions devient gênante.
- **Test du bin avec `providers-api`** (`cargo test --features tui,providers-api`) : gap **pré-existant à OH7** (le job test n'a jamais activé providers-api) ; hors périmètre de ce lot (qui ferme le gap *introduit* par OH7). À noter comme suivi éventuel.
- Split des interfaces (tui/web/cli/shell en crates) : non retenu (choix « couches »).

## Self-Review (rempli à l'écriture)

- **Couverture spec** : garde-fou `cargo build -p armadai-core` ✓ ; features déjà re-plombées aux Lots 2–4 (storage, providers-api) → rien à re-plomber ici ✓ ; le vrai enjeu (gap CI tests membres) identifié et fermé ✓.
- **Placeholders** : aucun ; toutes les commandes sont concrètes.
- **Risque** : un crate jamais testé en CI pourrait révéler un lint/test rouge → Step 2 le détecte en local AVANT le commit ; consigne STOP-et-signale plutôt que corriger en douce.
- **Cohérence** : `providers` testé/linté `--features api` (couvre les 66 dont 14 api-gated jamais exécutés) ; core featureless ; storage/secrets sans feature.
