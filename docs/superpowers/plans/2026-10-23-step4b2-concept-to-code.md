# Étape 4, lot B2 — pack « concept-to-code » : plan d'implémentation

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** livrer le pack natif `concept-to-code` (équipe, modèle `checker`, flows `deliver`/`guide`, commandes, skills, prises) installable, valide et lié vers Claude et OpenCode.

**Architecture:** du contenu dans `crates/armadai-core/starters/concept-to-code/` (embarqué par `include_dir` comme les 7 autres packs) ; les listes de packs des tests d'étalons s'étendent au 8e ; aucun code de compilation ne change.

**Tech Stack:** fichiers du modèle canonique (`format: 1`), tests Rust existants.

**Spec:** `docs/superpowers/specs/2026-10-23-step4b2-concept-to-code-design.md`

## Global Constraints

- Arborescence : `pack.yaml` (`format: 1`, `name: concept-to-code`, description), `project.yaml` (`schema_version: 1`, `targets: [claude, opencode]`, `coordinator: lead`, `policy: { delegation: strict, free_agents: [explore] }`), `templates/checker.md`, `agents/*.md` (9), `flows/deliver.yaml`, `flows/guide.yaml`, `commands/*.md` (4), `skills/<id>/SKILL.md` (5).
- Ids exacts : agents `lead analyst scout designer architect implementer coach guardian validator` ; commandes `brief mockup iterate init-standards` ; skills `briefing cadre mockups diagrams standards-setup` (une commande ne peut pas porter le nom d'une skill : `/brief` charge `briefing`, `/mockup` et `/iterate` chargent `mockups`, `/init-standards` charge `standards-setup`).
- Prises : `code-standards`, `security`, `infra-docs`, `ux-guidelines`, déclarées comme dans la spec §4 ; aucune skill du pack ne les remplit.
- Prompts en anglais ; chaque agent s'adresse à l'utilisateur dans la langue de l'utilisateur ; tout sous-agent rend ses questions à `lead` au lieu de les poser ; rangement des documents de la spec §7 dans les prompts concernés ; aucune consigne de ligne de verdict (générée par `link`).
- `armadai validate` sur le pack installé : 0 erreur, 0 avertissement.
- Tests à égalité exacte ; une mutation vue rouge par test ajouté ou changé ; `crates/armadai` binaire seul (`--bin armadai` / `--test <nom>`), `--no-fail-fast`.
- Barrière : `cargo fmt --all -- --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --no-fail-fast`, `cargo build -p armadai-core`, `cargo build -p armadai-model`, `(cd frozen/armadai-audit && cargo check --all-targets)`.

## Review Focus

1. Un nom de commande ou de skill égal à une commande intégrée (`design`, `init`, `review`…) : `validate` doit rester sans avertissement — test en tâche 1.
2. `guardian`/`validator` rendus depuis `checker` : leur fichier Claude contient la consigne `STATE:` avec les bons états (`approved, rejected` ; `approved, rejected, scope_changed`) — vérifié dans les étalons en tâche 2.
3. Les deux flows partagent l'entrée `refine` et ne divergent qu'à `build` : graphe valide (aucune étape inatteignable) — test en tâche 1.
4. OpenCode : `strict` donne à chaque agent les cibles de délégation de ses liens ; `coach` et `implementer` n'ont pas de cible interdite — vérifié dans l'étalon OpenCode en tâche 2.
5. `init --pack concept-to-code --link` dans un dossier vide réussit et installe les 4 commandes et le modèle — test en tâche 1.

---

### Task 1 : le contenu du pack

**Files:**
- Create: tout `crates/armadai-core/starters/concept-to-code/` (Global Constraints)
- Modify: les listes de packs natifs des tests qui énumèrent les 7 packs (`crates/armadai/tests/init_pack_golden.rs` et toute autre liste trouvée par `grep -rn '"rust-dev"' crates`) — sans régénérer d'étalon dans cette tâche si ce n'est pas nécessaire à la barrière

- [ ] **Step 1 :** tests : le pack est listé par `armadai init --list` (ou équivalent existant) ; `init --pack concept-to-code --link` dans un dossier vide isolé réussit, écrit les 9 agents, le modèle, les 2 flows, les 4 commandes, les 5 skills, puis lie (`.claude/commands/{brief,mockup,iterate,init-standards}.md` présents) ; `armadai validate` rend 0 diagnostic (Review Focus 1, 3) ; `inspect guardian` montre `from: checker`.
- [ ] **Step 2 :** écrire le contenu (spec §4-§7 ; texte des prompts laissé au constructeur dans les limites des Global Constraints).
- [ ] **Step 3 :** vert ; barrière ; commit `feat(starters): the concept-to-code pack`.

### Task 2 : étalons et documentation

**Files:**
- Create: `crates/armadai/tests/golden/init_pack/concept-to-code.txt`, `crates/armadai/tests/golden/init_pack_opencode/concept-to-code.txt`
- Modify: `docs/wiki/starter-packs.md`, `CHANGELOG.md` (`## Unreleased`, `### Added`), `README.md` si elle liste les packs

- [ ] **Step 1 :** étalons générés (`UPDATE_GOLDEN=1`) et relus un par un ; vérifier Review Focus 2 et 4 dans le texte.
- [ ] **Step 2 :** doc : le pack, son équipe, ses deux flows, ses commandes, ses prises et comment les remplir (`/init-standards`, skill de l'utilisateur avec `armadai-slot`), le rangement des documents, la limite « l'écriture ne peut pas être restreinte à un dossier ».
- [ ] **Step 3 :** barrière ; commits `test(starters): golden outputs of the concept-to-code pack` et `docs: the concept-to-code pack`.

### Task 3 : essai réel (contrôleur)

Préparer un dépôt vide avec le pack installé et lié, puis donner à Dimitri les essais un par un (spec §9.3) : `/init-standards`, `/brief`, `/mockup` puis `/iterate`, le flow `deliver` jusqu'à un rejet de `guardian` et sa reprise, avec `armadai flow status --watch`.
