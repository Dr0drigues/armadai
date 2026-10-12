# Plan B1 — TUI/Web enrichment

> **Statut** : À faire
> **Effort** : 🟡 Moyen (5-8 jours)
> **Version cible** : 0.11.0+

---

## État actuel

### TUI (Ratatui)

**7 tabs** : Agents, Prompts, Skills, Starters, History, Costs, Models
+ 5 vues détail (AgentDetail, PromptDetail, SkillDetail, StarterDetail, ModelDetail)

**Command palette** (`:` ou `Ctrl+p`) : 10 commandes — navigation entre tabs, refresh, new, quit

**Navigation** : `j/k`, `1-7` (tabs), `Enter` (détail), `Esc` (retour), `i` (init starter), `R` (sync models), `r` (refresh)

### Web (Axum)

**12 endpoints API** : CRUD agents, prompts, skills, starters, history, costs, models + refresh models
**UI** : SPA vanilla JS, thème dark GitHub-like, tables cliquables

### Lacunes identifiées

1. Pas de recherche/filtre dans les listes
2. Pas de tri (les listes sont en ordre de chargement)
3. History/Costs non interactifs (pas de sélection)
4. Pas de filtre avancé (par provider, model, tags, stack)
5. Web UI sans aucune recherche
6. Pas de pagination (problème de perf avec 100+ items)
7. Pas de drill-down dans les coûts (par provider, model)

---

## Features proposées

### Phase 1 — Recherche et filtres (prioritaire)

#### TUI : Recherche inline dans les listes

- **Touche `/`** : Active un champ de recherche en bas de la liste active
- Filtre en temps réel sur le nom + description (case-insensitive)
- `Esc` pour quitter la recherche, `Enter` pour sélectionner
- Appliquer sur tous les tabs : Agents, Prompts, Skills, Starters, Models

**Fichiers impactés** : `src/tui/app.rs` (état search), `src/tui/views/*.rs` (rendu filtré)

#### TUI : Tri des colonnes

- **Touche `s`** : Cycle le tri (nom ↑, nom ↓, défaut)
- Pour History : tri par date, durée, coût
- Pour Costs : tri par coût total, nombre de runs, tokens

**Fichiers impactés** : `src/tui/app.rs` (état sort), `src/tui/views/*.rs`

#### Web : Barre de recherche

- Input de recherche en haut de chaque table
- Filtre côté client (JS) — pas besoin d'endpoint supplémentaire
- Même logique : nom + description, case-insensitive

**Fichiers impactés** : `src/web/index.html`

### Phase 2 — Interactivité

#### TUI : History/Costs interactifs

- History : sélection de lignes avec `j/k`, `Enter` pour voir le détail d'un run
- Costs : sélection avec drill-down par agent → runs de cet agent
- Nouveau tab `HistoryDetail` avec le contenu complet du run

**Fichiers impactés** : `src/tui/app.rs` (nouveaux tabs), `src/tui/views/history.rs`, `src/tui/views/costs.rs`

#### Web : Détails cliquables pour History/Costs

- Lignes de table cliquables → vue détail
- Endpoint existant suffisant (`/api/history` retourne déjà les données)

### Phase 3 — Filtres avancés

#### TUI : Filtres par attribut

- **Touche `f`** : Ouvre un popup de filtre contextuel
  - Agents : par provider, model, tags
  - Skills : par outils, source
  - Models : par provider, context window, coût
- Filtres cumulables (provider=anthropic AND model contient "sonnet")

#### Web : Filtres dropdown

- Dropdowns de filtre au-dessus des tables
- Mêmes critères que le TUI

---

## Délégation

| Phase | Agent |
|-------|-------|
| Phase 1-3 TUI | @ui-specialist |
| Phase 1-3 Web | @ui-specialist |
| Tests | @qa-specialist |

## Critères de complétion

### Phase 1
- [ ] TUI : recherche `/` fonctionnelle sur tous les tabs de liste
- [ ] TUI : tri `s` sur les colonnes pertinentes
- [ ] Web : barre de recherche dans chaque table
- [ ] Tests unitaires pour la logique de filtre/tri

### Phase 2
- [ ] TUI : History et Costs interactifs avec sélection
- [ ] TUI : HistoryDetail tab
- [ ] Web : lignes cliquables History/Costs

### Phase 3
- [ ] TUI : filtres avancés par attribut (`f`)
- [ ] Web : dropdowns de filtre
- [ ] Tests de non-régression sur les vues existantes
