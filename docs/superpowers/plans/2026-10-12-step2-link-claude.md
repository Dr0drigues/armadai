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
- Agent Claude : frontmatter `name: <id>`, `description` (échappée YAML), `model`, `tools` (S4/§4.2 de la spec : lecture = Read, Grep, Glob ; écriture = Edit, Write, NotebookEdit ; shell = Bash ; web = WebFetch, WebSearch ; delegate = Agent ; `tools` omis si aucune capacité déclarée), `native.claude` de l'agent fusionné dans le frontmatter (collision de clé avec une clé générée = erreur nommant la clé), corps = prompt.
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
- `decide` : `tool_name == "Agent"` seulement ; `tool_input.subagent_type` ; projet résolu depuis `cwd` (remontée, repli sur le checkout principal d'un worktree — reprendre `resolve_project`/`main_checkout_of` existants) ; `policy.delegation == strict` ; cible hors `allowed_agents` → refus (`permissionDecision: deny`, raison citant les agents permis) ; tout autre cas → `None` (sortie vide).
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
