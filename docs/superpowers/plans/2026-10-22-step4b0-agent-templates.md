# Étape 4, lot B0 — modèles d'agents : plan d'implémentation

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `.armadai/templates/<id>.md` + agents `from:`/`vars:` rendus au chargement, pour que tout l'aval (adaptateurs, gate, graph) voie des agents ordinaires.

**Architecture:** tout se passe dans `crates/armadai-model` : le chargeur lit les modèles, rend chaque instance (substitution dans les valeurs, fusion clé par clé, corps à la suite) puis l'analyse comme un agent typé ; l'agent rendu garde sa forme d'instance pour l'écrivain. La CLI (`list`, `inspect`, `new agent --from`) et l'installation de pack s'adaptent.

**Tech Stack:** Rust 2024, serde + serde-saphyr, serde_json, schemars.

**Spec:** `docs/superpowers/specs/2026-10-22-step4b0-agent-templates-design.md`

## Global Constraints

- MVP (décision de Dimitri) : les non-objectifs de la spec §2 ne sont pas implémentés, même partiellement.
- `crates/armadai-model` reste pur : serde, serde-saphyr, serde_json, thiserror, schemars seulement.
- Variable : `{{nom}}` ou `{{ nom }}`, `nom` valide pour `is_valid_id` ; `\{{` → `{{` littéral, jamais une variable.
- Substitution dans le corps et dans les valeurs chaînes du frontmatter (y compris imbriquées, y compris `native`), jamais dans les clés, jamais sur le texte YAML.
- Fusion : clé de premier niveau de l'instance remplace celle du modèle ; corps de l'instance non vide ajouté après le corps rendu du modèle, séparé par exactement une ligne vide.
- Codes : `unknown-template`, `missing-template-var`, `unused-template-var` (avertissement), `vars-without-template`, `nested-template`, `template-agent-collision`, `bad-id`.
- Le nouveau module ne s'appelle pas `templates.rs` (déjà pris par les modèles de flows) : `agent_templates.rs`.
- Tests à égalité exacte ; une mutation vue rouge par test ajouté (restaurer puis `touch`) ; `crates/armadai` binaire seul (`--bin armadai` / `--test <nom>`), toujours `--no-fail-fast`.
- Barrière : `cargo fmt --all -- --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --no-fail-fast`, `cargo build -p armadai-core`, `cargo build -p armadai-model`, `(cd frozen/armadai-audit && cargo check --all-targets)`.

## Review Focus

1. Une valeur de variable qui contient `{{autre}}` : insérée telle quelle, **pas** rendue une seconde fois (pas de rendu récursif) — test en tâche 1.
2. Une instance qui redéfinit `capabilities: { write: allow }` alors que le modèle a `{ write: deny, shell: deny }` : le résultat est `{ write: allow }` seul (remplacement de la clé entière, comme la spec le dit) — test en tâche 1, et la doc le dit en clair.
3. Un modèle modifié : le `link` suivant réécrit les agents de ses instances sans `--force` (les fichiers générés n'ont pas changé à la main) — test en tâche 4.
4. `new flow` ou `new skill` qui réécrit le projet : les instances restent des instances (aucune commande n'aplatit) — test en tâche 3.
5. Un fichier `templates/x.txt` ou un sous-dossier : ignoré comme dans `agents/` — test en tâche 1.

---

### Task 1 : chargement et rendu

**Files:**
- Create: `crates/armadai-model/src/agent_templates.rs` (exporté par `lib.rs`)
- Modify: `crates/armadai-model/src/load.rs` (`Kind::Template`, `Project.templates`, rendu des agents `from:`), `crates/armadai-model/src/types.rs` (`TemplateFront`, `from`/`vars` d'`AgentFront`)
- Modify: tout constructeur littéral de `Agent`, `AgentFront`, `Project` du workspace qui ne compile plus

**Interfaces:**
- Produces :
  - `pub struct Template { pub front: TemplateFront, pub body: String }` ; `TemplateFront` = les champs d'`AgentFront` tous facultatifs (même `deny_unknown_fields`), sans `from` ni `vars`.
  - `Project.templates: BTreeMap<String, Template>` ; `Kind::Template` (origines et diagnostics).
  - `AgentFront.from: Option<String>`, `AgentFront.vars: BTreeMap<String, String>` (omis à l'écriture quand vides) — **toujours vides dans un agent rendu** ;
  - `load::Agent.instance: Option<Instance>` avec `pub struct Instance { pub from: String, pub vars: BTreeMap<String, String>, pub front: serde_json::Map<String, serde_json::Value>, pub body: String }` (les clés propres à l'instance hors `from`/`vars`, et son propre corps) ; `None` pour un agent écrit à la main.
  - `pub fn render(text: &str, vars: &BTreeMap<String, String>) -> Result<String, Vec<String>>` dans `agent_templates.rs` : renvoie les noms de variables manquantes ; et `pub fn variables(text: &str) -> BTreeSet<String>`.

- [ ] **Step 1 :** tests de `render`/`variables` : `{{a}}` et `{{ a }}` remplacés ; `\{{a}}` → `{{a}}` littéral et `a` absent de `variables` ; valeur `"x {{b}}"` insérée telle quelle (Review Focus 1) ; `{{Bad Name}}` laissé tel quel et non compté comme variable ; manquante → `Err(vec!["a"])`.
- [ ] **Step 2 :** tests de chargement (projet temporaire) :
  - `templates/checker.md` (`description: "Checks {{subject}}"`, `tier: pro`, `capabilities: { write: deny, shell: deny }`, corps `You check {{subject}}.\n`) + `agents/guardian.md` (`from: checker`, `vars: { subject: the frame }`, `skills: [cadre]`, corps `Extra line.\n`) → `project.agents["guardian"]` a `description == "Checks the frame"`, `tier == Some(Pro)`, `skills == ["cadre"]`, `body == "You check the frame.\n\nExtra line.\n"`, `from == None`, `vars` vide, `instance.is_some()`.
  - instance `capabilities: { write: allow }` → `capabilities` = `{ write: allow }` seul (Review Focus 2).
  - valeur `"a: b\nc"` → reste une chaîne, `description == "Checks a: b\nc"`.
  - chaque code de la spec §5 avec son message exact (dont `unknown-template` avec suggestion `did you mean`, comme `unknown-agent`) ; `unused-template-var` est un avertissement et l'agent est quand même chargé.
  - `templates/x.txt` et `templates/sub/` ignorés comme dans `agents/` (Review Focus 5).
- [ ] **Step 3 :** rouge, implémenter (frontmatter d'instance lu en YAML générique ; si `from` : rendu des valeurs du modèle et de l'instance, fusion, puis analyse typée `AgentFront` sur le résultat ; sinon chemin actuel), vert.
- [ ] **Step 4 :** barrière ; commit `feat(model): agent templates rendered at load time`.

### Task 2 : écriture, validation, schémas

**Files:**
- Modify: `crates/armadai-model/src/write.rs`, `crates/armadai-model/src/check.rs`, `crates/armadai-model/src/schema.rs`, `schemas/v1/` (régénérés), `crates/armadai-model/tests/roundtrip.rs`

**Interfaces:**
- Consumes : tâche 1.
- Produces : `pub fn render_template(t: &Template) -> String` ; `render_agent` écrit la forme d'instance quand `agent.instance` est `Some` ; `write_project` écrit `templates/<id>.md` ; schéma `template.schema.json`.

- [ ] **Step 1 :** tests : aller-retour `load` → `write_project` → `load` d'un projet avec un modèle et deux instances, fichiers identiques octet pour octet à une écriture canonique ; `render_agent` d'une instance contient `from: checker` et `vars:` et **pas** la description rendue ; `template-agent-collision` (modèle et agent `guardian`) ; un modèle utilisé comme étape de flow ou comme `coordinator` → `unknown-agent` existant (aucun nouveau code).
- [ ] **Step 2 :** schémas régénérés (`cargo run -p armadai-model --example gen_schemas`) ; `agent.schema.json` n'exige plus `description` quand `from` est présent — si schemars ne l'exprime pas simplement, rendre `description` facultative dans le schéma seul et le dire dans le rapport (la validation sémantique l'exige toujours après rendu).
- [ ] **Step 3 :** rouge, implémenter, vert ; barrière ; commit `feat(model): write and validate agent templates`.

### Task 3 : CLI et packs

**Files:**
- Modify: `crates/armadai/src/cli/{mod,new,new_target,new_wizard,list,inspect,native_pack}.rs`
- Test: tests unitaires de ces modules, `crates/armadai/tests/` pour `new` et `init --pack`

**Interfaces:**
- Consumes : tâches 1 et 2.

- [ ] **Step 1 :** tests :
  - `armadai new agent guardian --from checker --var subject=the frame` écrit `.armadai/agents/guardian.md` = `---\nfrom: checker\nvars:\n  subject: the frame\n---\n` ; `--var` répétable ; `--var` sans `=` refusé par la CLI ; modèle inconnu ou variable manquante → refus avec le message du diagnostic, rien écrit ; `--from` combiné à un gabarit intégré (`--template`) refusé.
  - le menu guidé propose « depuis un modèle du projet » seulement s'il y en a ; la fonction pure `plan_*` correspondante testée.
  - `list` : section `Templates` ; un agent instancié affiché `guardian (from checker)`.
  - `inspect guardian` : affiche l'agent rendu (description, corps rendus) et `from: checker`.
  - `new flow` puis `new skill` sur un projet avec une instance : `agents/guardian.md` inchangé octet pour octet (Review Focus 4).
  - `init --pack` d'un pack de test avec `templates/checker.md` et une instance : les deux installés (liste blanche : `["templates", f] => f.ends_with(".md")`).
- [ ] **Step 2 :** rouge, implémenter, vert ; barrière ; commits `feat(new): create an agent from a project template` et `feat(init): install the templates of a native pack`.

### Task 4 : équivalence, documentation

**Files:**
- Create: `crates/armadai/tests/agent_templates.rs`, deux fixtures `crates/armadai/tests/fixtures/templates-instances/` et `crates/armadai/tests/fixtures/templates-flattened/` (le même projet, agents écrits à la main avec le texte rendu)
- Modify: `docs/wiki/project-format.md`, `CHANGELOG.md` (`## Unreleased`, `### Added`)

- [ ] **Step 1 :** test `instances_link_exactly_like_their_flattened_agents` : pour chacune des 4 cibles, `link --dry-run --json` (ou le plan de compilation) des deux fixtures est identique, et les fichiers écrits par un vrai `link` dans deux dossiers temporaires sont identiques octet pour octet (hors `link-manifest.yaml`).
- [ ] **Step 2 :** test `a_changed_template_relinks_without_force` (Review Focus 3) : link, modification du corps du modèle, link → code 0, agent régénéré contient le nouveau texte.
- [ ] **Step 3 :** doc : format des modèles et des instances, règles de fusion (avec l'exemple `capabilities` remplacée entière), échappement `\{{`, codes, limites du MVP listées comme « reportées ».
- [ ] **Step 4 :** barrière ; commits `test(link): agent templates link like hand-written agents` et `docs: agent templates`.
