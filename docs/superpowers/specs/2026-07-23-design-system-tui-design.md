# Design System → TUI (ratatui) — Design

## Contexte

2ᵉ surface du chantier design system « pont de commandement » (Web ✅ refait en Svelte). Le TUI (`src/tui/`, ratatui/crossterm, feature `tui`) sert `armadai tui` (dashboard) et `armadai shell`. Aujourd'hui : ~15 vues + widgets, couleurs `Color::` **ANSI-16 nommées éparpillées**, aucun module thème, pas de laiton/chart-blue. Le DS fournit `assets/terminal-palette.json` (mapping truecolor→256→16) + 2 écrans UX (`ui_kits/tui/{dashboard-ux,shell-ux}.html`).

## Objectif

Un module thème central ratatui portant l'identité DS (laiton + chart-blue + pavillons de signal), avec dégradation propre truecolor→256→16 et un jeu de glyphes ASCII de secours, puis re-skin des vues, de la Workroom et de la palette.

## Architecture

### Module thème `src/tui/theme.rs`
- **Tokens** (valeurs de `terminal-palette.json`), chacun portant (truecolor hex, xterm256, ansi16) :
  - `brass #c79a4a`/179/yellow, `brass_strong #d6ad5f`/215/bright-yellow
  - `signal_ok #5cbf87`/114/green, `signal_warning #e2b24c`/214/bright-yellow, `signal_critical #d75f4d`/167/red, `signal_running #57a9cc`/74/cyan, `signal_halted #8a929b`/245/bright-black
  - `text_primary #f1f4f6`/255/white, `text_secondary #c2c9cf`/251/white, `text_muted #9aa4ad`/245/bright-black
  - `surface_bg #0f1b2d`/234/black, `surface_panel #1c2b40`/236/black, `border #3a4a60`/239/bright-black
- **Niveau de couleur** `enum ColorTier { Truecolor, Xterm256, Ansi16 }`, détecté une fois à l'init :
  - `Truecolor` si `COLORTERM` ∈ {`truecolor`,`24bit`} ; sinon `Xterm256` si `TERM` contient `256color` ; sinon `Ansi16`.
  - `NO_COLOR` défini → forcer `Ansi16` (couleurs nommées neutres, pas d'accent agressif).
- **Résolution** : `Theme::color(token) -> ratatui::style::Color` selon le tier (`Rgb` | `Indexed` | nommée). Helpers de style : `accent()`, `panel_border()`, `signal(kind)`, `selected()`, `muted()` — les vues ne manipulent plus de `Color::` brut.
- **Glyphes** : `struct Glyphs` avec deux jeux (unicode par défaut : bordures box-drawing, pavillons ◆●▚ ; ASCII de secours) sélectionnés par un booléen `ascii` porté par le thème.
- **Portée** : un `Theme` construit à l'init (tier + ascii) et passé/accessible aux vues (via l'état de l'app `App`). Aucune vue ne lit l'environnement elle-même.

### Flag `--ascii`
- Flag CLI **uniquement** (décision Dimitri) sur `armadai tui` et `armadai shell` (`src/cli/mod.rs`) → passé à l'init du `Theme` (`Glyphs` ASCII).

## Découpage (une PR + revue indépendante + validation visuelle par sous-lot)
- **T1 — Fondation thème** : `theme.rs` (tokens + `ColorTier` + détection + helpers + `Glyphs`/`--ascii`) + re-skin de la vue **dashboard** (master-detail) comme pilote de bout en bout. Prouve le thème sur une vraie vue.
- **T2 — Re-skin des vues restantes** : toutes les autres vues (`agent_detail`, `costs`, `history` dense, `models_list`/`model_detail`, `orchestration`, `prompts_list`/`prompt_detail`, `skills_list`/`skill_detail`, `starters_list`/`starter_detail`, `shortcuts`) + widgets (`agent_list`, `cost_chart`, `log_viewer`, `search_bar`) → passent par `theme`.
- **T3 — Workroom + interactions** : Workroom adaptative au pattern (arbre hiérarchique 3A par défaut, dégradant en liste ; colonnes blackboard / ring en extension), drill-down popup **⌃W**, styling de la palette de commandes **⌃P**.

## Fidélité
Caler sur les écrans DS `ui_kits/tui/{dashboard-ux,shell-ux}.html` : accent **laiton** sur l'actif/la sélection ; couleurs de **signal** pour les statuts (running/ok/halted/warning/critical) ; surfaces chart-blue en fond là où le terminal le permet (sinon fond par défaut) ; bordures box-drawing (→ ASCII sous `--ascii`).

## Validation
Le TUI étant terminal, **pas d'Artifact** : Dimitri valide en lançant `armadai tui` et `armadai shell` en local (par sous-lot, avant merge). Tester aussi `--ascii` et un terminal 256/16 (`COLORTERM=` vide) pour la dégradation.

## Tests & CI
- `theme.rs` **testable unitairement** : mapping token→couleur pour chaque `ColorTier`, sélection du tier depuis les variables d'env simulées, jeux de glyphes unicode vs ASCII. (Attention aux tests touchant l'env : les sérialiser via le mécanisme `ENV_MUTEX` ou éviter la lecture d'env globale — cf. le fix flaky récent.)
- Rendu des vues : validation manuelle (pas de snapshot pixel).
- Gate : clippy **3 modes** (`tui`, `tui,providers-api`, `tui,web,storage`) `-D warnings` + `cargo fmt -- --check` + `cargo test`. Feature `tui`.

## Hors périmètre
Sortie CLI humaine (`armadai run`) et docs/README — surfaces suivantes du chantier (après le TUI).

## Risques
- **Détection de capacité couleur** imparfaite selon les terminaux/multiplexeurs (tmux) → `--ascii` + dégradation `Ansi16` comme filets de sécurité ; ne pas surcharger, rester lisible dans tous les tiers.
- **Volume T2** (beaucoup de vues) → découper l'exécution en tâches par groupe de vues dans le plan.
- **Fonds chart-blue** : appliquer un `bg` plein sur toute la surface d'un terminal peut mal rendre selon le thème du terminal de l'utilisateur → privilégier accents/bordures/texte, fond plein avec parcimonie (à valider visuellement).
