# Étape 4, lot B1 — commandes et prises : plan d'implémentation

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** ajouter au modèle canonique les commandes (`.armadai/commands/<id>.md`) et les prises (`slots:` d'agent, `metadata.armadai-slot` de skill), compilées vers Claude Code et OpenCode 2.x.

**Architecture:** le modèle (`crates/armadai-model`) porte les types, le chargement, la validation et deux fonctions pures de résolution des prises ; les adaptateurs (`crates/armadai/src/compile/{claude,opencode,legacy_targets}.rs`) les consomment ; le manifeste de link gagne un genre `Command` ; la CLI (`new`, `list`, `inspect`) et l'installation de pack natif connaissent les commandes.

**Tech Stack:** Rust 2024, serde + serde-saphyr, schemars, serde_json.

**Spec:** `docs/superpowers/specs/2026-10-21-step4b1-commands-design.md`

## Global Constraints

- `crates/armadai-model` reste pur : serde, serde-saphyr, serde_json, thiserror, schemars seulement.
- Seul `$ARGUMENTS` est un placeholder accepté ; `$<chiffre>` et `$ARGUMENTS[` sont une erreur ; `` !` `` et une ligne commençant par ` ```! ` sont une erreur.
- Une commande tourne dans le fil principal : aucune clé `agent`, `context`, `subagent` générée (seulement via `native`).
- Claude : `.claude/commands/<id>.md` ; OpenCode : `.opencode/commands/<id>.md` (sous le dossier de sortie OpenCode réel) ; Codex, Copilot : `not-compiled` avec la raison `no command support yet`.
- Une prise vide n'est jamais une erreur : ligne `not-compiled` avec la raison `no skill fills it`.
- Ordre des skills d'un agent : d'abord `skills:` dans l'ordre déclaré, puis les skills de ses prises triées par id, sans doublon.
- Tests à égalité exacte ; une mutation vue rouge par test ajouté (restaurer puis `touch` le fichier) ; `crates/armadai` est binaire seul (`--bin armadai` / `--test <nom>`), toujours `--no-fail-fast`.
- Barrière : `cargo fmt --all -- --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --no-fail-fast`, `cargo build -p armadai-core`, `cargo build -p armadai-model`, `(cd frozen/armadai-audit && cargo check --all-targets)`.
- Commits Conventional Commits en anglais, un type par sujet.

## Review Focus

1. Un `$5` dans du texte ordinaire (« coûte $5 ») : refusé, car les deux cibles le remplaceraient — le message doit dire de reformuler (test en tâche 2).
2. Un sous-dossier `commands/a/b.md` : ignoré avec un avertissement (Claude en ferait `/a:b`, OpenCode `/a/b`) — test en tâche 1.
3. Une skill qui remplit une prise qu'aucun agent n'utilise : aucun diagnostic (le pack de normes peut être installé avant l'équipe) — test en tâche 2.
4. Un `.claude/commands/<id>.md` écrit à la main avant `link` : refusé comme tout fichier existant non écrit par link, rien d'écrasé — test en tâche 3.
5. `link --agents x` : les commandes sont écrites quand même (elles ne dépendent d'aucun agent) — test en tâche 3.

---

### Task 1 : types, chargement, écriture et schémas

**Files:**
- Modify: `crates/armadai-model/src/types.rs` (nouveau `CommandFront`, champ `slots` d'`AgentFront`)
- Modify: `crates/armadai-model/src/load.rs` (`Command`, `Project.commands`, `Kind::Command`, lecture de `commands/`)
- Modify: `crates/armadai-model/src/write.rs` (`render_command`, `write_project`)
- Modify: `crates/armadai-model/src/schema.rs` + régénérer `schemas/v1/` (`cargo run -p armadai-model --example gen_schemas`)
- Modify: tout constructeur littéral de `Project` / `AgentFront` qui ne compile plus (grep)

**Interfaces:**
- Produces :
  - `pub struct CommandFront { pub description: String, pub argument_hint: Option<String>, pub native: Native }` (`deny_unknown_fields`, `argument_hint` et `native` omis à l'écriture quand vides)
  - `pub struct Command { pub front: CommandFront, pub body: String }`
  - `Project.commands: BTreeMap<String, Command>` ; `Kind::Command`
  - `AgentFront.slots: Vec<String>` (`#[serde(default, skip_serializing_if = "Vec::is_empty")]`, placé après `skills`)
  - `pub fn render_command(c: &Command) -> String` ; `write_project` écrit `commands/<id>.md`
  - schéma `command.schema.json`

- [ ] **Step 1 :** tests dans `load.rs` : `commands_are_loaded` (un `commands/brief.md` `---\ndescription: Refine an idea\nargument_hint: <idea>\n---\nRefine: $ARGUMENTS\n` → `project.commands["brief"]` avec ces valeurs et `body == "Refine: $ARGUMENTS\n"`) ; `unknown_command_key_is_an_error` (clé `agent: x` → diagnostic d'erreur de clé inconnue, comme pour un agent) ; `nested_command_folder_is_ignored` (`commands/a/b.md` → avertissement « ignored » sur le dossier `commands/a`, aucune commande) ; `agent_slots_are_loaded` (`slots: [code-standards]`).
- [ ] **Step 2 :** test d'aller-retour dans `write.rs` : `render_command` d'une commande chargée relue à l'identique ; `write_project` sur un projet avec une commande écrit `.armadai/commands/brief.md`.
- [ ] **Step 3 :** les voir rouges, implémenter (calquer la boucle `agents` du loader : fichiers `.md` seulement, dossier → `cx.ignored`), régénérer les schémas, voir vert.
- [ ] **Step 4 :** barrière ; commit `feat(model): commands and agent slots`.

### Task 2 : validation et résolution des prises

**Files:**
- Modify: `crates/armadai-model/src/check.rs`
- Create: `crates/armadai-model/src/slots.rs` (exporté par `lib.rs`)

**Interfaces:**
- Consumes : tâche 1.
- Produces :
  - `pub fn slot_fills(project: &Project, slot: &str) -> Vec<String>` — ids des skills dont `metadata["armadai-slot"]`, découpé sur `,` et rogné, contient `slot` ; triés.
  - `pub fn agent_skills(project: &Project, agent: &str) -> Vec<String>` — `skills:` dans l'ordre, puis `slot_fills` de chaque prise de l'agent dans l'ordre des prises, sans doublon (première occurrence gardée).
  - codes de diagnostic (nouveaux, stables) : `command-empty-description`, `command-empty-body`, `command-positional-argument`, `command-shell-expansion`, `command-shadows-skill` (erreur), `command-shadows-builtin` (avertissement), `bad-slot` (erreur : valeur `armadai-slot` vide, ou un élément qui n'est pas un id valide), et `bad-id` réutilisé pour un id de commande et pour un nom de `slots:`.
  - `pub const BUILTIN_COMMANDS: &[&str]` dans `check.rs`, commentée avec ses sources (`code.claude.com/docs/en/commands.md` relevée le 2026-10-21, `opencode packages/web/src/content/docs/tui.mdx` au commit 681156c8), triée, avec exactement ces noms :
    `add-dir adddir advisor agents allowed-tools android app artifacts autocompact autofix-pr background bashes batch bg branch btw bug cd checkpoint checkup chrome claude-api claude-in-chrome clear code-review color compact config connect context continue copy cost debug deep-research design desktop details diff doctor editor effort exit export extra-usage fast feedback fewer-permission-prompts focus fork goal heapdump help hooks ide import init insights install-github-app install-slack-app ios keybindings list-agents login logout loop mcp memory mobile model models new output-style passes peers permissions plan plugin powerup pr-comments privacy-settings proactive q quit radio rate-limit-options rc recap redo release-notes reload-plugins reload-skills remote-control remote-env rename reset resume review rewind routines run sandbox schedule scroll-speed security-review sessions settings setup-bedrock setup-vertex share simplify skill-doctor skills stats status statusline stickers stop subtask summarize tasks team-onboarding teleport terminal-setup theme themes thinking tp tui ultraplan ultrareview undo update-config upgrade usage usage-credits verify vim voice web-setup workflows`

- [ ] **Step 1 :** tests à message exact pour chaque code : description vide, corps vide (espaces seuls), `$1`, `$ARGUMENTS[0]`, `coûte $5` (même code `command-positional-argument`, message qui dit que `$<chiffre>` est remplacé par les deux cibles et de reformuler), `` !`git diff` ``, une ligne ` ```! `, id `Bad_Id`, commande `lint` + skill `lint` → `command-shadows-skill`, commande `review` → avertissement `command-shadows-builtin`, `slots: [Bad Slot]` → `bad-id`, `armadai-slot: ""` → `bad-slot`, `armadai-slot: "a, b"` → valide.
- [ ] **Step 2 :** tests de `slots.rs` : `slot_fills` (deux skills `b-std`, `a-std` remplissent `code-standards` → `["a-std", "b-std"]` ; une skill `x` avec `armadai-slot: "code-standards, security"` remplit les deux) ; `agent_skills` (`skills: [z, a-std]`, `slots: [code-standards]` → `["z", "a-std", "b-std"]`) ; `unused_slot_is_not_a_diagnostic` (Review Focus 3).
- [ ] **Step 3 :** rouge, implémenter, vert ; barrière ; commit `feat(model): validate commands and resolve slots`.

### Task 3 : adaptateur Claude, cibles héritées, manifeste et packs

**Files:**
- Modify: `crates/armadai/src/compile/claude.rs`, `crates/armadai/src/compile/legacy_targets.rs`
- Modify: `crates/armadai/src/linker/manifest.rs` (`ProducedByKind::Command`, `ProducedBy::command`)
- Modify: `crates/armadai/src/cli/native_pack.rs` (liste blanche : `["commands", f] => f.ends_with(".md")`) et tout autre chemin d'installation de pack qui filtre les dossiers (grep `"automations"`)
- Test: tests unitaires des adaptateurs + `crates/armadai/tests/` (nouveau fichier `link_commands.rs`)

**Interfaces:**
- Consumes : `agent_skills`, `slot_fills` (tâche 2), `Project.commands`.
- Produces : artefact `commands/<id>.md` sous la racine Claude, `produced_by: ProducedBy::command(id)` ; lignes de rapport `command <id>` et `agent <id> slot <s>`.

- [ ] **Step 1 :** tests Claude (égalité exacte du fichier) :
  - `brief` → `.claude/commands/brief.md` = `---\ndescription: Refine an idea\nargument-hint: <idea>\n---\nRefine: $ARGUMENTS\n` (clé renommée `argument-hint`) ; ligne `command brief: compiled`.
  - `native.claude: { allowed-tools: Read }` ajouté au frontmatter ; `native.claude: { description: x }` → erreur `command `brief`: native.claude key `description` collides with a generated key`.
  - un agent `dev` avec `slots: [code-standards]`, skills `a-std`, `b-std` qui la remplissent → frontmatter `skills: [a-std, b-std]` ; ligne `agent dev slot code-standards: compiled (a-std, b-std)` ; prise vide → `agent dev slot security: not-compiled (no skill fills it)`.
  - le coordinateur avec `slots:` → ligne `approximated` avec la même raison que `skills:` du coordinateur aujourd'hui.
- [ ] **Step 2 :** tests cibles héritées : pour Codex et Copilot, `command brief: not-compiled (no command support yet)` ; leurs fichiers générés inchangés (étalons existants verts sans régénération) ; une prise d'agent y reçoit la même ligne que les `skills:` de l'agent reçoivent aujourd'hui sur ces cibles, calculée sur `agent_skills`.
- [ ] **Step 3 :** tests d'intégration `link_commands.rs` : link puis unlink vers `claude` revient à l'arbre initial (comparaison d'arbre complet) ; un `.claude/commands/brief.md` écrit à la main avant link → link échoue sans rien écrire (Review Focus 4, message exact de la garde existante) ; `link --agents dev` écrit quand même `.claude/commands/brief.md` (Review Focus 5) ; `init --pack` d'un pack de test contenant `commands/brief.md` l'installe.
- [ ] **Step 4 :** rouge, implémenter, vert ; barrière ; commits `feat(link): compile commands and slots for Claude Code` et `feat(init): install the commands of a native pack`.

### Task 4 : adaptateur OpenCode

**Files:**
- Modify: `crates/armadai/src/compile/opencode.rs`
- Test: tests unitaires + `crates/armadai/tests/link_commands.rs`

**Interfaces:**
- Consumes : tâches 2 et 3 (`ProducedBy::command`).

- [ ] **Step 1 :** tests :
  - `brief` → `<sortie>/commands/brief.md` = `---\ndescription: Refine an idea\n---\nRefine: $ARGUMENTS\n` ; ligne `command brief: approximated (argument_hint has no OpenCode equivalent)` quand `argument_hint` est présent, `compiled` sinon.
  - `native.opencode` ajouté au frontmatter, collision → erreur au même format qu'en tâche 3.
  - agent `dev` avec `slots: [code-standards]` → `permission.skill` = `"*": deny`, `a-std: allow`, `b-std: allow` et la consigne « load these skills » liste `a-std`, `b-std` ; un agent sans `skills:` ni prise remplie → aucune clé `permission.skill` (décision de Dimitri : il garde toutes les skills).
  - avec `--output oc`, le fichier va dans `oc/commands/brief.md`.
- [ ] **Step 2 :** intégration : link puis unlink vers `opencode` revient à l'arbre initial.
- [ ] **Step 3 :** rouge, implémenter, vert ; barrière ; commit `feat(link): compile commands and slots for OpenCode`.

### Task 5 : CLI (`new command`, `list`, `inspect`)

**Files:**
- Modify: `crates/armadai/src/cli/mod.rs`, `new.rs`, `new_target.rs`, `new_wizard.rs`, `list.rs`, `inspect.rs`
- Test: tests unitaires de ces modules

**Interfaces:**
- Consumes : tâche 1 (`render_command`), tâche 2 (`slot_fills`, validation).

- [ ] **Step 1 :** tests :
  - `armadai new command brief --description "Refine an idea"` écrit `.armadai/commands/brief.md` = `---\ndescription: Refine an idea\n---\n$ARGUMENTS\n` ; refuse un id existant, un id invalide, un id égal à une skill (message du code `command-shadows-skill`) ; annule l'écriture si le projet devient invalide (même garde que `new skill`).
  - le menu guidé (`new` sans argument) propose `command` ; la fonction pure `plan_*` correspondante a son test.
  - `list` affiche une section `Commands` ; `inspect` d'un agent avec prises affiche chaque prise et ses skills (`code-standards: a-std, b-std`, prise vide : `security: (none)`).
- [ ] **Step 2 :** rouge, implémenter, vert ; barrière ; commit `feat(new): create a command; list and inspect show commands and slots`.

### Task 6 : étalons, documentation

**Files:**
- Create: `crates/armadai/tests/fixtures/commands-slots/` (projet : 2 agents dont un coordinateur, `dev` avec `slots: [code-standards, security]`, skills `a-std` et `b-std` qui remplissent `code-standards`, commandes `brief` (avec `argument_hint`) et `mockup` (avec `native.claude`)) et son étalon pour les 4 cibles
- Modify: `docs/wiki/project-format.md`, `docs/wiki/link.md`, `CHANGELOG.md` (`## Unreleased`, `### Added`)

- [ ] **Step 1 :** étalon par cible (même harnais que `link_golden.rs`, `UPDATE_GOLDEN` à la même règle) ; relire chaque fichier généré.
- [ ] **Step 2 :** doc : format des commandes et des prises, les règles de validation et leurs codes, la table de correspondance de la spec §5, la limite « une prise ne contient que des skills », le fait que `.claude/commands/` est le format hérité documenté par Claude Code.
- [ ] **Step 3 :** barrière ; commits `test(link): golden outputs for commands and slots` et `docs: commands and slots`.

### Task 7 : essai réel (préparé par le contrôleur, exécuté par Dimitri)

Pas d'implémentation : après la revue finale, préparer un projet d'essai lié vers Claude et OpenCode, et la liste d'essais du critère 4 de la spec, donnés un par un à Dimitri.
