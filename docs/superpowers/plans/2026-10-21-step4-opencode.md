# Étape 4, lot A : adaptateur OpenCode 2.x fidèle — plan d'implémentation

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `armadai link --target opencode` produit une équipe que OpenCode 2.x lit réellement (agents, coordinateur par défaut, skills par agent, flows en guide, délégation contraignante sous `strict`), avec un rapport de conformité honnête et un `unlink` exact.

**Architecture:** un nouveau compilateur `compile/opencode.rs` (sur le modèle de `compile/claude.rs`) remplace `LegacyCompiler { target: "opencode" }`. Les fichiers entiers (agents, skills, fichier d'équipe) passent par les `Artifact` existants ; `opencode.json`, partagé avec l'utilisateur, passe par un nouveau patch de **clés possédées** (`compile/json_keys.rs` + `PatchKind::JsonKeys`), appliqué et retiré par `cli/link.rs` / `unlink` comme `settings.json` l'est pour Claude. La règle de délégation réutilise `claude_adapter::policy_gate::allowed_targets`.

**Tech Stack:** Rust 2024, `serde_json`, `armadai-model` (types canoniques), crates existants du workspace (aucune dépendance ajoutée).

**Spec:** `docs/superpowers/specs/2026-10-21-step4-opencode-design.md` (à lire avant toute tâche : §4.1 à §4.5 et le tableau §5 font foi).

## Global Constraints

- Cible : **OpenCode 2.x seulement** (2.1.2 chez Dimitri, branche `dev` d'`anomalyco/opencode`, doc `opencode.ai/docs`). Toute clé, tout nom d'outil se revérifie dans ce code avant d'être écrit.
- Frontmatter d'agent : **liste blanche** `description`, `mode`, `model`, `temperature`, `top_p`, `steps`, `disable`, `hidden`, `color`, `permission`, plus les clés de `native.opencode` de l'agent. Jamais `skills`, `tier`, `tags`, `title`, `capabilities` (une clé inconnue part chez le fournisseur).
- `model` : jamais écrit par défaut ; seulement `native.opencode.model`, sinon `model_tiers.opencode.<tier>` de `~/.config/armadai/config.yaml` ; format `fournisseur/modèle`.
- Permissions 2.x : clés `read`, `edit`, `shell`, `task`, `webfetch`, `websearch`, `skill` ; valeurs `allow` | `ask` | `deny` ; dans un objet de motifs, `"*"` en premier, la dernière règle qui correspond l'emporte.
- `opencode.json` : clés possédées seulement (`default_agent`, l'entrée `.opencode/armadai-team.md` du tableau `instructions`, `agent.build.permission.task`, `agent.plan.permission.task`, `experimental.subagent_depth`, plus `native.opencode` du projet) ; jamais `AGENTS.md`, `CLAUDE.md`, ni `~/.config/opencode/`.
- Fichier d'équipe généré : `.opencode/armadai-team.md`, contenu **neutre** (aucune phrase à la 2ᵉ personne qui donne un rôle) ; le rôle du coordinateur va dans son propre agent.
- Règles du dépôt : barrière `cargo fmt --all -- --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --no-fail-fast`, `cargo build -p armadai-core`, `cargo build -p armadai-model`, `(cd frozen/armadai-audit && cargo check --all-targets)` ; mutation vue rouge pour chaque test ajouté ; Conventional Commits, un type par sujet ; `CARGO_TARGET_DIR` privé par worktree ; jamais `git stash` ; `HOME` et `ARMADAI_CONFIG_DIR` isolés pour tout essai du binaire.

## Review Focus

1. **`opencode.jsonc` déjà présent** (OpenCode lit aussi ce nom) : un utilisateur qui l'a à la main ne doit pas se retrouver avec deux configurations ; attendu : `link` refuse avant toute écriture, message qui nomme le fichier et dit quoi faire (renommer en `opencode.json` ou retirer). Test dans la tâche 2.
2. **Agent `.armadai/` nommé `build`, `plan`, `general`, `explore` ou `scout`** : il remplacerait un agent intégré et entrerait en conflit avec les clés `agent.build|plan` ; attendu : refus nommé avant écriture. Test dans la tâche 3.
3. **Valeur d'une clé possédée modifiée à la main après `link`** (ex. `default_agent` changé) : attendu : `link` refuse de l'écraser en silence (même règle que les clés natives de `settings.json`) et `unlink` la laisse. Test dans la tâche 2.
4. **`opencode.json` invalide ou lien symbolique** : attendu : refus nommé, fichier intact (gardes de `compile/patch.rs`). Test dans la tâche 2.
5. **`strict` avec des `free_agents` aux noms Claude** (`Explore`, `general-purpose`) : attendu : ligne `approximated` qui les nomme, et ils ne débloquent rien côté OpenCode. Test dans la tâche 4.

---

### Task 1: correspondance palier → modèle dans la configuration utilisateur

**Files:**
- Modify: `crates/armadai-core/src/config.rs` (struct `UserConfig`, ~l.92)
- Test: même fichier, module de tests

**Interfaces:**
- Produces: `UserConfig.model_tiers: BTreeMap<String, BTreeMap<armadai_model::Tier, String>>` (sérialisation `model_tiers`, défaut vide, ignorée si vide) ; `pub fn model_for_tier(config: &UserConfig, target: &str, tier: armadai_model::Tier) -> Option<String>`.

- [ ] **Step 1 : tests** `model_tiers_are_read_per_target` (YAML `model_tiers: {opencode: {fast: opencode/free-a, max: anthropic/claude-x}}` → `model_for_tier(.., "opencode", Fast) == Some("opencode/free-a")`, `Pro` → `None`, cible `codex` → `None`) et `a_config_without_model_tiers_still_reads` (fichier avec seulement `starters_dirs`).
- [ ] **Step 2 :** lancer `cargo test -p armadai-core model_tiers` → échec (champ absent).
- [ ] **Step 3 :** ajouter le champ et la fonction ; `Tier` doit être `Ord` + clé de map (ajouter `PartialOrd, Ord` à `armadai_model::Tier` si absent, sans changer sa sérialisation `fast|pro|max`).
- [ ] **Step 4 :** relancer → vert ; vérifier que l'édition de `config starters-dir add` (#487) conserve `model_tiers` octet pour octet (test d'intégration existant `starters_dir_edit.rs` : ajouter un cas avec `model_tiers`).
- [ ] **Step 5 :** mutation (`model_for_tier` qui ignore la cible) → rouge ; commit `feat(config): read model_tiers for per-target model choices`.

### Task 2: patch de clés possédées dans un fichier JSON partagé

**Files:**
- Create: `crates/armadai/src/compile/json_keys.rs`
- Modify: `crates/armadai/src/compile/patch.rs` (enum `PatchKind`, fonctions `preflight_*`/`apply_*`/`remove_patch`), `crates/armadai/src/compile/mod.rs` (déclarer le module)
- Test: `json_keys.rs` (unitaires) et `patch.rs` (tests existants à compléter)

**Interfaces:**
- Produces:
  - `pub struct JsonKeys { pub values: BTreeMap<String, serde_json::Value>, pub array_entries: BTreeMap<String, Vec<serde_json::Value>> }` : clés désignées par un chemin pointé (`"agent.build.permission.task"`, `"experimental.subagent_depth"`, `"default_agent"`) et entrées possédées de tableaux (`"instructions" → [".opencode/armadai-team.md"]`).
  - `pub struct OwnedJson { pub values: BTreeMap<String, Value>, pub array_entries: BTreeMap<String, Vec<Value>>, pub containers: Vec<String> }` (sérialisable dans le manifeste, comme `claude_settings::Owned`).
  - `pub fn merge_json_keys(existing: Option<&str>, wanted: &JsonKeys, previous: &OwnedJson) -> anyhow::Result<(String, OwnedJson)>` et `pub fn remove_json_keys(existing: &str, previous: &OwnedJson) -> anyhow::Result<String>`.
  - `PatchKind::JsonKeys(OwnedJson)` ; `patch::preflight_json(root, rel, trust, wanted, previous) -> anyhow::Result<()>` et `patch::apply_json(root, rel, trust, wanted, previous) -> anyhow::Result<(PatchedFile, bool)>` (même contrat que `preflight_settings` / `apply_settings`) ; `remove_patch` traite la nouvelle variante.

- [ ] **Step 1 : tests unitaires** `merge_json_keys` :
  - fichier absent → `{"default_agent":"lead","instructions":[".opencode/armadai-team.md"]}` créé, conteneurs créés listés dans `containers` ;
  - fichier utilisateur `{"theme":"x","instructions":["mine.md"],"agent":{"build":{"model":"m"}}}` → clés utilisateur conservées, `instructions == ["mine.md", ".opencode/armadai-team.md"]`, `agent.build.model` intact ;
  - re-`link` avec un autre `wanted` → l'ancienne valeur possédée remplacée, une entrée de tableau qui n'est plus voulue retirée ;
  - **Review Focus 3** : valeur possédée modifiée par l'utilisateur (`default_agent` passé de `lead` à `other`) → erreur qui nomme la clé, aucune écriture ;
  - clé voulue déjà présente avec une autre valeur et non possédée → erreur nommée (comme une clé native en conflit) ;
  - `remove_json_keys` : retire valeurs, entrées et conteneurs créés devenus vides ; garde un conteneur que l'utilisateur avait ; garde une valeur possédée que l'utilisateur a changée.
- [ ] **Step 2 : tests de `patch.rs`** : **Review Focus 4** (JSON invalide, lien symbolique → refus, fichier intact) ; **Review Focus 1** : `opencode.jsonc` présent à côté (ou à la place) → `preflight_json` refuse avec un message qui nomme `opencode.jsonc` (la garde vit dans `preflight_json`, pilotée par un paramètre `also_refuse: &[&str]` passé par l'appelant OpenCode).
- [ ] **Step 3 :** lancer `cargo test --bin armadai json_keys patch::` → échecs attendus.
- [ ] **Step 4 :** implémenter (parcours par chemin pointé dans un `serde_json::Value`, écriture via `write_atomic` existant, mêmes gardes `checked`/`read` que `apply_settings`).
- [ ] **Step 5 :** relancer → vert ; mutations (retrait de la garde « valeur modifiée », retrait du nettoyage des conteneurs, retrait de la garde `opencode.jsonc`) → rouge ; commit `feat(link): patch owned keys of a shared JSON config file`.

### Task 3: compilateur OpenCode — agents, skills, modèle, capacités

**Files:**
- Create: `crates/armadai/src/compile/opencode.rs`
- Modify: `crates/armadai/src/compile/mod.rs` (`compiler_for("opencode")` → `opencode::OpencodeCompiler`)
- Test: `opencode.rs` (unitaires)

**Interfaces:**
- Consumes: `armadai_core::config::model_for_tier` (tâche 1) via `try_load_user_config()` (une erreur de lecture de la config utilisateur fait échouer `link` avec le message existant).
- Produces: `pub struct OpencodeCompiler;` implémentant `Compiler` ; `Plan` gagne `pub json: Option<(PathBuf, JsonKeys)>` (rempli à la tâche 4, `None` ici) ; `pub const BUILTIN_AGENTS: [&str; 5] = ["build", "plan", "general", "explore", "scout"];`.

- [ ] **Step 1 : tests** sur un projet de test en mémoire :
  - `agent_frontmatter_has_only_known_keys` : pour chaque agent produit, l'ensemble des clés du frontmatter ⊂ liste blanche (Global Constraints) ∪ clés de son `native.opencode` ; un agent avec `tier`, `tags`, `title`, `skills` n'en écrit aucune ;
  - `no_model_by_default` : `tier: max` sans `model_tiers` → pas de clé `model`, ligne `model: approximated (inherited from the OpenCode session)` ; avec `model_tiers.opencode.max = "anthropic/x"` → `model: anthropic/x`, ligne `compiled (from model_tiers in your config)` ; `native.opencode.model` l'emporte sur les deux ;
  - `skills_are_allowed_per_agent` : agent `skills: [a, b]` → `permission.skill == {"*":"deny","a":"allow","b":"allow"}` et le corps contient la consigne de charger `a` et `b` ; agent sans `skills` → pas de `permission.skill` ; chaque skill copiée en `skills/<id>/SKILL.md` ;
  - `capabilities_become_denies` : `capabilities` `write: deny`, `shell: deny`, `web: deny`, `delegate: deny` → `permission.edit`, `permission.shell`, `permission.webfetch` et `permission.websearch`, `permission.task` à `deny` (noms à confirmer dans le code 2.x ; ajuster le test au nom vérifié) ;
  - **Review Focus 2** : agent `plan` (ou tout nom de `BUILTIN_AGENTS`) → erreur de compilation qui le nomme et dit de le renommer.
- [ ] **Step 2 :** lancer → échecs.
- [ ] **Step 3 :** implémenter `OpencodeCompiler::compile` pour agents (`mode: subagent`, sauf coordinateur traité en tâche 4), skills, modèle, capacités, `native.opencode` d'agent ; un rapport par élément au format de `compile/claude.rs` ; automations : `not-compiled (needs an OpenCode plugin)`.
- [ ] **Step 4 :** relancer → vert ; mutations (une clé interdite réintroduite, modèle écrit par défaut, `permission.skill` sans `"*": deny`) → rouge ; commit `feat(link): compile agents, skills and models for OpenCode 2.x`.

### Task 4: coordinateur, fichier d'équipe, délégation `strict`

**Files:**
- Modify: `crates/armadai/src/compile/opencode.rs`
- Test: `opencode.rs` (unitaires)

**Interfaces:**
- Consumes: `claude_adapter::policy_gate::allowed_targets(project: &Project, caller: Option<&str>) -> BTreeSet<String>` (inchangée) ; `JsonKeys` (tâche 2) ; `Plan.json` (tâche 3).
- Produces: `Plan.json = Some((PathBuf::from("opencode.json"), keys))` quand au moins une clé est voulue ; artefact `armadai-team.md` (kind `Instructions`) dans le dossier de sortie.

- [ ] **Step 1 : tests :**
  - `coordinator_is_the_default_primary_agent` : `coordinator: lead` → agent `lead` en `mode: primary`, son corps contient son rôle et la consigne de délégation ; `json` contient `default_agent: lead` et l'entrée `instructions` ;
  - `team_file_is_neutral` : `armadai-team.md` liste chaque agent (id + description) et chaque flow (étapes, liens, états, `on_failure`, boucles) ; il ne contient ni « You are » ni « Tu es » ;
  - `strict_restricts_every_delegation` : flow `a → b`, `b -(rejected)-> a`, `strict` → `permission.task` de `a` == `{"*":"deny","b":"allow"}`, de `b` == `{"*":"deny","a":"allow"}` ; `agent.build.permission.task` et `agent.plan.permission.task` == règle du fil principal (`allowed_targets(project, None)`) ; `experimental.subagent_depth` == longueur de la plus longue chaîne de délégation sans cycle (au moins 1, borné à 8) ; ligne `policy delegation=strict: compiled (enforced by OpenCode permissions; agents of your own outside .armadai/ not covered)` ;
  - `off_writes_no_task_rule` : `off` → aucune clé `permission.task`, ni `agent.*`, ni `subagent_depth` ; flows `approximated (guidance only)` ;
  - **Review Focus 5** : `free_agents: [Explore, general]` → `general` autorisé, ligne `approximated` qui nomme `Explore` comme inconnu d'OpenCode ;
  - `native_opencode_of_the_project_is_merged` : `native.opencode` du projet → dans `json.values` ; une clé qui heurte une clé possédée → erreur nommée.
- [ ] **Step 2 :** lancer → échecs.
- [ ] **Step 3 :** implémenter ; la plus longue chaîne se calcule sur le graphe agent → cibles de `allowed_targets` (parcours en profondeur, cycles ignorés).
- [ ] **Step 4 :** relancer → vert ; mutations (`"*":"deny"` retiré, règle `build/plan` omise, `subagent_depth` figé à 1, phrase de rôle dans le fichier d'équipe) → rouge ; commit `feat(link): coordinator, team file and strict delegation for OpenCode`.

### Task 5: branchement dans `link` / `unlink` et manifeste

**Files:**
- Modify: `crates/armadai/src/cli/link.rs` (struct `Prepared` ~l.22, bloc `if target == "claude"` ~l.145, application ~l.425), `crates/armadai/src/cli/unlink.rs` (ou équivalent qui appelle `remove_patch`), `crates/armadai/src/linker/manifest.rs` si la variante `JsonKeys` doit y être lue
- Test: `crates/armadai/tests/link_opencode.rs` (nouveau, binaire réel, `HOME`/`ARMADAI_CONFIG_DIR` isolés)

**Interfaces:**
- Consumes: `Plan.json` (tâches 3-4), `patch::preflight_json` / `apply_json` (tâche 2).

- [ ] **Step 1 : tests d'intégration :**
  - `link_then_unlink_restores_a_hand_written_opencode_json` : `opencode.json` utilisateur `{"theme":"x","instructions":["mine.md"]}` + projet `strict` avec coordinateur → après `link`, clés possédées présentes ; après `unlink`, le fichier redevient **égal en valeur** à l'original ; `.opencode/agents/`, `.opencode/skills/`, `.opencode/armadai-team.md` retirés ; aucun `AGENTS.md` ni `CLAUDE.md` créé ou modifié (octets comparés) ;
  - `link_creates_opencode_json_and_unlink_removes_it` : sans `opencode.json` au départ → créé par `link`, supprimé par `unlink` ;
  - `the_ignored_instructions_md_is_removed_as_stale` : projet lié par la version précédente (fichier `.opencode/instructions.md` enregistré dans un manifeste existant) → retiré au `link` suivant, ligne de rapport « stale » ;
  - `dry_run_writes_nothing` : `link --dry-run` → aucun fichier ni clé.
- [ ] **Step 2 :** lancer → échecs.
- [ ] **Step 3 :** brancher : préflight avant toute écriture (comme `settings`), application après les fichiers entiers, enregistrement du `PatchedFile` dans le manifeste, retrait par `unlink` ; le patch JSON vise `<racine>/opencode.json` (pas le dossier de sortie).
- [ ] **Step 4 :** relancer → vert ; mutations (préflight retiré, `unlink` qui oublie `JsonKeys`) → rouge ; commit `feat(link): apply and remove OpenCode's owned config keys`.

### Task 6: étalons des 7 packs, documentation, retrait de l'ancien linker

**Files:**
- Modify: `crates/armadai/tests/golden/link/opencode.txt` et les étalons `init_pack` qui couvrent `opencode` ; `docs/wiki/link.md` (section « Codex, Copilot, OpenCode » → section OpenCode à part) ; `CHANGELOG.md` (`## Unreleased`) ; `crates/armadai/src/linker/mod.rs` et `linker/opencode.rs` (supprimer `OpencodeLinker` s'il n'a plus d'appelant)
- Create: une fixture de projet `strict` avec coordinateur et boucle (ex. `crates/armadai/tests/fixtures/opencode-strict/`) et son étalon

- [ ] **Step 1 :** régénérer les étalons (`UPDATE_GOLDEN=1`), relire chaque différence ; ajouter le test `every_pack_links_to_opencode_with_known_keys_only` (les 7 packs : `link --target opencode --dry-run` réussit et le test de liste blanche de la tâche 3 passe sur chaque agent écrit).
- [ ] **Step 2 :** documentation : ce que lit OpenCode 2.x, la table §5 de la spec, `model_tiers`, la règle sur `build`/`plan` sous `strict`, ce qui reste `not-compiled` (automations) ; CHANGELOG `Changed` (cible OpenCode) ; `.opencode/instructions.md` n'est plus écrit.
- [ ] **Step 3 :** supprimer `OpencodeLinker` et `linker/opencode.rs` si plus aucun appelant (`git grep OpencodeLinker`), sinon le dire dans le rapport.
- [ ] **Step 4 :** barrière complète ; commit(s) `test(link): golden outputs for OpenCode 2.x`, `docs: the OpenCode 2.x target`, `refactor(linker): remove the legacy OpenCode linker`.

### Task 7: essai réel dans OpenCode 2.1.2 (Dimitri)

Pas de code. Après la tâche 6 et la revue de branche, l'exécutant prépare un projet de test (fixture de la tâche 6, `link --target opencode`) et une liste d'essais ; Dimitri les lance et rapporte :
1. le coordinateur est l'agent par défaut et présente son équipe ;
2. sous `strict`, une délégation hors flow est refusée, depuis le coordinateur **et** depuis `build` (Tab) ; une délégation permise passe ; un sous-agent du flow peut déléguer à son successeur ;
3. un agent avec `skills:` ne voit que ses skills ;
4. aucun modèle imposé par défaut ; avec `model_tiers`, le modèle attendu ;
5. `unlink` laisse le projet propre.
Tout écart devient une correction avant fusion.
