# Propale — Audit dead code automatisé & Stratégie plugin Claude

**Date** : 2026-05-12
**Auteur** : Dimitri RODRIGUES-OLIVEIRA (avec assist Claude Code)
**Statut** : Draft, à valider
**Cible** : v1.0.0 (P0 audit) + post-v1 (plugin)

---

## 1. Contexte

Deux sujets remontés en session :

1. **Résidus de code inutiles** dans le codebase (constat empirique : `#[allow(dead_code)]` sur `core/config` et `core/orchestration`, wrappers passthrough découverts dans `json_runner.rs` lors du cleanup #122, dead code détecté manuellement dans `parser/{mod.rs,metadata.rs}` lors de PR #135). Besoin d'automatiser la détection plutôt que de compter sur la chance.

2. **Transformation `armadai-authoring` en plugin Claude** : opportunité de distribution + supposé gain de tokens. À challenger.

---

## 2. Sujet A — Audit dead code automatisé

### 2.1 Diagnostic actuel

Indicateurs de dette :

| Localisation | Type | Statut |
|--------------|------|--------|
| `src/core/mod.rs:2-3` | `#[allow(dead_code)] pub mod config` | À investiguer |
| `src/core/mod.rs:7-8` | `#[allow(dead_code)] pub mod orchestration` | À investiguer |
| `src/parser/` | `validate_agent`, `validate_metadata` | ✅ Supprimés (PR #135) |
| `src/shell/json_runner.rs` | `JsonFlags`, `parse_json_response`, wrappers codex/copilot/opencode | ✅ Supprimés (PR #122) |
| `src/providers/{openai,google,proxy}.rs` | Stubs | À évaluer (utilisés en production ?) |
| `src/cli/` | Commandes orphelines ? | Non audité |

**Constat** : la dette est détectée par hasard. Un audit complet manuel prendrait ~1 jour. Il faut un filet automatique.

### 2.2 Stratégie en 3 niveaux

#### Niveau 1 — Outillage CI stable (effort: 1h, ROI: élevé)

Ajoute dans `.github/workflows/ci.yml` un job `dead-code-check` :

```yaml
dead-code-check:
  runs-on: ubuntu-latest
  steps:
    - uses: actions/checkout@v4
    - name: Install cargo-machete
      run: cargo install cargo-machete --locked
    - name: Detect unused dependencies
      run: cargo machete
    - name: Ban #[allow(dead_code)] outside tests
      run: |
        # Trouve les occurrences hors blocs #[cfg(test)] et hors fichiers tests/
        ! grep -rn "#\[allow(dead_code)\]" src/ \
          --include="*.rs" \
          | grep -v "tests/" \
          | grep -v "#\[cfg(test)\]"
```

**Cible** : faire échouer la CI si :
- `Cargo.toml` contient des deps non importées
- Un nouveau `#[allow(dead_code)]` est introduit en code production

**Limites** : ne détecte pas les fonctions/structs internes non appelées (limites du rustc en mode stable).

#### Niveau 2 — Lint strict ponctuel (effort: 2-4h selon résultats, ROI: élevé)

Audit semestriel ou pré-release :

```bash
RUSTFLAGS="-D dead_code -D unused_imports -D unused_variables -D unused_must_use" \
  cargo +nightly check --all-targets --all-features 2>&1 | tee dead-code-report.md
```

Couplé avec `cargo +nightly udeps` pour les deps unused au niveau build (plus précis que `machete`).

**Output attendu** : rapport markdown listant chaque résidu, classé par module. Triage manuel → 1 PR par module pour ne pas tout mélanger.

**Variante** : exposer ce flow dans une nouvelle commande `armadai audit-dead-code` qui :
- Build avec les flags ci-dessus
- Parse la sortie rustc en JSON
- Génère un rapport markdown groupé par module
- (Bonus) Crée une issue GitHub par module avec checklist

#### Niveau 3 — Agent ArmadAI dédié (effort: 1 jour, ROI: moyen, dogfooding élevé)

Agent `dead-code-auditor` dans le pack `armadai-authoring` :

```yaml
# starters/armadai-authoring/agents/dead-code-auditor.md
- provider: anthropic
- model: claude-sonnet-4-6
- tags: [audit, quality]
- scope: [src/**/*.rs]
```

**Inputs** :
- Rapport `cargo +nightly check -D dead_code`
- Sortie `cargo machete`
- Liste des `#[allow(dead_code)]` (grep)

**Outputs** :
- Plan de cleanup priorisé par module
- Justification de chaque suppression (ou non-suppression : "stub provider conservé pour future API")
- Optionnellement, applique les suppressions via `Write`/`Edit`

**Pipeline** : `dead-code-auditor → reviewer → refactor-applier` (chaîne de 3 agents dans une `pipeline:` nommée, via item #31 du backlog).

### 2.3 Recommandation

**Séquence** :

1. **Maintenant** (post-merge #135/#136) : niveau 1 (CI gate)
2. **Avant release v1.0.0** : niveau 2 (audit ponctuel) → liste exhaustive, on traite par PRs thématiques
3. **Après v1.0.0** : niveau 3 (agent dédié) → dogfooding, démontre la puissance d'ArmadAI sur son propre code

**Risque principal** : niveau 2 peut révéler beaucoup de dette → discipline nécessaire pour ne pas mélanger cleanup et features.

---

## 3. Sujet B — Plugin Claude vs Starter ArmadAI

### 3.1 Hypothèse à challenger

> "Transformer `armadai-authoring` en plugin Claude permettrait des économies de tokens."

### 3.2 Réalité technique

| Mécanisme | System prompt parent | System prompt agent | Total |
|-----------|---------------------|---------------------|-------|
| `armadai run <agent>` (API directe) | ø | ~2-5k | **~2-5k** |
| Plugin Claude Code | ~25k (Claude Code) | ~2-5k (ajouté) | **~27-30k** |
| Agent loaded via `armadai link` puis lancé dans Claude Code | ~25k (Claude Code) | ~2-5k (système) | **~27-30k** |

**Conclusion factuelle** : le plugin Claude **ne fait pas** d'économie de tokens vs l'usage actuel. Pire, il ajoute son prompt à celui de Claude Code parent (il ne le remplace pas).

**La vraie économie de tokens** se fait via `armadai run` direct, qui contourne complètement le harness Claude Code.

### 3.3 Autres dimensions à considérer

| Dimension | Plugin Claude | Starter ArmadAI |
|-----------|---------------|-----------------|
| **Tokens** | ❌ Identique au statu quo | ✅ Économie si `armadai run` direct |
| **Distribution** | ✅ Marketplace Claude | ⚠️ Via `armadai init --pack` |
| **Provider-agnostic** | ❌ Claude only | ✅ Cursor, Copilot, Gemini, Aider, etc. |
| **Maintenance** | ❌ Duplique le code | ✅ Source unique |
| **Discoverability** | ✅ Visible dans Claude | ⚠️ Nécessite connaître ArmadAI |
| **Cohérence vision** | ❌ Trahit le design provider-agnostic | ✅ Aligné |

### 3.4 Options

**Option 1 — Statu quo** ⭐ recommandée
- Garder `armadai-authoring` comme starter pack.
- **Promouvoir l'usage via `armadai run`** pour le token-saving (à documenter).
- Ajouter une commande `armadai author <agent-name> "<prompt>"` qui wrappe `armadai run` avec un flag pré-rempli pour les agents authoring.

**Option 2 — Plugin Claude thin**
- Créer un plugin Claude **minimal** qui se contente d'invoquer `armadai run <agent>` en arrière-plan.
- Pas de duplication de prompts (pas hébergés dans le plugin).
- Utile uniquement pour la discoverability marketplace.
- Coût : ~1 jour de setup + maintenance d'une dépendance externe (binaire `armadai` installé).

**Option 3 — Transformation complète**
- Réimplémenter `armadai-authoring` comme plugin Claude full.
- ❌ Vendor lock-in.
- ❌ Maintenance doublée.
- ❌ Trahit la vision ArmadAI.
- Aucun argument technique en faveur, sauf si on abandonne le multi-provider (ce qui n'est pas le plan).

### 3.5 Recommandation

**Option 1** maintenant + **Option 2** plus tard si traction sur le marketplace Claude justifie l'effort.

Action concrète à ajouter au backlog v1 :

> **42. `armadai author <agent>` quick-launcher** — wrappe `armadai run` avec UX optimisée pour le workflow authoring (auto-load pack, model par défaut, prompt history). Documente le gain de tokens vs Claude Code.

---

## 4. Décisions à acter

| # | Décision | Validation requise |
|---|----------|--------------------|
| D1 | Niveau 1 (CI gate dead-code) implémenté avant merge release/1.0.0 | Oui |
| D2 | Niveau 2 (audit ponctuel) planifié comme pré-release v1.0.0 | Oui |
| D3 | Niveau 3 (agent `dead-code-auditor`) → post-v1.0.0 | Oui |
| D4 | `armadai-authoring` reste un starter pack (pas de transformation plugin) | Oui |
| D5 | Ajouter item #42 (`armadai author`) au backlog v1 | Oui |
| D6 | Option 2 (plugin thin) reportée post-v1.0.0, ré-évaluée selon traction | Oui |

---

## 5. Prochaines étapes si propale validée

1. Créer issue/PR pour D1 (CI gate)
2. Mettre à jour `docs/SESSION-STATE.md` :
   - Item #2bis (déjà ajouté) raffiné avec ces 3 niveaux
   - Nouveau item #42 (`armadai author`)
3. Documenter dans `README.md` la stratégie token-saving (`armadai run` direct vs Claude Code)
4. Sur le wiki : page "Audit qualité" listant les outils + procédure
