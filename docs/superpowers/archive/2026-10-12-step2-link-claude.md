# Étape 2 — `link` sur le nouveau modèle, adaptateur Claude Code, rapport : plan d'implémentation

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal :** `link` compile le projet `armadai-model` vers les quatre cibles, avec un adaptateur Claude Code complet (agents, skills, CLAUDE.md, hooks, gate, `native`) et un rapport de conformité.

**Architecture :** nouveau module `crates/armadai/src/compile/` (plan d'artefacts + rapport + un compilateur par cible) ; l'écriture et le suivi passent par le `linker/manifest.rs` existant (état d'écriture, `unlink`). Les linkers Codex/OpenCode/Copilot existants sont conservés et alimentés par un `LinkAgent` construit depuis le modèle. Le gate `__claude-policy-gate` est réécrit pour lire le nouveau modèle.

**Tech Stack :** Rust 2024, `armadai-model` (étape 1), `serde_json`, `clap`.

**Spec :** `docs/superpowers/specs/2026-10-12-step2-link-claude-design.md` (lire en entier ; §2 = choix S1–S10, §4 = adaptateur, §5 = rapport).

## Global Constraints

- Barrière avant chaque commit : `cargo fmt --all -- --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --no-fail-fast`, `cargo build -p armadai-core`, `cargo build -p armadai-model`, `(cd frozen/armadai-audit && cargo check --all-targets)` avec `CARGO_TARGET_DIR` hors dépôt.
- `crates/armadai` est binaire : tester avec `--bin armadai` ou `--test <nom>`.
- Conventional Commits, un type par sujet ; trailer `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`. Code et commits en anglais.
- Mutation obligatoire par test ajouté (casser, voir rouge, restaurer par copie, jamais en préservant la mtime). TDD réel : voir le test rouge avant d'implémenter.
- Formats Claude Code (vérifiés sur la doc officielle le 2026-10-12) : sous-agent `.claude/agents/<id>.md`, frontmatter `name` (= id), `description`, `tools` (liste YAML ou chaîne séparée par virgules), `model` (alias `sonnet|opus|haiku|fable`, id complet ou `inherit`) ; outil de délégation = `Agent` ; hook = `{"hooks":{"<Event>":[{"matcher":"…","hooks":[{"type":"command","command":"…","timeout":…}]}]}}` ; refus de gate = `{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"…"}}` ; sortie vide = flux normal ; un seul `CLAUDE.md` (racine) ; sortie versionnée dans `.claude/settings.json` (jamais `settings.local.json`).
- Aucune modification de signature publique de `armadai-core` avant la Task 8 (le crate gelé `audit` en dépend).
- Aucun marqueur dans les objets de hook ; propriété des entrées tracée par le manifeste (valeurs exactes).
- `armadai link` n'effectue aucune requête réseau.
- Fixtures sous `.armadai/` : exceptions `.gitignore` explicites (`.armadai/` est ignoré) ; vérifier `git ls-files`.

## Review Focus

1. `.claude/settings.json` utilisateur existant avec hooks, permissions et clés inconnues : préservé, relancer `link` ne duplique rien, `unlink` ne retire que les ajouts d'ArmadAI ; fichier JSON invalide = erreur sans écriture.
2. Gate : payload tronqué, `subagent_type` absent, cwd dans un worktree git, projet sans `.armadai/`, projet en `policy: off` : sortie vide, jamais de refus par incompréhension.
3. Projet avec `capabilities` partiels, tous `deny`, ou `delegate: deny` pour le coordinateur : `tools` cohérent, rapport honnête.
4. Projet sans coordinateur, sans flow, sans automation, ou avec des agents non référencés : `link` réussit, aucun fichier vide parasite.
5. Collision `native` et chemins dangereux (`..`, absolu) dans `.armadai/native/claude/` : erreur sans écriture.

## Révisions issues de la revue indépendante (PR #465) — **ces décisions priment sur le texte des tâches en cas de contradiction**

Chaque point corrige un défaut mesuré par la revue (bloquants B1–B5, importants I1–I11) ; l'implémenteur lit le code cité avant d'écrire.

- **R-B1 Fichiers patchés, hors `write_files`.** `settings.json` et le `CLAUDE.md` racine ne sont PAS des `Artifact` écrits par `manifest::write_files` (qui gère des fichiers entiers : `SkippedExisting` sans `--force`, suppression du fichier utilisateur entier à `unlink --force`). Nouveau module `compile/patch.rs` : `PatchedFile { path: PathBuf, kind: PatchKind, created: bool }`, `PatchKind::{ SettingsHooks { entries: Vec<HookEntry> }, MarkdownBlock }`, persistés dans `TargetManifest` par un champ `#[serde(default)] patched: Vec<PatchedFile>` (manifestes anciens relus sans erreur). `apply_settings(path, entries, previous) -> Result<PatchedFile>` et `apply_markdown_block(path, block) -> Result<PatchedFile>` : écriture **atomique** (fichier temporaire + renommage), refus si le fichier est un lien symbolique ou un JSON invalide (rien n'est écrit), `serde_json` avec la feature `preserve_order` (ordre des clés utilisateur conservé). `unlink` retire les `entries` (égalité de valeur au niveau du handler, puis supprime les groupes/tableaux/objets `hooks` vides **créés** par link, préserve un `"hooks": {}` préexistant) ou le bloc entre marqueurs, et ne supprime le fichier que si `created: true` et qu'il est devenu vide. Une entrée identique déjà présente sans manifeste n'est pas dupliquée et n'est pas revendiquée (rapport : `approximated: existing identical hook kept, not owned`). Signatures exactes à ajouter à `linker/manifest.rs` : `write_target(..., patched: &[PatchedFile])`.
- **R-B2 Racine de confiance et entrées périmées.** Le `CLAUDE.md` racine est un `MarkdownBlock` (racine de confiance = racine du projet, hors `.claude/`) ; un `CLAUDE.md` existant sans marqueurs reçoit le bloc en fin de fichier, contenu préservé. L'ancien `.claude/CLAUDE.md` et tout fichier du manifeste précédent qui n'est plus produit (anciens `.claude/prompts/*`, agents nommés par titre) : supprimés si leur digest correspond encore au contenu disque, sinon laissés en place avec une ligne de rapport `stale file left in place`.
- **R-B3 Tests et câblage progressif.** `crates/armadai` est binaire : les tests des Tasks 1–5 sont des `#[cfg(test)] mod tests` dans les modules ; les tests de `tests/*.rs` (binaire lancé par `assert_cmd`) n'arrivent qu'à partir de la Task 6. `mod compile` porte un `#![allow(dead_code)]` **temporaire** (Tasks 1–5), retiré en Task 6 (vérifier `git grep allow(dead_code)` vide en fin de Task 6).
- **R-B4 Gate sans régression.** Reprendre `claude_adapter/policy_gate.rs` : matcher `Agent|Task` (alias `Task` conservé, test `the_task_alias_is_policed_too` porté), `subagent_type` absent = sous-agent implicite **contrôlé** (test `an_agent_call_without_subagent_type_is_still_policed` porté), sémantique dépendante de l'appelant de `check_delegation` portée sur le graphe : fil principal → coordinateur (ou, sans coordinateur, steps sans lien entrant) ; agent X → Y si un lien X→Y existe dans un flow ; `free_agents` toujours permis. Le rapport dit `enforced: delegation topology` seulement dans ce cas. Les tests unitaires de `policy_gate.rs` (écrits sur `config.yaml`) sont réécrits sur `project.yaml` dans la Task 4.
- **R-B5 Capabilities → `disallowedTools`.** ArmadAI n'émet jamais `tools` (héritage de Claude Code conservé : MCP, Skill, TodoWrite…). `deny` → `disallowedTools` (même format que `tools` : vérifier dans la doc des sous-agents), `allow` = défaut (compiled, sans sortie), `ask` = sans effet par agent (`approximated`). `delegate: deny` → `Agent` dans `disallowedTools` ; `delegate: allow` sur un agent non coordinateur : `approximated: subagents cannot spawn subagents` après vérification de la doc.
- **R-I1 Inventaire des commandes.** Chaque commande de `armadai --help` reçoit une disposition justifiée dans le message de commit de la Task 7 : portées (`list`, `inspect`, `validate`, `link`, `unlink`, `init`, `new`) ; retirées (`extract`, `prompts` : le concept de prompt n'existe plus, C6) ; inchangées (`models`, `config`, `setup`, `update` : vérifier chaque dépendance restante à `project_registry`, `model_updater`, `agent_source` par `grep` et les conserver tant qu'elles servent) ; `skills` conservée, chemins de projet lus sous `.armadai/skills`. `register_project` et `auto_check_and_prompt` restent appelés par `link` (comportement existant). `unlink --with-config` et `--coordinator` : retirés si sans objet dans le nouveau modèle (justifier), sinon redéfinis.
- **R-I2 `unlink` sans manifeste** (`Lookup::Fallback`) : retire les fichiers dont le contenu égale le plan recompilé ; ne touche jamais `settings.json` ni le `CLAUDE.md` (rapport : « hooks/CLAUDE.md block not removed: no manifest »).
- **R-I3 Octets et variantes.** `write_files` accepte du contenu binaire (`Vec<u8>`) ; les fichiers non UTF-8 de skills sont copiés intacts (aujourd'hui perdus en silence). `ProducedByKind` gagne `Native` ; un ancien binaire lira ce manifeste en `Fallback` (noté dans CHANGELOG).
- **R-I4 Manifeste hors git.** `.armadai/` est désormais versionné : `link`, `migrate` et `init` créent `.armadai/.gitignore` (s'il manque) contenant `link-manifest.yaml` et `.migrated/` ; test.
- **R-I5 Modèles déterministes.** Claude : alias `haiku`/`sonnet`/`opus` (`fast`/`pro`/`max`) ; sans `tier` ni `native.claude.model` : `model` omis (`inherit`). Autres cibles : lire `armadai-core/src/model_resolution.rs` et `linker/model_resolution.rs` ; si aucune fonction n'évite le cache local de la machine, ajouter à core une fonction pure `embedded_model_for_tier(provider, tier)` (ajout `pub` autorisé) ; pas de `compile/models.rs` en doublon de la logique existante. Les tests isolent `HOME` et `ARMADAI_CONFIG_DIR`.
- **R-I6 Rapport complet.** Lignes pour : `temperature` (`not-compiled: no equivalent in Claude Code subagents`), `tags`, `title` (informatifs : `compiled` ou `not-compiled` justifiés), `agent.skills` (`skills:` du frontmatter de sous-agent si la doc le confirme, sinon `not-compiled`), `states`/`on_failure` (`approximated: guidance`). `native.claude` du **manifeste** : fusionné au niveau racine de `settings.json` (collision avec une clé générée = erreur) ; `native.claude` d'une **automation** : fusionné dans son objet de handler. Fichiers `.armadai/native/claude/<chemin>` : copiés vers `<racine projet>/<chemin>` (ex. `native/claude/.mcp.json` → `.mcp.json`) ; `--output` ne les déplace pas.
- **R-I7** `native.claude.model` surcharge `model` ; toute autre clé native qui collisionne avec une clé générée = erreur nommant la clé.
- **R-I8** Task 5 : « identiques à l'ancien chemin, aux différences listées près » : choix du modèle (plus de prompt interactif, de `--model`, de `resolve_latest_placeholders`, de remappage par cache), slugs (id et non titre), prompts recomposés par `migrate`. La liste des différences est dans le message de commit.
- **R-I9 `init --pack` / `new`.** `init --pack` : matérialiser le pack dans un dossier temporaire avec un `armadai.yaml` synthétique listant agents/prompts/skills, puis `read_old` → `convert` → `write_all` (sans perdre les `SkillCopy`) ; `new` : parser le modèle embarqué avec le parseur existant, le convertir en un `Agent` du nouveau modèle et l'écrire avec `armadai_model::render_agent`.
- **R-I10 Tests à réécrire en Task 6/7** en plus de la liste : `hook_stdout.rs`, `models_update_declarations.rs`, `new_templates.rs`.
- **R-I11 Liens symboliques.** La copie des skills et de `native` rejette tout fichier dont le chemin réel sort du dossier source (même garde que `manifest::is_trusted`), avec erreur nommant le fichier.
- **Précisions** : `--json` est une option nouvelle de `link` ; `--output` avec plusieurs cibles = erreur ; `--agents` restreint les agents écrits, pas `CLAUDE.md` ni les hooks, et chaque agent écarté a une ligne de rapport `skipped (--agents)` ; le coordinateur n'est pas écrit dans `.claude/agents/` (il vit dans `CLAUDE.md`, comportement actuel) ; `ClaudeLinker` est supprimé en Task 6 ; le gate écrit un avertissement sur stderr quand le projet est invalide (puis reste silencieux sur stdout) ; « `manifest.targets` » et « `outputs` » désignent `project.yaml`, le fichier d'état de link est `link-manifest.yaml` ; la Task 7 se réduit pour `validate` à l'erreur sur l'ancien format.

---

## Structure des fichiers

| Fichier | Responsabilité |
|---|---|
| `crates/armadai-model/src/lib.rs` (+ `load.rs`) | `load_checked(root) -> Result<(Project, Origins), Vec<Diagnostic>>` : charge et refuse sur erreur |
| `crates/armadai/src/compile/mod.rs` | `Artifact`, `Plan`, `Compiler`, `compile_all` |
| `crates/armadai/src/compile/report.rs` | `Status`, `ReportLine`, rendu texte/JSON |
| `crates/armadai/src/compile/claude.rs` | Adaptateur Claude (agents, skills, CLAUDE.md, native) |
| `crates/armadai/src/compile/claude_settings.rs` | Hooks, gate, fusion de `settings.json` |
| `crates/armadai/src/compile/legacy_targets.rs` | `LinkAgent` depuis le modèle + Codex/OpenCode/Copilot |
| `crates/armadai/src/compile/models.rs` | Résolution palier → modèle depuis la table embarquée |
| `crates/armadai/src/claude_adapter/policy_gate.rs` | Gate sur le nouveau modèle |
| `crates/armadai/src/cli/link.rs`, `unlink.rs`, `list.rs`, `inspect.rs`, `validate.rs`, `init.rs`, `new.rs` | Câblage |
| `crates/armadai/tests/link_new_*.rs`, `tests/golden/link/**` | Tests et étalons |

---

### Task 1 : Charger-et-refuser, plan d'artefacts, rapport

**Files:** `crates/armadai-model/src/{load.rs,lib.rs}`, `crates/armadai/src/compile/{mod,report}.rs`, `crates/armadai/src/main.rs`.

**Interfaces — Produces :**
- `armadai_model::load_checked(root: &Path) -> Result<(Project, Origins), Vec<Diagnostic>>` : `Err` si au moins une erreur (les warnings passent).
- `compile::Artifact { path: PathBuf /*relatif à la sortie de la cible*/, content: Vec<u8>, kind: ArtifactKind, produced_by: ProducedBy }` ; `ArtifactKind { Agent, Skill, Instructions, Settings, Native }`.
- `compile::Plan { artifacts: Vec<Artifact>, report: Report }`.
- `compile::report::{Status { Compiled, Approximated, NotCompiled }, ReportLine { target: String, element: String, status: Status, reason: Option<String> }, Report { lines: Vec<ReportLine> }}` ; `Report::render_text() -> String` (trié par cible puis élément), `Report::to_json() -> serde_json::Value`, `Report::push(...)`.
- `compile::Compiler` trait : `fn target(&self) -> &'static str; fn compile(&self, project: &Project, root: &Path) -> anyhow::Result<Plan>`.

- [ ] **Step 1 : tests d'abord** : `load_checked` rend `Err` sur une erreur et `Ok` avec warning seul ; rendu texte du rapport (ordre, ligne `approximated` avec raison) ; JSON stable (champs `target`, `element`, `status`, `reason`).
- [ ] **Step 2–4 :** rouge, implémenter, vert.
- [ ] **Step 5 :** mutations (inverser `has_errors`, ne pas trier, perdre la raison). **Step 6 :** barrière + commit `feat(compile): add the artifact plan and conformance report types`.

---

### Task 2 : Résolution des paliers et adaptateur Claude (agents, skills, CLAUDE.md)

**Files:** `compile/models.rs`, `compile/claude.rs`, tests `crates/armadai/tests/link_new_claude.rs`.

**Interfaces — Consumes :** Task 1. **Produces :** `compile::claude::ClaudeCompiler` (impl `Compiler`), `compile::models::resolve_tier(tier: Tier, target: &str) -> Option<String>`.
- `resolve_tier` : lire `crates/armadai-core/src/model_resolution*.rs` / `model_aliases.rs` pour la table embarquée existante ; pas de réseau ; `native.<cible>.model` du modèle prime. Pour Claude, un palier peut être exprimé par l'alias (`fast→haiku`, `pro→sonnet`, `max→opus`) **si** la table embarquée n'a pas de meilleure entrée : trancher en lisant le code, documenter le choix dans le commit.
- Agent Claude : frontmatter `name: <id>`, `description` (échappée YAML), `model`, `disallowedTools` (R-B5 : lecture = Read, Grep, Glob ; écriture = Edit, Write, NotebookEdit ; shell = Bash ; web = WebFetch, WebSearch ; delegate = Agent ; jamais de `tools`), `native.claude` de l'agent fusionné dans le frontmatter (collision de clé avec une clé générée = erreur nommant la clé), corps = prompt.
- Skills : copie de `.armadai/skills/<id>/` vers `.claude/skills/<id>/` (tous fichiers, contenu octet pour octet, ordre trié).
- `CLAUDE.md` racine : si `coordinator` est déclaré, prompt du coordinateur + section `## Delegation` générée (pour chaque flow : steps, liens, états `when`, `on_failure`), encadrée par `<!-- armadai:begin -->`/`<!-- armadai:end -->` ; un `CLAUDE.md` existant sans ces marqueurs n'est jamais écrasé : section ajoutée en fin de fichier (le contenu existant est préservé), rapport `approximated`. Sans coordinateur : pas de `CLAUDE.md` généré.
- Rapport : une ligne par agent (`compiled`), par skill, par flow (`approximated: rendered as guidance, not enforced`, sauf la topologie appliquée par le gate en `strict` : ligne `enforced: delegation topology` ajoutée en Task 4), `ask` → `approximated`.

- [ ] **Step 1 : tests d'abord** : agent minimal ; agent avec capabilities mixtes (`allow`/`ask`/`deny`) ; `tools` omis sans capacité ; `native.claude.model` prime sur `tier` ; collision de clé native ; skill copiée avec fichier annexe ; `CLAUDE.md` généré avec flow en boucle ; `CLAUDE.md` existant préservé ; description avec `:` et guillemets (aller-retour YAML) ; aucun artefact vide.
- [ ] **Step 2–4 :** rouge, implémenter, vert. **Step 5 :** mutations (inverser deny/allow, ignorer `native`, écraser `CLAUDE.md`). **Step 6 :** barrière + commit `feat(compile): compile agents, skills and CLAUDE.md for Claude Code`.

---

### Task 3 : Hooks et fusion de `.claude/settings.json`

**Files:** `compile/claude_settings.rs`, tests `crates/armadai/tests/link_new_settings.rs`.

**Interfaces — Produces :** `pub fn hook_entries(project: &Project) -> Vec<HookEntry>` avec `HookEntry { event: &'static str, matcher: Option<String>, command: String }` ; `pub fn merge_settings(existing: Option<&str>, entries: &[HookEntry], previous: &[HookEntry]) -> anyhow::Result<String>` (JSON pretty, clés existantes préservées, entrées `previous` retirées par égalité puis `entries` ajoutées, aucune duplication d'une entrée identique déjà présente) ; `pub fn remove_entries(existing: &str, previous: &[HookEntry]) -> anyhow::Result<String>`.
- Événements : session-start→`SessionStart`, session-end→`SessionEnd`, before-tool→`PreToolUse`, after-tool→`PostToolUse`, on-prompt→`UserPromptSubmit`, on-stop→`Stop` ; `matcher` absent (= tous) pour les automations ; `command` = `run` tel quel ; `timeout` omis.
- La liste `previous` vient du manifeste d'écriture : étendre `linker/manifest.rs` pour persister `hook_entries` par cible (lire le format actuel, compat ascendante : champ optionnel `#[serde(default)]`).
- JSON invalide en entrée = `Err` (rien écrit) ; fichier absent = création.

- [ ] **Step 1 : tests d'abord** : création ; fusion préservant hooks/permissions/clés inconnues utilisateur ; idempotence (`merge(merge(x)) == merge(x)`) ; retrait exact (`remove_entries` après `merge` redonne `x` à l'ordre des clés près, compare par `Value`) ; doublon identique déjà présent non dupliqué ; JSON invalide = erreur ; manifeste ancien sans `hook_entries` se relit.
- [ ] **Step 2–4 :** rouge, implémenter, vert. **Step 5 :** mutations (supprimer le retrait, dupliquer, écraser les clés utilisateur). **Step 6 :** barrière + commit `feat(compile): merge ArmadAI hooks into Claude settings without clobbering`.

---

### Task 4 : Gate de délégation sur le nouveau modèle

**Files:** `claude_adapter/policy_gate.rs`, `compile/claude_settings.rs` (entrée du gate), tests `crates/armadai/tests/gate_new.rs`.

**Interfaces — Consumes :** `armadai_model::load_checked` (silence sur erreur), `HookEntry`. **Produces :** `pub fn decide(raw_payload: &str) -> Option<String>` (existant, nouvelle sémantique) ; `pub fn allowed_agents(project: &Project) -> BTreeSet<String>` = ids des steps de tous les flows ∪ `coordinator` ∪ `policy.free_agents`.
- `decide` : `tool_name` ∈ {`Agent`, `Task`} ; `tool_input.subagent_type` ; projet résolu depuis `cwd` (remontée, repli sur le checkout principal d'un worktree — reprendre `resolve_project`/`main_checkout_of` existants) ; `policy.delegation == strict` ; cible hors `allowed_agents` → refus (`permissionDecision: deny`, raison citant les agents permis) ; tout autre cas → `None` (sortie vide).
- Quand `strict` : `hook_entries` ajoute `{ event: "PreToolUse", matcher: Some("Agent"), command: "armadai __claude-policy-gate" }` et le rapport `enforced: delegation topology`.

- [ ] **Step 1 : tests d'abord** : payloads réels : cible permise ; cible refusée (message cite les permis) ; free agent ; `policy: off` ; hors projet ; worktree ; JSON tronqué ; `tool_name` autre ; `subagent_type` absent ; projet invalide (erreur de validation) = silence. Test e2e du binaire : `armadai __claude-policy-gate` lit stdin, stdout = uniquement le JSON de décision.
- [ ] **Step 2–4 :** rouge, implémenter, vert. **Step 5 :** mutations (refuser par défaut sur erreur, oublier `free_agents`, matcher `Task`). **Step 6 :** barrière + commit `feat(gate): read the canonical model and install the delegation gate`.

---

### Task 5 : Cibles Codex, OpenCode, Copilot depuis le nouveau modèle

**Files:** `compile/legacy_targets.rs`, tests `crates/armadai/tests/link_new_targets.rs`.

**Interfaces — Produces :** `pub fn link_agents(project: &Project, target: &str) -> Vec<LinkAgent>` (id → `name`, corps → `system_prompt`, `description` → `description`, `tags`, `temperature` (défaut 0.7), `model` résolu par `resolve_tier`/`native.<cible>.model`, `provider` : `anthropic` pour claude, `openai` pour codex, sinon `None`) ; `LegacyCompiler { target }` impl `Compiler` qui appelle `create_linker(target).generate(...)` et convertit les `OutputFile` en `Artifact`.
- Le coordinateur éventuel passe au linker comme aujourd'hui (`coordinator: Option<&LinkAgent>`).
- Rapport par cible : chaque agent `compiled` ; chaque skill `compiled` si le linker les copie, sinon `not-compiled`; chaque automation/flow/capability `not-compiled (step 4)`.

- [ ] **Step 1 : tests d'abord** : pour les trois cibles, un projet à deux agents et un flow produit les mêmes fichiers que l'ancien chemin sur le projet équivalent (comparer à des étalons committés générés une fois depuis l'ancien `link` sur la fixture — avant de supprimer l'ancien chemin) ; rapport `not-compiled` présent pour automations et capabilities.
- [ ] **Step 2–4, 5, 6 :** comme ci-dessus. Commit `feat(compile): feed the Codex, OpenCode and Copilot linkers from the canonical model`.

---

### Task 6 : Câbler `link` et `unlink`

**Files:** `cli/link.rs`, `cli/unlink.rs`, `cli/mod.rs`, `linker/mod.rs`, tests `crates/armadai/tests/link_new_cli.rs`.

**Interfaces — Consumes :** Tasks 1–5. **Produces :** `armadai link [--target T …] [--output DIR] [--agents a,b] [--force] [--dry-run] [--json]` ; retrait de `--coordinator` et `--model`.
- Séquence : ancien projet détecté (`.armadai/config.yaml`/`armadai.yaml|yml` sans `project.yaml`) → erreur `legacy format, run armadai migrate`, code 1 ; `load_checked` ; cibles = `--target` sinon `manifest.targets` sinon `claude` ; `outputs` du manifeste = dossier de sortie par cible ; plan par cible ; collision `native` (fichier de `.armadai/native/<cible>/` au même chemin qu'un artefact généré) = erreur, rien écrit ; chemins `native` hors dossier (`..`, absolu, lien symbolique sortant) = erreur ; copie des `native` comme artefacts `Native` ; écriture via `manifest::write_files` ; rapport imprimé (texte, ou JSON seul sur stdout avec `--json`) ; `--dry-run` n'écrit rien (test par comparaison d'arbres).
- `unlink` : ne dépend plus de l'ancienne config ; retire aussi les `hook_entries` enregistrées via `remove_entries` (Task 3).
- Supprimer l'ancien chemin de `link` qui lisait `ProjectConfig` (pas de double chemin) ; les tests `link_*` existants (`link_golden`, `link_manifest`, `link_coordinator_no_match`, `link_list_gate`, `unlink_content_guard`, `prompt_section_boundaries`, `duplicate_metadata_key_warns`, `orchestration_pattern_declarable`, `gemini_unlink`) sont réécrits sur le nouveau modèle ou supprimés **avec justification dans le message de commit** (le comportement qu'ils protégeaient existe-t-il encore ?). Aucun test supprimé sans qu'un équivalent couvre le même risque.

- [ ] **Step 1 : tests d'abord** : e2e sur les six fixtures migrées (`migrate` puis `link --target claude`) ; `--dry-run` ; `--json` pur ; collision `native` ; chemin `native` hostile ; legacy → erreur ; `unlink` après `link` rend l'arbre initial (comparaison) y compris `settings.json` utilisateur préexistant ; relancer `link` idempotent.
- [ ] **Step 2–4, 5 :** mutations (écrire pendant `--dry-run`, ignorer la collision, ne pas retirer les hooks). **Step 6 :** commit `feat(link): compile the canonical model for every target`.

---

### Task 7 : `list`, `inspect`, `validate`, `init`, `new`, retrait d'`extract`

**Files :** `cli/{list,inspect,validate,init,new,extract,mod}.rs`, tests.

- `list`/`inspect` : agents, skills, flows, automations du nouveau modèle (`--json` conservé s'il existe). `validate` : mode projet = nouveau modèle ; ancien format = erreur « run `armadai migrate` » (code 1) ; mode pack inchangé ; supprimer l'usage de `validate_project_config`.
- `init --project` : écrit `.armadai/project.yaml` (`schema_version: 1`, `targets` inféré comme aujourd'hui) + dossiers `agents/ skills/ flows/ automations/` (avec `.gitkeep`) ; `init --pack` : lit le pack (ancien format), le convertit avec le convertisseur de `migrate` en mémoire (`migrate::convert`) et écrit le nouveau format (réutiliser `armadai_model::write_project`) ; `new` : idem pour un modèle embarqué → un agent `.armadai/agents/<id>.md`. Lire `migrate/{read,convert}.rs` pour factoriser sans dupliquer : exposer une fonction `migrate::from_legacy_dir(dir: &Path) -> (Project, Report)` si besoin.
- `extract` : supprimer la commande, ses tests, son aide, `dependency_resolver` si plus utilisé ailleurs (`grep`), l'entrée de CHANGELOG « Removed ».
- Tests : un par commande sur fixtures du nouveau modèle ; `init --pack` produit un projet que `validate` accepte et que `link --target claude` compile ; `new` idem.

- [ ] **Steps 1–6** comme ci-dessus. Commits séparés : `feat(cli): read the canonical model in list, inspect and validate`, `feat(init): write the canonical format`, `refactor(cli): remove extract`.

---

### Task 8 : Étalons, nettoyage d'`armadai-core`, documentation

**Files :** `crates/armadai/tests/link_golden.rs` (réécrit), `tests/golden/link/**`, `schemas/armadai.schema.json` (supprimer), `crates/armadai-core/src/**` (nettoyage S10), `docs/wiki/*`, `CHANGELOG.md`, `CLAUDE.md`.

- Étalons : pour chacune des six fixtures migrées et la cible Claude, l'arbre complet produit par `link` (agents, skills, `CLAUDE.md`, `.claude/settings.json`) + `report.txt` ; pour Codex/OpenCode/Copilot, l'arbre et le rapport sur une fixture ; `UPDATE_GOLDEN=1` régénère ; relire chaque étalon à la main et noter ce qui a été vérifié.
- Nettoyage S10 : `grep -rn` de chaque symbole public de `armadai-core` (`ProjectConfig`, `AgentRef`, `PromptRef`, `SkillRef`, `parse_agent_file`, `agent_source::*`, `project_registry::*`, `pack_validation::validate_project_config`, `orchestration::*` non lu par `migrate`, …) ; supprimer ce qui n'est lu ni par `migrate`, `init --pack`, `new`, ni par le crate gelé `audit` ; si le crate gelé casse, le re-geler en copiant le minimum dans `frozen/armadai-audit/src/vendored/` (comme à l'étape 0) ; `cargo build -p armadai-core` vert.
- Documentation : wiki `link` (nouveau pipeline, rapport, `--json`, `native`), page du gate (`policy`, allowed agents), format du projet (renvoi), CHANGELOG (Changed/Removed : `extract`, `--coordinator`, `--model`, ancien chemin de `link`), `CLAUDE.md` (architecture, commandes, fin de l'état transitoire).

- [ ] **Step 1 : tests/étalons d'abord ; Step 2 :** nettoyage avec preuve `grep` avant/après dans le message de commit ; **Step 3 :** docs ; **Step 4 :** barrière complète + `cargo test -p armadai` ; commits `test(link): golden snapshots on the canonical model`, `refactor(core): drop the legacy model no longer read`, `docs: document link, the gate and the conformance report`.

---

## Auto-relecture

- **Couverture de la spec** : S1 (T6), S2 (T5, T6), S3/§5 (T1, T2, T4, T5), S4 (T2), S5 (T3), S6 (T4), S7 (T2), S8 (T7), S9 (T6), S10 (T8) ; critères 1 (T8), 2 (T3, T6), 3 (T4), 4 (T2, T4, T5), 5 (T6), 6 (T5, T8), 7 (T8), 8 (T8).
- **Écarts assumés** : propriété des hooks par le manifeste et non par marqueur (champ inconnu non documenté) ; refus de gate au format `hookSpecificOutput` ; la liste exacte de la syntaxe `permissions` n'est pas émise à l'étape 2 (non vérifiée : `ask` reste un `tools`+rapport).
- **Risque d'ordre** : les étalons de `link` actuels doivent être générés depuis l'ancien chemin (Task 5, Step 1) **avant** sa suppression (Task 6).
