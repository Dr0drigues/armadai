# Plan B4 — Revue complète et audit du code

> **Statut** : À faire
> **Effort** : 🟡 Moyen (3-5 jours)
> **Version cible** : 0.11.0

---

## Contexte

Le codebase fait ~27 000 lignes de Rust réparties sur 12 modules. La CI est verte (469 tests, clippy clean). L'objectif est de consolider avant d'ajouter de nouvelles features.

## État des lieux

| Indicateur | Statut |
|-----------|--------|
| Compilation | ✅ Clean |
| Clippy | ✅ 0 warnings |
| Tests | ✅ 469 passing |
| TODOs | 1 (history.rs:10 — replay) |
| Unsafe | 15 blocs (config.rs, skill.rs — env vars) |
| unwrap() | 36 fichiers |
| panic! | 3 fichiers |

## Axes d'audit

### 1. Gestion d'erreurs (`unwrap()` audit)

**Priorité : Moyenne**

- Passer en revue les 36 fichiers contenant `unwrap()`
- Focus sur les chemins critiques : `storage/queries.rs` (64 instances), modules orchestration
- Remplacer par `?`, `.context()` (anyhow), ou `.unwrap_or_default()` selon le cas
- Conserver `unwrap()` uniquement dans les tests et les invariants prouvés

### 2. Blocs `unsafe` (config.rs, skill.rs)

**Priorité : Basse**

- 10 blocs dans `config.rs` (manipulation de variables d'environnement `set_var`/`remove_var`)
- 5 blocs dans `skill.rs`
- Vérifier que chaque usage est nécessaire et documenté
- Explorer des alternatives safe si possible (ex: `temp_env` crate pour les tests)

### 3. TODO/FIXME

**Priorité : Basse**

- `history.rs:10` — "TODO: lookup run by ID and re-execute" (replay feature)
- Décider : implémenter ou supprimer le TODO et créer une issue

### 4. Architecture et cohérence des modules

**Priorité : Moyenne**

| Module | LOC | % | Point d'attention |
|--------|-----|---|-------------------|
| `core/` | 10 619 | 39% | Module très large — orchestration seule = 6 970 LOC. Envisager un sous-module dédié (déjà fait) |
| `cli/` | 5 602 | 21% | Vérifier la cohérence des patterns entre commandes |
| `linker/` | 2 957 | 11% | 8 implémentations de linkers — vérifier la duplication |
| `tui/` | 2 363 | 9% | Vérifier la séparation état/rendu |
| `providers/` | 1 340 | 5% | Stubs OpenAI/Google/Proxy — les compléter ou les documenter |

### 5. Couverture de tests

**Priorité : Moyenne**

- 46 modules avec `#[cfg(test)]`, 469 tests
- Identifier les modules sans tests (ou avec couverture faible)
- Focus : `cli/` (intégration), `linker/` (regressions), `providers/` (mocks)

### 6. Dépendances et sécurité

**Priorité : Basse**

- Vérifier `cargo audit` pour les vulnérabilités connues
- Revoir les dépendances optionnelles et leur gating (`#[cfg(feature)]`)
- S'assurer que les features flags sont testés dans les 2 modes CI

## Délégation

| Axe | Agent |
|-----|-------|
| Gestion d'erreurs, unsafe, architecture | @core-specialist |
| Linkers, providers | @provider-specialist |
| CLI cohérence | @cli-specialist |
| TUI/Web séparation | @ui-specialist |
| Tests, couverture, CI | @qa-specialist |

## Critères de complétion

- [ ] Audit `unwrap()` — chemins critiques nettoyés
- [ ] Blocs `unsafe` — documentés ou remplacés
- [ ] TODO résolu ou transformé en issue
- [ ] Stubs providers documentés
- [ ] Couverture de tests — modules faibles identifiés et renforcés
- [ ] `cargo audit` clean
- [ ] CI verte dans les 2 modes
