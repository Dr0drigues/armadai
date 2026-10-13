# Étape 3 — Création guidée, starters natifs, flows, Mermaid : plan d'implémentation

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal :** Rendre la création native au modèle canonique : starters natifs, `new agent|flow|automation|skill` (interactif ou non), gabarits de flows, `armadai graph` (Mermaid).

**Architecture :** logique pure (gabarits de flows, rendu Mermaid) dans `armadai-model` ; commandes dans `crates/armadai/src/cli/` ; écriture via `armadai_model::{render_agent, render_flow, render_automation, render_skill, write_project}` et validation par `armadai_model::validate` avec annulation (T8). Les packs vivent dans `crates/armadai-core/starters/` (embarqués) au format natif ; seul un pack distant à l'ancien format passe encore par le convertisseur de `migrate`.

**Tech Stack :** Rust 2024, `armadai-model`, `dialoguer`, `include_dir`, `clap`.

**Spec :** `docs/superpowers/specs/2026-10-13-step3-guided-creation-starters-design.md` (lire en entier ; §2 = T1–T9).

## Global Constraints

- Barrière avant chaque commit : `cargo fmt --all -- --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --no-fail-fast`, `cargo build -p armadai-core`, `cargo build -p armadai-model`, `(cd frozen/armadai-audit && cargo check --all-targets)` avec `CARGO_TARGET_DIR` hors dépôt.
- `crates/armadai` est binaire : tests unitaires en `#[cfg(test)] mod tests` ; tests de commandes dans `tests/*.rs` via `assert_cmd` avec `ARMADAI_CONFIG_DIR` et `HOME` isolés. Fixtures sous `.armadai/` : exceptions `.gitignore` explicites, `git ls-files` vérifié.
- Conventional Commits, un type par sujet ; trailer `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>` ; code et commits en anglais, wiki en anglais.
- TDD réel (test rouge vu avant d'implémenter) ; mutation par test (casser, voir rouge, restaurer par copie — jamais en préservant la mtime) ; pas de `#[allow(...)]` ajouté ; `git status` propre en fin de tâche.
- **Écritures sûres** : toute écriture de fichier issue d'un id fourni par l'utilisateur passe par une validation d'id (`[a-z0-9][a-z0-9-]{0,63}`), refuse l'écrasement sans `--force`, et vérifie que le chemin réel reste sous `.armadai/` du projet (pas de lien symbolique sortant) — mêmes gardes que `link` (`compile::check_destination` à réutiliser ou factoriser). Les revues précédentes ont trouvé à chaque tour des défauts de cette famille.
- Aucun accès réseau pour les commandes de création.
- `armadai-core` : aucune signature publique supprimée sans preuve `grep` de l'absence d'usage.

## Review Focus

1. Id d'agent/flow/skill hostile (`../x`, `a/b`, `.git`, majuscules, accents, vide, très long, nom réservé Windows) : refusé ou normalisé de façon documentée, jamais d'écriture hors `.armadai/`.
2. Annulation (T8) : si la validation échoue après écriture, l'arbre revient exactement à l'état initial (y compris dossiers créés) ; ne supprime jamais un fichier préexistant.
3. Mermaid : ids et descriptions avec `"`, `[`, `]`, `(`, `)`, `-`, unicode, mots réservés Mermaid (`end`, `graph`, `subgraph`) : sortie rendable ; flow sans lien ; boucle ; `failed` ; deux flows.
4. `init --pack` sur un projet existant (fichiers déjà présents, `project.yaml` déjà là), pack inconnu, pack dont un agent est invalide, pack distant ancien format.
5. Gabarits de flows avec des agents dupliqués, un seul agent, des ids inexistants dans le projet : erreurs claires, aucun flow invalide écrit.

---

### Task 1 : Gabarits de flows et rendu Mermaid (`armadai-model`)

**Files:** Create `crates/armadai-model/src/templates.rs`, `crates/armadai-model/src/mermaid.rs`, tests `crates/armadai-model/tests/templates.rs`, `tests/mermaid.rs` ; Modify `src/lib.rs`.

**Interfaces — Produces :**
- `pub enum FlowTemplate { Pipeline, ReviewLoop, PlanBuildVerify }` (`FromStr`/`as_str` : `pipeline`, `review-loop`, `plan-build-verify`) ; `pub const FLOW_TEMPLATES: [FlowTemplate; 3]` ;
- `pub fn flow_from_template(t: FlowTemplate, agents: &[String], description: Option<String>) -> Result<Flow, TemplateError>` ; `TemplateError { TooFewAgents{ template, min }, TooManyAgents{ template, max }, DuplicateAgent(String), InvalidAgentId(String) }` (`thiserror`, messages anglais) ; ids d'étapes = ids d'agents ; formes : cf. spec §3.3 (review-loop : étapes `auteur`, `relecteur` avec `states: [approved, rejected]`, liens `relecteur → auteur when rejected`, `relecteur → livreur when approved`) ;
- `pub fn flow_to_mermaid(id: &str, flow: &Flow) -> String` : `flowchart TD`, déterministe ; nœud = id assaini (`[^A-Za-z0-9_]` → `_`, préfixé `n_` si commence par un chiffre ou égale à un mot réservé Mermaid `end|graph|subgraph|flowchart|style|class|click|default`), étiquette `["step (agent)"]` avec `"` → `#quot;`, `[`/`]`/`(`/`)` conservés car entre guillemets ; arête `a --> b`, `a -- when --> b` (when ≠ `done`), `a -. failed .-> b` pour `failed` et `on_failure` ; collisions de nœuds après assainissement départagées par suffixe `_2`.

- [ ] **Step 1 : tests d'abord** : chaque gabarit produit un `Flow` qui passe `validate` sans diagnostic (via projet temporaire avec les agents) ; erreurs de nombre/duplicat/id invalide ; mermaid : étalons exacts (pipeline 3 agents, review-loop avec boucle, `on_failure`), ids piégeux (`end`, `1-a`, `a b`, unicode), étiquette avec guillemets, flow vide, déterminisme (deux appels = mêmes octets).
- [ ] **Step 2–4 :** rouge, implémenter, vert. **Step 5 :** mutations (inverser `when`, perdre l'assainissement, ordre des liens, `failed` en trait plein). **Step 6 :** barrière + commit `feat(model): add flow templates and a Mermaid export`.

---

### Task 2 : `armadai graph`

**Files:** Create `crates/armadai/src/cli/graph.rs`, `crates/armadai/tests/graph_cli.rs` ; Modify `cli/mod.rs`.

**Interfaces — Consumes :** Task 1, `armadai_model::load_checked`. **Produces :** `armadai graph [FLOW] [--format mermaid]` : stdout = uniquement le Mermaid (un flow, ou tous séparés par une ligne vide et un commentaire `%% flow: <id>`) ; flow inconnu = erreur listant les flows ; aucun projet ou ancien format = erreur claire (`run armadai migrate`) ; projet invalide = diagnostics sur stderr et code 1 ; recherche du projet comme `list`/`inspect` (lire `cli/list.rs`).

- [ ] **Step 1 : tests d'abord** (e2e `assert_cmd`) : un flow ; tous les flows ; flow inconnu ; hors projet ; ancien format ; sortie pure (rien d'autre sur stdout).
- [ ] **Step 2–6 :** comme Task 1 ; mutations (écrire un en-tête sur stdout, mauvais flow, code de sortie). Commit `feat(cli): add armadai graph`.

---

### Task 3 : modèles d'agents natifs et `new agent|flow|automation|skill` (non interactif)

**Files:** Rewrite `crates/armadai/templates/*.md` (12 modèles) ; Modify `crates/armadai/src/cli/new.rs`, `cli/mod.rs`, `tests/new_templates.rs` ; tests `tests/new_cli.rs`.

**Interfaces — Produces :**
- Modèles d'agents au format canonique : frontmatter `description: "{{description}}"`, `tier: pro|fast|max`, `tags`, corps = ancien prompt système + instructions + format de sortie composés (relire chaque ancien modèle : aucun contenu perdu, `## Pipeline` retiré) ; placeholders `{{name}}`, `{{description}}`, `{{stack}}` rendus avec `armadai_core::template::render_lenient` (existant) ; priorité au dossier de modèles du disque conservée ; le test « l'ensemble embarqué égale le dossier » reste.
- `armadai new agent [ID] [--template T] [--description D] [--stack S] [--tier T] [--skills a,b] [--force]` ; `new flow [ID] --template T --agents a,b,c [--description D] [--force]` (agents inconnus du projet = erreur) ; `new automation [ID] --event E --run CMD [--description D] [--force]` ; `new skill [ID] --description D [--force]` ; `armadai new` sans sous-commande = Task 4 (ici : aide).
- Garde commune `NewTarget::resolve(kind, id, force)` : valide l'id (normalise en kebab-case si l'entrée est un titre — « Mon Agent » → `mon-agent`, avec message —, refuse `..`, `/`, vide, > 64), exige un projet canonique (`.armadai/project.yaml`, sinon « run `armadai init --project` »), refuse l'écrasement sans `--force`, vérifie le chemin réel sous `.armadai/` (réutiliser la garde de `link`).
- **T8 annulation** : après écriture, `armadai_model::validate` ; s'il y a une erreur, supprimer exactement ce qui a été créé (fichiers et dossiers créés par la commande, jamais un préexistant) et rendre les diagnostics.
- L'ancienne forme `new <nom>` : erreur claire « use `armadai new agent <nom>` ».
- Supprimer le convertisseur hérité de `new` (`write_canonical_agent`, passage par `migrate::run`) — il n'est plus appelé (grep).

- [ ] **Step 1 : tests d'abord** : chaque modèle (12) → `new agent --template X` valide sans erreur et sans avertissement ; collision ; `--force` ; ids hostiles (Review Focus 1) ; hors projet ; flow avec agent inconnu ; gabarits ; automation (events valides/invalides, `run` vide) ; skill ; annulation (injecter une erreur de validation : par ex. un modèle disque invalide) avec comparaison d'arbres ; ancienne forme refusée ; aucun `## Pipeline` dans les modèles (test de grep sur les fichiers).
- [ ] **Step 2–4 :** rouge, implémenter, vert. **Step 5 :** mutations (écraser sans `--force`, oublier la validation d'id, ne pas annuler, supprimer un préexistant). **Step 6 :** barrière + commits `refactor(templates): rewrite agent templates in the canonical format`, `feat(cli): add new agent, flow, automation and skill`.

---

### Task 4 : assistants interactifs

**Files:** Modify `crates/armadai/src/cli/new.rs` (ou nouveau `cli/new_wizard.rs`), tests.

**Interfaces :** `armadai new` (sans sous-commande) et `new <kind> -i` : questions `dialoguer` ; **toute la logique de décision est dans des fonctions pures** `plan_agent(answers) -> AgentSpec`, `plan_flow(answers) -> FlowSpec` testables sans terminal ; la couche `dialoguer` ne fait que collecter les `answers`. Agent : id, description, modèle de départ (liste des modèles embarqués), palier, skills du projet (multi-sélection), capacités (5 questions allow/ask/deny avec défaut « laisser par défaut »). Flow : gabarit (liste, description de chacun), agents (multi-sélection ordonnée parmi ceux du projet), aperçu Mermaid, confirmation. Automation : événement (liste), commande. Pas de TTY (`stdin` non terminal) = erreur claire « interactive mode needs a terminal; pass the options ».

- [ ] **Step 1 : tests d'abord** sur les fonctions pures (`plan_*`) : réponses → fichier attendu, capacités « par défaut » = absentes, ordre des agents préservé ; test e2e « sans TTY » ; l'aperçu Mermaid est celui de la Task 1.
- [ ] **Step 2–6 :** comme ci-dessus ; mutations sur `plan_*`. Commit `feat(cli): add interactive creation wizards`.

---

### Task 5 : starters natifs et `init --pack`

**Files:** Rewrite `crates/armadai-core/starters/*` (7 packs) ; Modify `crates/armadai-core/src/starter.rs`, `pack_validation.rs`, `crates/armadai/src/cli/init.rs`, `tests/list_inspect_init.rs` ; tests `tests/init_pack.rs` ; fixtures sous `crates/armadai/tests/fixtures/legacy-pack/` (faux pack distant ancien format, `.gitignore` si besoin).

**Interfaces :**
- Réécriture : pour chaque pack, produire le format natif en partant de la sortie de `armadai migrate` du pack actuel (outil : `init --pack` actuel puis copie), **relue agent par agent** (descriptions soignées plutôt que dérivées, `tier`, `tags`, flows/policy/automations pertinents pour `fullstack` et `armadai-dev`), retirer les 3 agents CLI (T3) et `prompts/` (les prompts autonomes deviennent des skills ou sont fusionnés) ; chaque pack garde son `pack.yaml` (`name`, `description`) — le champ `agents:` n'est plus lu.
- `StarterPack` : charge le nouveau `pack.yaml` ; détection `PackKind::{Native, Legacy}` (Legacy = `pack.yaml` avec `agents:`/`prompts:` et sans `agents/*.md` à frontmatter ; décider par l'absence de `schema_version` dans `project.yaml` ET présence de la liste `agents:`).
- `init --pack` : `Native` → copie sous `.armadai/` (sans écraser, avertissement par fichier ignoré), `project.yaml` par défaut si absent, `.armadai/.gitignore`, validation puis annulation si erreur (T8) ; `Legacy` → chemin actuel (matérialisation + `migrate::run`) avec une ligne « legacy pack converted ». Gardes de chemins : un pack distant peut contenir des liens symboliques ou `..` : refuser (réutiliser la garde de lecture de `compile::read_tree`).
- `validate` mode pack : valide un pack natif (structure + `validate` de l'arbre) ; mode pack ancien format inchangé.

- [ ] **Step 1 : tests d'abord** : pour chacun des 7 packs embarqués : `init --pack` → `validate` sans erreur et sans avertissement, `link --target claude` réussit ; contenu des agents non vide ; pack distant ancien format installable (fixture) ; pack natif invalide refusé avec diagnostics et arbre inchangé ; projet existant ; pack inconnu ; symlink dans un pack distant refusé ; plus aucun agent CLI dans `fullstack`.
- [ ] **Step 2–4 :** rouge, implémenter, vert. **Step 5 :** mutations. **Step 6 :** barrière + commits `feat(starters): rewrite the embedded packs in the canonical format`, `feat(init): install native packs by copy`.

---

### Task 6 : étalons, documentation, nettoyage

**Files:** `tests/golden/init_pack/*` (arbres produits par `init --pack` pour les 7 packs), wiki (`getting-started.md`, `templates.md`, `starter-packs.md`, `project-format.md`, nouvelle page `graph.md` ou section dans `project-format.md`, `SUMMARY.md`), `CHANGELOG.md`, `CLAUDE.md`.

- [ ] **Step 1 : étalons** `UPDATE_GOLDEN=1` (même mécanisme que `migrate_golden`) ; relire chaque pack étalon à la main et noter ce qui est vérifié.
- [ ] **Step 2 : nettoyage** : `grep` des usages de `migrate::{convert,run}` hors `migrate` et du chemin pack ancien format ; supprimer ce qui n'a plus d'appelant (preuve avant/après dans le message de commit) ; `armadai-core::starter` : retirer les champs de `pack.yaml` hérités s'ils ne servent plus qu'au chemin legacy (sinon les garder et le dire).
- [ ] **Step 3 : docs** : guide « Créer son projet » (init, new agent/flow, graph, link), table des gabarits de flows avec leur Mermaid, format du pack natif, CHANGELOG (Changed : `new` en sous-commandes, starters natifs ; Removed : 3 agents CLI, `## Pipeline`), CLAUDE.md.
- [ ] **Step 4 :** barrière complète + vérifs transverses ; commits `test(init): golden snapshots of the embedded packs`, `docs: document creation, starters and graph`.

---

## Auto-relecture
- Couverture de la spec : T1 (Task 5), T2 (Task 5), T3 (Task 5, Task 3), T4 (Task 3), T5 (Task 3), T6 (Task 1), T7 (Tasks 1–2), T8 (Tasks 3, 5), T9 (Task 4) ; critères 1–2 (Task 5), 3 (Tasks 3–4), 4 (Task 1), 5 (Task 2), 6 (Task 3), 7 (Task 6), 8 (Task 6).
- Ordre : Task 1 (pur) avant 2 et 3 ; Task 5 après 3 car `init --pack` réutilise la garde et l'annulation de `new`.
