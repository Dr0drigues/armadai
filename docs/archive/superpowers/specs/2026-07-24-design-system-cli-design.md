# Design System → CLI (sortie humaine) — Design

## Contexte

3ᵉ surface du chantier design system « pont de commandement » (Web Svelte ✅, TUI T3a-d ✅ — cf. mémoire `project_design_system_rollout`). Cible : la **sortie humaine des commandes CLI** (`armadai run` en mode humain d'abord). Différence clé vs le TUI : c'est du **texte plat sur stdout/stderr** (pas ratatui) → couleurs **ANSI**, avec strip automatique quand ce n'est pas pertinent (pipe, CI, `NO_COLOR`, `--json`). Aujourd'hui la sortie humaine n'a **aucune couleur** et les `println!`/`eprintln!` sont éparpillés sur ~15 fichiers `src/cli/` (aucun helper central).

⚠️ Ne PAS confondre avec `src/theme.rs` (ratatui `Color`, pour le TUI/shell). Ce lot crée un module **séparé** pour la sortie CLI plate.

## Objectif

Un module de style CLI central, cohérent avec l'identité DS (laiton + signaux), appliqué d'abord à la sortie humaine de `armadai run`, avec gestion propre TTY / `NO_COLOR` / `--json`. Les autres commandes suivront dans des lots séparés.

## Décisions validées (Dimitri, 2026-07-24)

1. **Périmètre lot CLI-1** : module `src/cli/style.rs` (socle) + application à `armadai run` humain (+ résumé orchestration). Autres commandes = lots ultérieurs.
2. **Approche** : `anstyle` + `anstream` (tous deux déjà dans `Cargo.lock` en transitif ; à ajouter en `[dependencies]`). `anstream` strippe automatiquement les codes ANSI selon `NO_COLOR`/`CLICOLOR[_FORCE]`/stdout non-TTY.
3. **Accent-only** : le corps de texte reste en couleur par défaut du terminal (lisible fond clair ET sombre) ; on n'accentue que titres/actif (laiton), statuts (signaux ok/warn/err/running), secondaire (muted).
4. **Module NON gated `tui`** : la sortie CLI de base doit fonctionner sans la feature `tui`.

## Architecture

### Module `src/cli/style.rs` (nouveau, non feature-gated)

- **Dépendances** : `anstyle` (types `Style`/`AnsiColor`/`RgbColor`) + `anstream` (flux auto-strip). Ajoutées à `Cargo.toml [dependencies]` (déjà résolues dans le lock).
- **Palette DS (accents), valeurs de `assets/terminal-palette.json`** — exprimées en `anstyle` :
  - `brass #c79a4a`, `signal_ok #5cbf87`, `signal_warning #e2b24c`, `signal_critical #d75f4d`, `signal_running #57a9cc`, `muted` = gris (AnsiColor::BrightBlack). Utiliser `RgbColor` pour les accents (les terminaux modernes gèrent le truecolor ; anstream/NO_COLOR gèrent l'absence de couleur ; pas de dégradation tier 256/16 pour ce lot — YAGNI, à revoir si besoin).
- **API sémantique** (miroir de `theme.rs` pour la cohérence) — chaque helper renvoie un `anstyle::Style` :
  - `header()` = bold brass ; `accent()` = brass ; `ok()` = signal_ok ; `warn()` = signal_warning ; `err()` = signal_critical ; `running()` = signal_running ; `muted()` = bright-black ; `agent()` = bold (rôle/nom d'agent) .
- **Rendu** : les call-sites utilisent les macros `anstream::println!`/`eprintln!` (ou un flux `AutoStream`) avec les styles inline via `anstyle`'s `Style` (`{style}texte{style:#}`). anstream décide seul de stripper. Fournir de petits helpers ergonomiques si utile (ex. `fn paint(style: Style, s: &str) -> impl Display`), sans sur-abstraire.
- **`--json` / `--quiet`** : ces chemins n'émettent pas via les helpers humains (déjà séparés dans `run.rs` : le contenu `--json` passe par `EventSink`, jamais par ces `println!`). anstream strippe en plus si non-TTY. Donc pas de couleur en machine-output.

### Application à `armadai run` (humain)

Styler la sortie humaine (non-`--json`, non-`--quiet`) dans `src/cli/run.rs` :
- résultat final (`println!("{content}")`, l.~107/317/1393/1472/1573) : encadré/label discret en muted, contenu brut non coloré.
- en-têtes d'étape pipeline (`--- [i/n name] ---`, l.~329) : `header()`/accent.
- fallback modèle (l.~528), warnings : `warn()` ; erreurs : `err()`.
- résumé tokens/coût/durée (l.~565) : labels en muted, valeurs accentuées si pertinent.
- statuts d'orchestration humains (branches blackboard/ring/hierarchical, l.~1313-1573, déjà gated `human_output`) : `[blackboard] Starting…` → `running()`/accent ; `Halted`/status → `ok()`/`warn()`/`err()` selon l'issue ; en cohérence avec les états de la Workroom TUI.

Aucune modification du flux `RunEvent`/`EventSink` ni du chemin headless-machine.

## Hors périmètre
- Les autres commandes (`list`, `new`, `link`, `init`, `models`, `audit`, `validate`, `registry`, `skills`, `config`, `setup`, `inspect`…) — lots CLI-2+.
- Dégradation tier 256/16 côté CLI (truecolor + strip suffisent ici).
- Tableaux/box-drawing riches (rester en accents ANSI simples pour ce lot).

## Tests

- **`style.rs` (unitaire, déterministe, pas d'accès env réel)** :
  - chaque helper renvoie le `anstyle::Style` attendu (couleur/bold).
  - **rendu on/off** : rendre une chaîne stylée dans un flux forcé **plain** (ex. `anstream::StripStream` ou `AutoStream::never`) → **aucun** octet ANSI ; dans un flux forcé **couleur** (`AutoStream::always`) → les codes attendus présents. Assertion sur les octets.
- **`armadai run` humain** : pas de test visuel automatisé. Le flux `--json`/e2e est **intact** (couleurs seulement dans le chemin humain ; en CI/non-TTY anstream strippe → octets identiques). Vérifier que la suite + e2e restent verts.
- **Validation visuelle** : Dimitri lance `armadai run` en mode humain (démo fake-claude) — sur fond clair — et vérifie l'application des accents + l'absence de couleur en pipe (`| cat`) et avec `NO_COLOR=1`.
- **Gate** : clippy **3 modes** (`tui` / `tui,providers-api` / `tui,web,storage`) `-D warnings` + `cargo fmt -- --check` + `cargo test`.

## Risques
- **anstream/anstyle en `[dependencies]`** : déjà dans le lock (transitif via clap) → pas de nouveau poids notable ; confirmer les versions au moment de l'ajout.
- **Détection couleur** : déléguée à anstream (respecte `NO_COLOR`/`CLICOLOR`/TTY) — ne pas réimplémenter ; pour un flux non-stdout (ex. tests), utiliser `AutoStream::always/never` explicitement.
- **Cohérence d'identité** : garder les valeurs d'accent alignées sur `assets/terminal-palette.json` (mêmes couleurs que le TUI), même si le code est distinct de `theme.rs`.
