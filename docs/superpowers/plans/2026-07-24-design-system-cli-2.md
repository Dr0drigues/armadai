# Design System → CLI, lot CLI-2 (toutes les commandes restantes) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Appliquer le module de style CLI (`src/cli/style.rs`, livré en CLI-1) à la sortie humaine de toutes les commandes `src/cli/` restantes, de façon cohérente et accent-only.

**Architecture:** Sweep mécanique guidé par un pattern déjà établi et exemplifié (CLI-1 : `src/cli/run.rs` + `src/cli/style.rs`). Chaque commande : identifier ses `println!`/`eprintln!` HUMAINS, les convertir en `anstream::println!`/`anstream::eprintln!` avec le helper sémantique adéquat ; laisser intactes les sorties machine (`--json`, `sink.emit`, valeurs scriptables destinées au pipe) et les logs `tracing::`. Découpé en 4 tâches par groupe de commandes.

**Tech Stack:** Rust edition 2024, `anstyle` + `anstream` (déjà en deps depuis CLI-1).

## Global Constraints
- Gate à chaque tâche : clippy **3 modes** (`--no-default-features --features tui`, `--features tui,providers-api`, `--features tui,web,storage`) `-D warnings` + `cargo fmt -- --check` + `cargo test --no-default-features --features tui`. En fin de lot (dernière tâche) : `cargo test --test e2e --no-default-features --features tui,storage`.
- **Ne styliser QUE la sortie HUMAINE.** NE PAS toucher : le contenu `--json`, les `sink.emit(...)`, les **valeurs brutes scriptables** (listes de noms destinées au pipe — ex. `list` sans filtre décoratif, `extract`), ni les `tracing::` logs.
- **Accent-only** : le CONTENU / les données restent en couleur par défaut du terminal (lisible fond clair ET sombre) ; on n'accentue que titres, labels, statuts, secondaire.
- **API** : `crate::cli::style::{header, accent, ok, warn, err, running, muted, agent} -> anstyle::Style`. Rendu inline `{s}texte{s:#}` (le `#` réinitialise), via `anstream::println!`/`anstream::eprintln!` **fully-qualified** (pas de clash avec les macros std). Détection couleur déléguée à `anstream` (ne rien réimplémenter).
- **Mapping sémantique** :
  - titres / en-têtes de section / colonnes → `header()`
  - noms d'entités (agent, prompt, skill, starter, modèle, projet) → `accent()` (ou `agent()` pour un nom d'agent en gras)
  - succès / « created » / « linked » / « done » / « ✓ » → `ok()`
  - avertissements / hints → `warn()` (ton alerte) ou `muted()` (ton discret) selon le cas
  - erreurs / échecs → `err()`
  - secondaire (chemins, compteurs, métadonnées, séparateurs, « N found ») → `muted()`
- **dead_code** : dans `src/cli/style.rs`, retirer le `#[allow(dead_code)]` sur `err()` et `agent()` (l.40/53 zone) dès qu'ils sont utilisés (ils le seront dans ce lot). Vérifier au clippy.
- Branche `feat/cli-ds-2` (déjà créée depuis `release/1.0.0`, contient `style.rs`). Une PR, revue indépendante + validation visuelle Dimitri.
- **rust-analyzer non fiable** (ABI/unlinked/inactive-cfg/snapshots) → vérifier au compilateur.

## Worked example (template à suivre pour toutes les commandes) — `src/cli/list.rs`

État actuel (extrait) et conversion cible :

```rust
// "No agents found." / hints → muted (secondaire, non-erreur)
let m = crate::cli::style::muted();
anstream::println!("{m}No agents found.{m:#}");
anstream::println!("{m}Create one with: armadai new --template basic <name>{m:#}");

// En-tête de colonnes → header
let h = crate::cli::style::header();
anstream::println!(
    "{h}  {:<name_w$}  {:<provider_w$}  {:<model_w$}  TAGS  STACKS{h:#}",
    "NAME", "PROVIDER", "MODEL",
);
// Ligne de séparation → muted
let m = crate::cli::style::muted();
anstream::println!(
    "{m}  {:<name_w$}  {:<provider_w$}  {:<model_w$}  ----  ------{m:#}",
    "-".repeat(name_w), "-".repeat(provider_w), "-".repeat(model_w),
);

// Lignes de données : le NOM d'agent accentué, le RESTE en couleur par défaut (données)
let a = crate::cli::style::accent();
anstream::println!(
    "  {a}{:<name_w$}{a:#}  {:<provider_w$}  {:<model_w$}  {}  {}",
    agent.name, agent.metadata.provider, agent.model_display(), tags_str, stacks_str,
);

// Compteur final → muted
let m = crate::cli::style::muted();
anstream::println!("\n{m}  {} agent(s) found.{m:#}", agents.len());

// Les `eprintln!("  warn: {err}")` de load_agents → warn()
let w = crate::cli::style::warn();
anstream::eprintln!("{w}  warn: {err}{w:#}");
```

Points clés démontrés : en-têtes/colonnes = `header` ; noms = `accent` (données autour = défaut) ; compteurs/hints/séparateurs = `muted` ; `warn:` = `warn`. **Aucune valeur brute n'est perdue** (les données restent lisibles/parsables ; seuls les accents décoratifs sont ajoutés autour). Appliquer ce même raisonnement à chaque commande de chaque tâche.

---

### Task 1: Discovery / read — `list`, `models`, `inspect`, `prompts`, `validate`

**Files:** Modify `src/cli/{list.rs, models.rs, inspect.rs, prompts.rs, validate.rs}` ; potentiellement `src/cli/style.rs` (retrait `allow(dead_code)` si `err`/`agent` utilisés ici).

**Interfaces:**
- Consumes: `crate::cli::style::*` + `anstream::{println,eprintln}` (CLI-1).

- [ ] **Step 1 : `list.rs`** — appliquer le Worked example ci-dessus verbatim (en-têtes `header`, noms `accent`, séparateur/compteur/hints `muted`, `warn:` `warn`). NE PAS colorer les colonnes de données (provider/model/tags/stacks).
- [ ] **Step 2 : `models.rs`** — lire le fichier ; en-têtes/sections de la liste de modèles → `header` ; nom de modèle/provider → `accent` ; coût/contexte/métadonnées → `muted` ; findings de dépréciation (`print_findings`) : le libellé d'alerte → `warn`, la cible de remplacement → `ok`/`accent`, le reste `muted`. Erreurs de fetch → `err`.
- [ ] **Step 3 : `inspect.rs`** — vue détaillée d'un agent : les LABELS de champs (Name/Provider/Model/Tags/System Prompt…) → `header` ou `muted` (labels = muted, valeurs = défaut, titre principal = header) ; le nom d'agent en tête → `header`/`accent`. Le corps du system prompt reste non coloré. (33 prints — rester méthodique, un label/section à la fois.)
- [ ] **Step 4 : `prompts.rs`** — même logique que `list`/`inspect` pour la liste/détail de prompts (en-têtes `header`, noms `accent`, secondaire `muted`).
- [ ] **Step 5 : `validate.rs`** — sortie de validation : succès/« valid » → `ok` ; problèmes/erreurs → `err` ; avertissements → `warn` ; en-tête/nom du fichier validé → `header`/`accent`.
- [ ] **Step 6 : Vérif + Gate.**

Run:
```bash
grep -n "anstream::" src/cli/{list,models,inspect,prompts,validate}.rs   # sanity: uniquement chemins humains
cargo fmt --all
cargo clippy --all-targets --no-default-features --features tui -- -D warnings
cargo clippy --all-targets --no-default-features --features tui,providers-api -- -D warnings
cargo clippy --all-targets --no-default-features --features tui,web,storage -- -D warnings
cargo test --no-default-features --features tui 2>&1 | tail -5
```
Expected : `No issues found` (3 modes) + suite verte.

- [ ] **Step 7 : Commit**
```bash
git add src/cli/list.rs src/cli/models.rs src/cli/inspect.rs src/cli/prompts.rs src/cli/validate.rs src/cli/style.rs
git commit -m "feat(cli): design-system accents for discovery commands (list/models/inspect/prompts/validate)"
```

---

### Task 2: Setup projet — `new`, `init`, `link`, `unlink`, `setup`, `update`

**Files:** Modify `src/cli/{new.rs, init.rs, link.rs, unlink.rs, setup.rs, update.rs}`.

**Interfaces:** Consumes `crate::cli::style::*` + `anstream::{println,eprintln}`.

- [ ] **Step 1 : `new.rs`** — création d'agent : « Created agent <name> » / « ✓ » → `ok` (+ nom en `accent`) ; chemins écrits → `muted` ; prompts interactifs (dialoguer) NON concernés (ce n'est pas du `println!` stylable) ; hints de suite → `muted` ; erreurs → `err`.
- [ ] **Step 2 : `init.rs`** — init projet / starter pack : titres d'étape → `header` ; « created … »/« ready » → `ok` ; fichiers/chemins → `muted` ; avertissements → `warn` ; erreurs → `err`. (24 prints.)
- [ ] **Step 3 : `link.rs`** — génération de config par cible : « Linked N agent(s) … to '<target>' » → `ok` (nom de cible `accent`) ; « wrote <path> » → `muted` ; collisions/avertissements → `warn` ; erreurs → `err`. Attention : le contenu machine éventuel reste intact.
- [ ] **Step 4 : `unlink.rs`** — « removed <path> »/« unlinked » → `ok`/`muted` ; avertissements → `warn`.
- [ ] **Step 5 : `setup.rs`** — assistant de setup / complétion shell (`print_completion_hint`) : titres → `header` ; instructions/chemins → `muted` ; succès → `ok`. (26 prints.)
- [ ] **Step 6 : `update.rs`** — mise à jour de modèles/config : « updated … » → `ok` ; « nothing to update » → `muted` ; erreurs → `err`.
- [ ] **Step 7 : Vérif + Gate** (même bloc que Task 1 Step 6, sur les 6 fichiers de cette tâche).
- [ ] **Step 8 : Commit**
```bash
git add src/cli/new.rs src/cli/init.rs src/cli/link.rs src/cli/unlink.rs src/cli/setup.rs src/cli/update.rs
git commit -m "feat(cli): design-system accents for project-setup commands (new/init/link/unlink/setup/update)"
```

---

### Task 3: Registry / skills — `registry`, `skills`

**Files:** Modify `src/cli/{registry.rs, skills.rs}`.

**Interfaces:** Consumes `crate::cli::style::*` + `anstream::{println,eprintln}`.

- [ ] **Step 1 : `registry.rs`** (50 prints) — sync/search/convert du catalogue awesome-copilot : en-têtes de section / titres de résultats → `header` ; noms d'entrées/agents → `accent` ; « synced N »/« installed » → `ok` ; compteurs/URLs/chemins → `muted` ; hints (« registry may be outdated… ») → `warn`/`muted` ; erreurs → `err`. Les éventuelles sorties destinées au parsing restent brutes.
- [ ] **Step 2 : `skills.rs`** (46 prints) — sync/search/install de skills GitHub : même logique (titres `header`, noms de skills/repos `accent`, succès `ok`, secondaire `muted`, hints `warn`, erreurs `err`).
- [ ] **Step 3 : Vérif + Gate** (même bloc, sur registry.rs + skills.rs).
- [ ] **Step 4 : Commit**
```bash
git add src/cli/registry.rs src/cli/skills.rs
git commit -m "feat(cli): design-system accents for registry/skills commands"
```

---

### Task 4: Config / divers — `config`, `audit`, `extract`

**Files:** Modify `src/cli/{config.rs, audit.rs, extract.rs}`.

**Interfaces:** Consumes `crate::cli::style::*` + `anstream::{println,eprintln}`.

- [ ] **Step 1 : `config.rs`** (37 prints) — affichage/édition de config : clés/labels → `muted` ou `header` (titre de section = header, clés = muted, valeurs = défaut) ; « set »/« saved » → `ok` ; erreurs de validation → `err` ; hints → `warn`/`muted`.
- [ ] **Step 2 : `audit.rs`** (10 prints) — sortie de l'audit agentique : titres/sections → `header` ; findings selon sévérité → `err` (critique) / `warn` (avertissement) / `ok` (rien à signaler) ; compteurs/chemins → `muted`.
- [ ] **Step 3 : `extract.rs`** (`print_summary`, 2 prints) — résumé d'extraction : titre → `header`, chemin de sortie → `muted`, « extracted N » → `ok`. **Attention** : si `extract` imprime une liste de noms destinée au pipe, la laisser BRUTE.
- [ ] **Step 4 : Vérif finale + Gate complet (fin de lot)** :
```bash
grep -rn "anstream::" src/cli/ | wc -l    # cohérence globale
cargo fmt --all
cargo clippy --all-targets --no-default-features --features tui -- -D warnings
cargo clippy --all-targets --no-default-features --features tui,providers-api -- -D warnings
cargo clippy --all-targets --no-default-features --features tui,web,storage -- -D warnings
cargo test --no-default-features --features tui 2>&1 | tail -5
cargo test --test e2e --no-default-features --features tui,storage 2>&1 | tail -5
```
Expected : tout vert ; e2e inchangé (anstream strippe en non-TTY → octets identiques). Vérifier qu'il ne reste plus de `#[allow(dead_code)]` sur un helper de `style.rs` désormais utilisé.
- [ ] **Step 5 : Commit**
```bash
git add src/cli/config.rs src/cli/audit.rs src/cli/extract.rs src/cli/style.rs
git commit -m "feat(cli): design-system accents for config/audit/extract commands"
```

**➡️ Fin du lot CLI-2. PR + revue indépendante + validation visuelle Dimitri : lancer chaque commande en TTY (accents), et vérifier `| cat` / `NO_COLOR=1` → aucune couleur, `--json` inchangé.**

---

## Self-Review
- **Spec coverage** : le design (module `style.rs` + accent-only + anstream) est celui de la spec CLI-1 ; CLI-2 l'applique aux commandes restantes → couvert par Tasks 1-4 (les 16 commandes réparties, aucune oubliée : list/models/inspect/prompts/validate + new/init/link/unlink/setup/update + registry/skills + config/audit/extract). ✓
- **Placeholder scan** : pas de TODO ; c'est un sweep pattern-based **délibéré** (le pattern + un exemple entièrement travaillé `list.rs` + le mapping sémantique + la référence CLI-1 `run.rs` fournissent le « comment » ; pré-écrire les 331 prints verbatim serait irréaliste et fragile). Chaque Step nomme le fichier + les types de lignes → helper. Les cas ambigus (sorties scriptables) sont explicitement « rester brutes ».
- **Type consistency** : helpers `style::{header,accent,ok,warn,err,running,muted,agent}` (définis en CLI-1) consommés partout ; `anstream::println!`/`eprintln!` fully-qualified ; retrait `allow(dead_code)` sur `err`/`agent` cohérent (Task 1 ou plus tard selon premier usage — vérifié au gate). Aucune nouvelle API introduite.
- **Risque** : le CONTENU/données doit rester non coloré (accent-only) et les sorties scriptables intactes — rappelé dans les Global Constraints ET le worked example ET les Steps ambigus ; la revue de tâche + la validation visuelle Dimitri sont le filet.
