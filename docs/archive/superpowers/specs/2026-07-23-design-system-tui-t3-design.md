# Design System → TUI T3 — Workroom adaptative, drill-down, nettoyage couleurs — Design

## Contexte

3ᵉ et dernier sous-lot du chantier design system côté **TUI** (T1 fondation thème + dashboard, T2 re-skin des vues, tous deux consolidés dans `#239` : thème central `src/theme.rs` accent-only, chaque `Block` bordé hérite du fg neutre `BORDER`). T3 traite la surface la plus dynamique — la **Workroom** du shell (`src/shell/workroom.rs`, rendue par `src/shell/tui.rs`/`app.rs`, feature `tui`) — plus la résorption des dernières couleurs codées en dur relevées par la revue de `#239`.

État actuel de la Workroom : composant du **shell** (`armadai shell`, PAS `armadai tui`). Rendu = une **liste indentée par rôle** (`Coordinator`/`Lead`/`Agent`), quel que soit le pattern d'orchestration. Infra de drill-down **déjà présente** : `selected`, `focused`, `set_focused`, `select_next/prev`, `selected_detail_markdown()`, `transitions`, `last_action`, `Ctrl+W` focus, popup (`show_popup`/`dismiss_popup` dans `shell/tui.rs`), hints `Ctrl+W focus · j/k select · Enter detail`.

## Objectif

Rendre la Workroom **adaptative au pattern d'orchestration** (`Hierarchical` / `Blackboard` / `Ring`) avec trois layouts distincts en mode focus, styliser le drill-down au DS, et éliminer tous les `Color::` littéraux restants des vues/widgets TUI + Workroom.

## Décisions validées (Dimitri, 2026-07-23)

1. **Ambition Workroom** : 3 layouts distincts (arbre hierarchical / colonnes blackboard / anneau ring), pas un simple arbre relabelisé.
2. **Largeur** : panneau compact (~35 cols) en veille ; **élargi (~60 cols) au focus Ctrl+W** pour afficher le layout riche ; **dégradation en liste compacte** si la largeur effective passe sous un seuil (44 cols), même en focus.
3. **Découpage** : T3a (plumbing + hierarchical) → T3b (blackboard + ring) → T3c (drill-down stylé + nettoyage couleurs). Une PR + revue indépendante + validation visuelle Dimitri par sous-lot.

## Architecture

### Composant modifié
`src/shell/workroom.rs` (logique + rendu) ; ajustement du calcul de largeur côté `src/shell/tui.rs`/`app.rs` (allocation de la zone Workroom conditionnée par `is_focused()`). Aucun autre module touché par la partie Workroom.

### Flux de données du pattern
- Nouveau champ `Workroom.pattern: OrchestrationPattern` (défaut `OrchestrationPattern::Hierarchical`), réutilisant l'enum existant `crate::core::orchestration::OrchestrationPattern { Hierarchical, Blackboard, Ring }`.
- Renseigné dans `init_from_config(&str)` par une **détection tolérante ligne à ligne** de la clé `pattern:` sous le bloc `orchestration:` (même style que le parsing coordinator/teams existant) : valeurs `hierarchical`/`blackboard`/`ring` (insensibles à la casse, quotes tolérées) ; toute autre valeur ou absence → `Hierarchical`.

### Dispatch de rendu
`render(&self, frame, area)` devient un dispatch sur `(self.pattern, self.focused, area.width)` :
- **Veille** (`!focused`) : rendu **compact** = la liste indentée actuelle, quel que soit le pattern. En-tête de bloc `" Workroom · <Pattern> "`.
- **Focus** (`focused`) ET `area.width >= SEUIL` (44) : rendu **riche** du pattern → `render_hierarchical` / `render_blackboard` / `render_ring`.
- **Focus mais `area.width < 44`** : dégradation → rendu compact (liste).
- L'élargissement de `area` (35 → ~60 cols au focus) est piloté par le **shell** : la contrainte de layout allouée à la Workroom devient conditionnelle à `workroom.is_focused()`. Pas de nouvel état global.

### Les trois layouts (mode focus, ~60 cols)

Glyphes via `theme::glyphs()` (unicode par défaut, ASCII sous `--ascii`). États : `Working` (spinner + `{elapsed}s`), `Delegating` (spinner), `Done` (`✓`/glyph ok), `Idle` (`○`/glyph muted). Styles d'état via le thème : working→`theme::working()`, delegating→`theme::delegating()`, done→`theme::done()`, idle→`theme::muted()`. Rôles via `theme::role_coordinator/role_lead/role_agent`. Bordures via `theme::border_style()`, titre via `theme::heading()`.

**Hierarchical** — arbre pyramidal avec connecteurs box-drawing (`├─`, `└─`) au lieu de la simple indentation :
```
┌ Workroom · Hierarchical ──────────────────────┐
│ ⚑ dev-lead              working 4s              │
│ ├─ ⚑ core-specialist    working 2s              │
│ ├─ ○ cli-specialist     idle                    │
│ └─ ✓ qa-specialist      done                    │
└────────────────────────────────────────────────┘
```
Connecteurs : les enfants (`Lead`/`Agent`) sous un parent utilisent `├─` sauf le dernier de leur groupe → `└─`. Variante ASCII (`--ascii`) : `+-` / `\-` via `glyphs()`.

**Blackboard** — parallèle à état partagé, pas de hiérarchie : une ligne d'en-tête « board » puis les agents à plat :
```
┌ Workroom · Blackboard ─────────────────────────┐
│ ▤ shared board · 3 agents · round 2            │
│   ⚑ researcher   working 5s                     │
│   ⚑ writer       working 1s                     │
│   ○ critic       idle (waiting on board)        │
└────────────────────────────────────────────────┘
```
La ligne board affiche : glyphe board (`▤`, ASCII `#`), nombre d'agents, et le compteur de round s'il est connu (sinon omis). Les agents `Idle` peuvent porter le suffixe `(waiting on board)`.

**Ring** — séquentiel avec passage de jeton : agents en anneau, **détenteur du jeton** (`current_agent`) mis en évidence en laiton, flèches de tour entre agents et une flèche de bouclage finale :
```
┌ Workroom · Ring ───────────────────────────────┐
│  ▸ architect ⚑ working 3s   ← holds token       │
│    ↓                                            │
│    reviewer  ○ idle                             │
│    ↓                                            │
│    tester    ○ idle                             │
│    ↑___________________________________________ │
└────────────────────────────────────────────────┘
```
Le détenteur du jeton = l'agent dont le nom == `current_agent` : marqueur `▸` + `theme::selection()` (laiton gras) + suffixe `← holds token`. Flèches `↓` entre agents, `↑` de bouclage en fin (ASCII : `v` / `^` via `glyphs()`). Si `current_agent` est `None`, aucun agent n'est mis en évidence.

En **veille** (35 cols) les trois patterns retombent sur la **liste compacte** actuelle, avec le seul en-tête `· <Pattern>`.

### Drill-down ⌃W (déjà implémenté — styliser seulement)
Aucune nouvelle logique. `Ctrl+W` focus (+ élargissement + rendu riche), `j/k` navigation, `Enter` popup de détail, `Esc` sortie. Le **popup** (`shell/tui.rs`) est stylisé DS : bordure `theme::border_style()`, titre `theme::heading()`, corps en couleurs nommées adaptatives. La ligne de hints du bas de la Workroom utilise `theme::glyphs()`.

## Nettoyage des couleurs codées en dur

Objectif : **zéro `Color::` littéral** dans `src/tui/views/`, `src/tui/widgets/` et `src/shell/workroom.rs` (hors `src/theme.rs`).

| Emplacement | Avant | Après |
|---|---|---|
| `src/tui/views/palette.rs` prompt `:` (l.30) | `Color::Cyan` | `theme::heading()` |
| `src/tui/views/palette.rs` bordures (l.39, 70) | `.border_style(fg(Color::Cyan))` | supprimé — le `.style(theme::border_style())` déjà présent suffit |
| `src/tui/views/agent_detail.rs` scope (l.109) | `Color::Cyan` | `theme::tag()` |
| `src/tui/views/agent_detail.rs` pattern (l.145) | `Color::Magenta` | `theme::heading()` |
| `src/tui/views/agent_detail.rs` parts (l.166) | `Color::Yellow` | `theme::stack()` |
| `src/tui/views/agent_detail.rs` parts (l.180) | `Color::Blue` | `theme::working()` |
| `src/shell/workroom.rs` bordure (l.582) | `Color::Rgb(48,54,61)` | `theme::border_style()` |

Note : les `Color::DarkGray`/`Color::Gray`/`Color::Green` déjà présents et cohérents (muted/résolution modèle) peuvent rester tels quels s'ils correspondent déjà à un helper thème équivalent ; l'objectif ferme porte sur Cyan/Magenta/Blue/Yellow/Rgb qui cassaient la cohérence ou la lisibilité fond clair. Vérifier par `grep -n "Color::" src/tui/views src/tui/widgets src/shell/workroom.rs` en fin de T3c : ne doivent subsister que des couleurs nommées adaptatives justifiées.

## Découpage (une PR + revue indépendante + validation visuelle par sous-lot)

- **T3a — Plumbing + hierarchical** : champ `Workroom.pattern` + parsing dans `init_from_config` ; dispatch `render` sur `(pattern, focused, width)` + dégradation ; élargissement du panneau au focus côté shell ; **refonte du layout hierarchical** (connecteurs box-drawing) comme pilote bout-en-bout. Prouve le mécanisme sur une vraie vue.
- **T3b — Blackboard + Ring** : `render_blackboard` (ligne board + agents à plat) et `render_ring` (anneau + détenteur de jeton + flèches).
- **T3c — Drill-down stylé + nettoyage couleurs** : styling DS du popup de détail + tableau de résorption ci-dessus (zéro `Color::` littéral non justifié).

## Tests & CI

- **Unitaires** (feature `tui`, sérialiser tout accès env comme pour le fix flaky) :
  - `init_from_config` détecte `hierarchical`/`blackboard`/`ring` et retombe sur `Hierarchical` par défaut/valeur inconnue.
  - Sélection du layout par `(pattern, focused, width)` : focus + largeur ≥ 44 → layout du pattern ; focus + largeur < 44 → compact ; non focus → compact.
  - Ring : l'agent dont le nom == `current_agent` porte `theme::selection()` (assertion sur `.fg`/modifier, pas sur une valeur tier-dépendante).
- **Rendu visuel** : validation manuelle Dimitri via `armadai shell` (orchestration réelle par pattern) + test de `--ascii`. Pas de snapshot pixel.
- **Gate** : clippy **3 modes** (`tui`, `tui,providers-api`, `tui,web,storage`) `-D warnings` + `cargo fmt -- --check` + `cargo test`.

## Hors périmètre
- L'onglet Orchestration du `armadai tui` (vue storage des runs passés — déjà stylisé en #239) n'est PAS concerné : T3 porte sur la Workroom du **shell**.
- Sortie CLI humaine (`armadai run`) et Docs/README — surfaces **suivantes** du chantier (après le TUI).
- Le glyphe `▤` (board) et `▤`/flèches doivent avoir leur équivalent ASCII : les ajouter au set `Glyphs` de `src/theme.rs` si absents (`board`, `arrow_down`, `arrow_up`, connecteurs d'arbre) — extension locale, pas un nouveau module.

## Risques
- **Détection de largeur** : `area.width` inclut les bordures ; caler le seuil (44) sur la largeur *interne* utile. Valider visuellement la bascule veille↔focus.
- **Élargissement au focus** : rétrécit la zone conversation ; vérifier que le retour en veille restaure la largeur et ne casse pas le reflow du markdown.
- **Glyphes unicode** : certains terminaux/tmux rendent mal `▤`/`⚑` ; le set ASCII (`--ascii`) est le filet ; garder des équivalents lisibles.
- **Parsing tolérant** : `init_from_config` reste heuristique (pas de désérialisation stricte) pour survivre aux configs partielles — cohérent avec l'existant.
