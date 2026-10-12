# Étape 1 — Modèle canonique, validateur et `migrate` : plan d'implémentation

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal :** Livrer le crate pur `armadai-model` (types stricts, chargeur, écrivain canonique, validateur positionné, JSON Schema), `armadai validate` sur le nouveau format et `armadai migrate`.

**Architecture :** `crates/armadai-model` ne dépend d'aucun autre crate du workspace (ni core, ni clap, ni tokio). La conversion ancien → nouveau vit dans le binaire (`crates/armadai/src/migrate/`), seul consommateur nouveau de `armadai-core`. Le validateur a deux couches : structure (serde strict, positions via `serde_yaml_ng::Error::location`) puis sémantique (table des symboles ; position = première occurrence du jeton fautif dans le fichier, voir Task 3).

**Tech Stack :** Rust édition 2024, `serde`, `serde_yaml_ng 0.10`, `thiserror 2`, `schemars 1.2.2` (déjà vérifiée disponible sur crates.io), `serde_json`, `tempfile` + `assert_cmd` en dev.

**Spec :** `docs/superpowers/specs/2026-10-12-step1-model-validator-migrate-design.md` (à lire avant toute tâche ; §3 = choix C1–C11, §4 = formats, §5 = validateur, §6 = migrate).

## Global Constraints

- Barrière avant chaque commit : `cargo fmt --all -- --check`, `cargo clippy --all-targets --no-default-features -- -D warnings`, `cargo test --no-fail-fast --no-default-features`, `cargo build -p armadai-core`, `cargo build -p armadai-model`.
- `crates/armadai` est un crate binaire : `cargo test --lib` y rend « 0 passed » sans erreur ; utiliser `--bin armadai` ou `--test <nom>`.
- Conventional Commits, un seul type par sujet ; trailer `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`. Commits et code en anglais.
- `armadai-model` : dépendances autorisées = serde, serde_yaml_ng, serde_json, thiserror, schemars. Aucune autre ; pas de `unsafe`, pas d'`#[allow(...)]` ajouté.
- `armadai-core` : aucune signature existante ne change (le crate gelé `frozen/armadai-audit` en dépend). Seuls des ajouts `pub` sont tolérés.
- Cibles supportées : `claude`, `codex`, `copilot`, `opencode` (liste unique `armadai_model::TARGETS`).
- Id : `[a-z0-9][a-z0-9-]{0,63}`. `schema_version` : entier, seule valeur acceptée `1`.
- Mutation obligatoire pour chaque test ajouté : casser le code testé, constater le rouge, restaurer (jamais `touch`/préserver la mtime d'un fichier restauré : `cargo` garderait la rlib mutée). Noter la mutation dans le message de commit ou le rapport.
- Pas de tâche en arrière-plan, pas de `sleep` en avant-plan.
- **État transitoire assumé** : après `armadai migrate`, le projet est au nouveau format ; `link` ne le lit pas encore (étape 2). `migrate` le dit dans son rapport. Le projet ancien reste intact dans `.armadai/.migrated/<horodatage>/`.
- Horodatage de sauvegarde = secondes Unix (`std::time`), pas de dépendance `chrono`.

## Review Focus

1. Frontmatter d'agent absent, vide, non fermé (`---` sans seconde barre) ou avec fins de ligne CRLF : diagnostic positionné, jamais de panique.
2. Clés YAML dupliquées, tabulations, fichier vide ou binaire dans `flows/`, `automations/`, `project.yaml` : erreur claire avec ligne.
3. Id d'agent collisionnant à la migration (deux H1 identiques, H1 non ASCII ou vide) : suffixe numérique signalé, jamais d'écrasement silencieux.
4. `migrate` interrompu ou relancé : refus sans `--force` ; sauvegarde déjà faite avant toute écriture ; `--dry-run` n'écrit rien (arbre comparé).
5. Ancien projet sans agents, avec agent de bibliothèque utilisateur absent, ou avec `orchestration:` référant un agent inconnu : rapport d'avertissements, pas d'abandon silencieux ni de flow invalide.

---

## Structure des fichiers

| Fichier | Responsabilité |
|---|---|
| `crates/armadai-model/Cargo.toml` | Manifeste du crate pur |
| `crates/armadai-model/src/lib.rs` | Ré-exports, `TARGETS`, `SCHEMA_VERSION` |
| `crates/armadai-model/src/types.rs` | Types strictes (Manifest, AgentFront, Flow, Automation, Capabilities, Native…) |
| `crates/armadai-model/src/diag.rs` | `Diagnostic`, `Severity`, rendu texte |
| `crates/armadai-model/src/load.rs` | Chargement du dossier, frontmatter, expansion `sequence`, `Project`, `Origins` |
| `crates/armadai-model/src/check.rs` | Couche sémantique (table des symboles) |
| `crates/armadai-model/src/write.rs` | Écriture canonique |
| `crates/armadai-model/src/schema.rs` | Génération des JSON Schemas |
| `crates/armadai-model/tests/` | Fixtures et tests par code, aller-retour, schéma |
| `schemas/v1/*.schema.json` | Schémas générés et committés |
| `crates/armadai/src/cli/validate.rs` | Mode nouveau/ancien |
| `crates/armadai/src/migrate/{mod,read,convert,report,write}.rs` | Conversion |
| `crates/armadai/src/cli/migrate.rs` | Commande |
| `crates/armadai/tests/migrate_golden.rs` + `tests/fixtures/migrate/*` | Étalons |

---

### Task 1 : Squelette du crate et types stricts

**Files:**
- Create: `crates/armadai-model/Cargo.toml`, `src/lib.rs`, `src/types.rs`
- Modify: `Cargo.toml` (workspace deps : `schemars = "1.2.2"`), `.github/workflows/ci.yml` (garde `cargo build -p armadai-model` à côté de celui de core)

**Interfaces:**
- Produces (types, tous `Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema`, `#[serde(deny_unknown_fields)]` sauf mention) :
  - `pub const SCHEMA_VERSION: u32 = 1; pub const TARGETS: [&str; 4]`
  - `Manifest { schema_version: u32, name: Option<String>, targets: Vec<String>, outputs: BTreeMap<String,String>, coordinator: Option<String>, policy: Policy, native: Native }`
  - `Policy { delegation: Delegation, free_agents: Vec<String> }`, `Delegation { Off (défaut), Strict }` (`lowercase`)
  - `Native = BTreeMap<String, serde_yaml_ng::Value>` (schéma : `BTreeMap<String, serde_json::Value>` via `#[schemars(with=…)]`)
  - `Tier { Fast, Pro, Max }`, `Cap { Allow, Ask, Deny }` (`lowercase`)
  - `Capabilities { read, write, shell, web, delegate: Option<Cap> }`
  - `AgentFront { description: String, title: Option<String>, tier: Option<Tier>, temperature: Option<f32>, tags: Vec<String>, skills: Vec<String>, capabilities: Capabilities, native: Native }`
  - `Step { agent: String, states: Vec<String>, on_failure: Option<String> }`, `Link { from: String, to: String, when: Option<String> }`
  - `FlowFile { description: Option<String>, steps: BTreeMap<String,Step>, links: Vec<Link>, sequence: Vec<SeqItem> }` (forme brute lue) ; `SeqItem` untagged : `Agent(String)` | `Explicit { id: String, agent: String }`
  - `Event { SessionStart, SessionEnd, BeforeTool, AfterTool, OnPrompt, OnStop }` (`kebab-case`), `Automation { event: Event, run: String, description: Option<String>, native: Native }`
  - `SkillFront { name: Option<String>, description: Option<String>, license, compatibility: Option<String>, metadata: Option<BTreeMap<String,String>> }` — **sans** `deny_unknown_fields` (clés inconnues = avertissement, calculées par le chargeur sur la `Value` brute).
  - Valeurs par défaut : tous les `Vec`/`BTreeMap`/`Option` et `policy`, `native` sont `#[serde(default)]` ; `skip_serializing_if` sur les `Option` et collections vides pour une écriture canonique minimale.

- [ ] **Step 1 : Écrire le test d'abord** (`crates/armadai-model/src/types.rs`, module `tests`)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn parse<T: serde::de::DeserializeOwned>(y: &str) -> Result<T, serde_yaml_ng::Error> {
        serde_yaml_ng::from_str(y)
    }

    #[test]
    fn manifest_minimal_roundtrips() {
        let m: Manifest = parse("schema_version: 1\n").unwrap();
        assert_eq!(m.policy.delegation, Delegation::Off);
        let out = serde_yaml_ng::to_string(&m).unwrap();
        assert_eq!(out, "schema_version: 1\n");
    }

    #[test]
    fn unknown_manifest_key_is_rejected() {
        let e = parse::<Manifest>("schema_version: 1\nbogus: 1\n").unwrap_err();
        assert!(e.to_string().contains("unknown field `bogus`"), "{e}");
    }

    #[test]
    fn native_accepts_free_yaml() {
        let m: Manifest = parse("schema_version: 1\nnative:\n  claude:\n    anything: [1, 2]\n").unwrap();
        assert!(m.native.contains_key("claude"));
    }

    #[test]
    fn duplicate_keys_are_rejected() {
        assert!(parse::<Manifest>("schema_version: 1\nschema_version: 1\n").is_err());
    }

    #[test]
    fn capability_value_outside_domain_is_rejected() {
        assert!(parse::<Capabilities>("read: maybe\n").is_err());
    }
}
```

- [ ] **Step 2 : Lancer, constater l'échec de compilation** — `cargo test -p armadai-model` → `error: package ID specification armadai-model matches no packages`.
- [ ] **Step 3 : Créer le crate** : `Cargo.toml` (`name = "armadai-model"`, `edition.workspace = true`, `license.workspace = true`, deps `serde`, `serde_json`, `serde_yaml_ng`, `thiserror`, `schemars` toutes `workspace = true` ; dev-deps `tempfile`), `lib.rs` (`pub mod types; pub use types::*;` + constantes), `types.rs` selon les Interfaces. Ajouter `schemars = "1.2.2"` à `[workspace.dependencies]`.
- [ ] **Step 4 : Lancer** `cargo test -p armadai-model` → 5 passed. `cargo tree -p armadai-model --edges normal | grep -E "clap|tokio|reqwest|armadai-core"` → aucune ligne.
- [ ] **Step 5 : Mutation** : retirer `deny_unknown_fields` de `Manifest` → `unknown_manifest_key_is_rejected` rouge ; restaurer. Changer `Delegation` défaut en `Strict` → `manifest_minimal_roundtrips` rouge ; restaurer.
- [ ] **Step 6 : Garde CI** : ajouter dans `ci.yml`, au job qui exécute `cargo build -p armadai-core`, la ligne `cargo build -p armadai-model`.
- [ ] **Step 7 : Barrière + commit** `feat(model): add the armadai-model crate with strict canonical types`

---

### Task 2 : Diagnostics et chargeur

**Files:**
- Create: `crates/armadai-model/src/diag.rs`, `src/load.rs`, `tests/load.rs`, `tests/fixtures/valid/…`
- Modify: `src/lib.rs`

**Interfaces:**
- Consumes : types de la Task 1.
- Produces :
  - `Severity { Error, Warning }` ; `Diagnostic { severity, code: &'static str, file: PathBuf, path: String, line: Option<usize>, column: Option<usize>, message: String, hint: Option<String> }` ; `impl Diagnostic { pub fn render(&self) -> String }` → `fichier:ligne:colonne: error[code]: message` (+ `\n  hint: …`) ; sans position : `fichier: error[code]: …`.
  - `pub struct Agent { pub front: AgentFront, pub body: String }`, `Skill { front: SkillFront, body: String }`, `Flow { description: Option<String>, steps: BTreeMap<String,Step>, links: Vec<Link> }` (forme canonique : `sequence` développée).
  - `pub struct Project { pub manifest: Manifest, pub agents: BTreeMap<String,Agent>, pub skills: BTreeMap<String,Skill>, pub flows: BTreeMap<String,Flow>, pub automations: BTreeMap<String,Automation> }` (`PartialEq`).
  - `pub struct Origins { pub files: BTreeMap<(Kind,String), (PathBuf, String)> }` (chemin + texte source) ; `Kind { Manifest, Agent, Skill, Flow, Automation }`.
  - `pub struct Loaded { pub project: Option<Project>, pub origins: Origins, pub diagnostics: Vec<Diagnostic> }`
  - `pub fn load_project(root: &Path) -> Loaded` (`root` = dossier contenant `.armadai/`). `project` est `None` seulement si `project.yaml` est absent/illisible ; sinon le chargeur continue et accumule les diagnostics (un fichier invalide est omis du `Project`).
  - `pub fn split_frontmatter(text: &str) -> Option<(&str /*yaml*/, &str /*body*/, usize /*ligne du 1er caractère du yaml, 1-based*/)>` ; gère `\r\n` ; `None` si pas de `---` initial ou pas de fermeture.
  - `pub fn expand_sequence(items: &[SeqItem]) -> (BTreeMap<String,Step>, Vec<Link>)` : chaque `Agent(a)` reçoit l'id d'étape `a`, suffixé `-2`, `-3`… si répété ; `Explicit{id,agent}` garde `id` ; liens `{from: n, to: n+1}` consécutifs.
- Codes émis ici : `missing-manifest`, `invalid-yaml` (erreur de syntaxe ou de structure serde, avec ligne/colonne ; pour un agent, ligne décalée de `first_line - 1`), `missing-frontmatter`, `unclosed-frontmatter`, `unsupported-schema-version`, `sequence-and-steps`, `unknown-skill-key` (warning).
- Découverte : `.armadai/agents/*.md`, `.armadai/skills/*/SKILL.md`, `.armadai/flows/*.yaml`, `.armadai/automations/*.yaml`, tri alphabétique ; l'id est le nom de fichier (sans extension) ou du dossier ; un fichier d'une autre extension est ignoré.

- [ ] **Step 1 : Tests d'abord** (`tests/load.rs`), un test par code, chacun avec une fixture écrite dans un `tempfile::tempdir()` par un helper `project(files: &[(&str, &str)]) -> TempDir`. Exemple de test de position (à reproduire pour chaque code) :

```rust
#[test]
fn invalid_yaml_in_agent_frontmatter_points_at_the_real_line() {
    let dir = project(&[
        (".armadai/project.yaml", "schema_version: 1\n"),
        (".armadai/agents/dev.md", "---\ndescription: ok\ntier: huge\n---\nBody\n"),
    ]);
    let l = load_project(dir.path());
    let d = l.diagnostics.iter().find(|d| d.code == "invalid-yaml").unwrap();
    assert_eq!((d.line, d.severity), (Some(3), Severity::Error));
    assert!(d.file.ends_with(".armadai/agents/dev.md"));
}
```
Autres tests : `missing_manifest`, `unsupported_schema_version` (ligne 1), `missing_frontmatter`, `unclosed_frontmatter`, `crlf_frontmatter_loads` (agent en `\r\n` charge, corps sans `\r`… conserver tel quel ; test : le chargement réussit et `description` lue), `sequence_expands_to_graph` (3 agents → 2 liens ; répétition → `-2`), `sequence_and_steps_conflict`, `unknown_skill_key_warns`, `empty_file_is_an_error_not_a_panic` (flow vide, `project.yaml` vide), `discovery_is_sorted_and_ignores_other_extensions`.

- [ ] **Step 2 : Constater l'échec** (`cargo test -p armadai-model --test load` ne compile pas).
- [ ] **Step 3 : Implémenter** `diag.rs` et `load.rs`. Points exacts : (a) lire chaque fichier avec `std::fs::read_to_string` ; erreur d'E/S ou UTF-8 invalide = `Diagnostic` `invalid-yaml` sans position, jamais `unwrap` ; (b) erreur serde : `e.location()` → `(line, column)` ; pour le frontmatter ajouter `first_line - 1` ; (c) `schema_version != 1` = `unsupported-schema-version` ligne de la clé ; (d) clés inconnues d'un skill = clés de la `serde_yaml_ng::Value` brute absentes de `{name, description, license, compatibility, metadata}`.
- [ ] **Step 4 : Faire passer** `cargo test -p armadai-model` entier.
- [ ] **Step 5 : Mutations** (une par code) : retirer le décalage `first_line - 1` → test de position rouge ; ne pas gérer `\r\n` → `crlf_frontmatter_loads` rouge ; supprimer le tri → `discovery_is_sorted…` rouge ; accepter `schema_version: 2` → rouge.
- [ ] **Step 6 : Barrière + commit** `feat(model): load a .armadai directory with positioned diagnostics`

---

### Task 3 : Validateur sémantique

**Files:**
- Create: `crates/armadai-model/src/check.rs`, `tests/check.rs`
- Modify: `src/lib.rs`

**Interfaces:**
- Consumes : `Loaded` (Task 2).
- Produces : `pub fn validate(root: &Path) -> Vec<Diagnostic>` = diagnostics du chargeur + `check(&project, &origins)` ; `pub fn check(project: &Project, origins: &Origins) -> Vec<Diagnostic>` ; `pub fn has_errors(&[Diagnostic]) -> bool`.
- Localisation : `fn locate(text: &str, token: &str) -> (Option<usize>, Option<usize>)` — première occurrence du jeton comme mot entier (frontières = caractères hors `[A-Za-z0-9_-]`), ligne et colonne 1-based. Commentaire de code obligatoire : `// shortcut: first whole-word occurrence, not the exact YAML node; upgrade if a position-aware YAML parser is ever adopted.`
- Codes (tous avec test) :
  - erreurs : `bad-id` (id de fichier hors motif), `unknown-target` (manifeste `targets`, `outputs`, clés `native` du manifeste/agent/automation), `unknown-agent` (steps, `coordinator`, `free_agents`), `unknown-skill` (agent), `unknown-step` (`from`/`to`/`on_failure`), `unknown-state` (`when` hors `done`, `failed`, `states` de la step source), `duplicate-failure-path` (`on_failure` + lien `when: failed` sur la même step, ou deux liens), `self-link`, `empty-flow` (aucune step), `empty-command` (`run` vide ou blanc), `skill-name-mismatch` (`name` ≠ dossier), `skill-missing-description`, `skill-description-too-long` (> 1024), `empty-description` (agent), `duplicate-step-state` (`states` répète `done`/`failed` ou un doublon).
  - avertissements : `unreachable-step` (non atteinte depuis une step sans lien entrant, `on_failure` compté comme lien), `no-entry-step` (toutes les steps ont un lien entrant : boucle pure).
- Test de cohérence : `TARGETS` == `["claude","codex","copilot","opencode"]` ; dans `crates/armadai/src/linker/` un test lit les noms de cibles supportées par `link` (voir `LinkTarget`/`classify_target`) et les compare, après retrait de `gemini` absent depuis l'étape 0.

- [ ] **Step 1 : Tests d'abord** : un test par code, assert sur `code`, `severity`, `line`, `file`. Modèle :

```rust
#[test]
fn unknown_agent_in_a_step_is_positioned_on_its_token() {
    let dir = project(&[
        (".armadai/project.yaml", "schema_version: 1\n"),
        (".armadai/flows/f.yaml", "steps:\n  a: { agent: ghost }\n"),
    ]);
    let ds = validate(dir.path());
    let d = ds.iter().find(|d| d.code == "unknown-agent").unwrap();
    assert_eq!((d.line, d.column), (Some(2), Some(15)));
    assert!(d.message.contains("ghost"));
}
```
Plus un test `valid_project_has_no_diagnostic` (fixture complète du §4 de la spec : manifeste, 2 agents, 1 skill, 1 flow avec boucle `revue → corrections → revue`, 1 automation) qui prouve l'absence de faux positif, et un test `loop_is_allowed`.
- [ ] **Step 2 : Constater l'échec.**
- [ ] **Step 3 : Implémenter `check.rs`** : construire les ensembles d'ids par genre, parcourir manifeste → agents → flows → automations ; accumuler sans arrêt au premier ; atteignabilité par parcours depuis les steps sans lien entrant. Chaque `Diagnostic` porte un `hint` quand une correction est évidente (ex. liste des ids connus, « did you mean » = id à distance d'édition ≤ 2 s'il existe).
- [ ] **Step 4 : Passer au vert** ; `cargo clippy -p armadai-model --all-targets -- -D warnings`.
- [ ] **Step 5 : Mutations** : inverser `when ∈ états` → `unknown-state` rouge ; ignorer `on_failure` dans l'atteignabilité → test `on_failure_counts_as_a_link` rouge ; `locate` sans frontières de mot (`ghost` trouvé dans `ghostly`) → test `locate_requires_whole_word` rouge ; un test par code muté en supprimant son émission.
- [ ] **Step 6 : Barrière + commit** `feat(model): add the semantic validator with stable diagnostic codes`

---

### Task 4 : Écriture canonique et aller-retour

**Files:**
- Create: `crates/armadai-model/src/write.rs`, `tests/roundtrip.rs`, `tests/fixtures/roundtrip/*`
- Modify: `src/lib.rs`

**Interfaces:**
- Produces : `pub fn write_project(project: &Project, root: &Path) -> std::io::Result<Vec<PathBuf>>` (écrit sous `root/.armadai/…`, crée les dossiers, retourne les chemins écrits triés) ; `pub fn render_agent(id: &str, agent: &Agent) -> String` ; `render_manifest`, `render_flow`, `render_automation`, `render_skill`.
- Forme canonique : YAML via `serde_yaml_ng::to_string` ; agent = `---\n<yaml>---\n<body>` (corps terminé par un seul `\n`) ; ordre des clés = ordre de déclaration des structs ; flows écrits en forme développée (`steps` + `links`, jamais `sequence`).

- [ ] **Step 1 : Tests d'abord** : `roundtrip_is_semantically_stable` (pour chaque dossier de `tests/fixtures/roundtrip/` : `a = load(x)`, écrire dans un tempdir, `b = load(tempdir)` ; `assert_eq!(a.project, b.project)` et `validate` sans erreur) ; `write_is_idempotent_byte_for_byte` (écrire `a`, relire, réécrire : octets identiques) ; `body_without_trailing_newline_gets_one` ; `sequence_flow_is_written_expanded`.
- [ ] **Step 2 : Constater l'échec, Step 3 : implémenter, Step 4 : vert.**
- [ ] **Step 5 : Mutations** : écrire `tier` sous un autre nom → aller-retour rouge ; omettre les `links` → rouge ; écrire `sequence` brut → rouge.
- [ ] **Step 6 : Barrière + commit** `feat(model): write the canonical format and prove the round trip`

---

### Task 5 : JSON Schema généré

**Files:**
- Create: `crates/armadai-model/src/schema.rs`, `tests/schema.rs`, `schemas/v1/{project,agent,skill,flow,automation}.schema.json`, `crates/armadai-model/examples/gen_schemas.rs`
- Modify: `src/lib.rs`

**Interfaces:**
- Produces : `pub fn schemas() -> Vec<(&'static str /*nom de fichier*/, String /*JSON*/)>` : `project.schema.json` (Manifest), `agent.schema.json` (AgentFront), `skill.schema.json` (SkillFront), `flow.schema.json` (FlowFile), `automation.schema.json` (Automation). JSON pretty + `\n` final, via `schemars::schema_for!` puis `serde_json::to_string_pretty`.
- `examples/gen_schemas.rs` : écrit les cinq fichiers dans `schemas/v1/` (`cargo run -p armadai-model --example gen_schemas`).

- [ ] **Step 1 : Tests d'abord** : `committed_schemas_are_up_to_date` (lit `../../schemas/v1/<nom>` depuis `CARGO_MANIFEST_DIR`, compare à `schemas()` ; message : « run `cargo run -p armadai-model --example gen_schemas` ») ; `schema_target_enum_matches_TARGETS` n'existe pas : les cibles sont des `String` — vérifier à la place que `project.schema.json` ne contient pas de liste de cibles figée et que la description de `targets` cite `TARGETS` (ajouter `#[schemars(description = "…claude, codex, copilot, opencode")]` généré depuis la constante via un test qui compare le texte) ; `agent_schema_rejects_unknown_property` (`additionalProperties: false` présent).
- [ ] **Step 2 : Rouge (fichiers absents). Step 3 : implémenter + lancer l'exemple. Step 4 : vert.**
- [ ] **Step 5 : Mutation** : ajouter un champ à `AgentFront` sans régénérer → `committed_schemas_are_up_to_date` rouge ; régénérer, annuler le champ, régénérer.
- [ ] **Step 6 : Fermer #456** (ne pas le faire : seulement le citer dans la PR « Closes #456 » après accord) ; **commit** `feat(model): generate and commit the v1 JSON Schemas`

---

### Task 6 : `armadai validate` sur le nouveau format

**Files:**
- Modify: `crates/armadai/Cargo.toml` (dep `armadai-model = { path = "../armadai-model" }`), `crates/armadai/src/cli/validate.rs`, `crates/armadai/src/cli/mod.rs` (option `--json` sur `Validate`)
- Create: `crates/armadai/tests/validate_new_format.rs`

**Interfaces:**
- Consumes : `armadai_model::{validate, has_errors, Diagnostic}`.
- Comportement : si `.armadai/project.yaml` existe sous `path` → mode « nouveau » : validation, rendu `Diagnostic::render`, résumé `N error(s), M warning(s)`, code de sortie 1 si au moins une erreur ; `--json` imprime un tableau JSON des diagnostics (champs de la spec §5) sur stdout et rien d'autre. Sinon : comportement actuel inchangé (pack, ancien projet) + pour un ancien projet, ligne finale `This project uses the legacy format. Run \`armadai migrate\` to convert it.`

- [ ] **Step 1 : Tests d'abord** (`assert_cmd`, `Command::cargo_bin("armadai")`, `ARMADAI_CONFIG_DIR` isolé) : `valid_new_project_exits_0`, `invalid_new_project_exits_1_and_prints_position` (stdout contient `.armadai/flows/f.yaml:2:15: error[unknown-agent]`), `json_output_is_pure_json` (stdout parse en `Vec<Value>`, chaque objet porte `code`, `file`, `line`), `legacy_project_output_is_unchanged_plus_invitation`.
- [ ] **Step 2–4 : rouge, implémenter, vert.**
- [ ] **Step 5 : Mutations** : retirer l'invitation → rouge ; code de sortie toujours 0 → rouge ; écrire un en-tête non JSON avant le JSON → rouge.
- [ ] **Step 6 : Barrière complète + commit** `feat(cli): validate projects in the new format`

---

### Task 7 : `migrate` — lecture de l'ancien projet et conversion en mémoire

**Files:**
- Create: `crates/armadai/src/migrate/{mod,read,convert,report}.rs`
- Modify: `crates/armadai/src/main.rs` (`mod migrate;`)
- Test: tests unitaires dans les modules + `crates/armadai/tests/fixtures/migrate/<cas>/` (anciens projets)

**Interfaces:**
- Consumes : `armadai_core::project::{ProjectConfig, find config, resolve_all_prompts, resolve_all_skills}`, `armadai_core::agent_source::{load_all_agents, project_fragments}`, `armadai_core::agent::{Agent, slugify, compose_agent_prompt, default_temperature}`, `armadai_core::prompt::Prompt`, `armadai_core::skill::Skill`, `armadai_core::orchestration::OrchestrationConfig`. **Lire d'abord** `crates/armadai-core/src/project.rs` (l. 139–229, 312–470), `agent_source.rs` (l. 192–400), `agent_decl.rs` (description d'un agent déclaré) : les signatures exactes de `resolve_all_*` y sont.
- Produces :
  - `read::OldProject { root: PathBuf, config: ProjectConfig, config_file: PathBuf, agents: Vec<Agent>, prompts: Vec<Prompt>, skill_dirs: Vec<PathBuf>, declared_descriptions: BTreeMap<String,String>, load_warnings: Vec<String> }` ; `pub fn read_old(root: &Path) -> anyhow::Result<OldProject>` (erreur si aucun fichier de config : `No legacy project found at …`).
  - `report::Report { written: Vec<PathBuf>, dropped: Vec<Dropped{subject: String, field: String, reason: String}>, warnings: Vec<String> }` ; `Report::render(&self) -> String` (sections `Written`, `Dropped`, `Warnings`, déterministe, trié).
  - `convert::convert(old: &OldProject) -> (armadai_model::Project, Vec<SkillCopy>, Report)` où `SkillCopy { id: String, from: PathBuf }` (dossier de skill à copier tel quel) ; les prompts autonomes sont convertis en `Skill` dans le `Project`.
- Règles (spec §6.1, à implémenter à la lettre) :
  - id d'agent = `slugify(name)` ; collision → suffixe `-2`, `-3`, warning « agent `X` renamed to `X-2` » ; slug vide → `agent-N`.
  - `description` = description déclarée, sinon 1ʳᵉ ligne non vide du `system_prompt` tronquée à 200 caractères (warning « description derived »).
  - corps = `compose_agent_prompt(system_prompt, instructions, output_format, context)` (pas de H1).
  - `title` = nom ; `temperature` seulement si ≠ `default_temperature()` ; `tags` repris.
  - modèle : `latest:fast|pro|max` → `tier` ; `latest:auto` → dropped (« no portable equivalent ») ; autre valeur → `native.<cible>.model` avec cible = `claude` si provider ∈ {anthropic, claude}, `codex` si ∈ {openai, codex}, sinon dropped avec raison ; `model` vide → rien.
  - champs abandonnés (spec §6.1) : chaque champ **présent** (non défaut) de `AgentMetadata` hors `tags`/`temperature`/`model`, plus `pipeline` s'il existe, est listé dans `dropped` avec `subject` = id d'agent.
  - prompts → skills (`name` = nom slugifié, `description` du frontmatter sinon dérivée, corps = `body`) ; `apply_to` non vide → dropped.
  - skills du projet → `SkillCopy` ; `tools`/`version` présents → dropped.
  - `link.target` → `targets` (valeur hors `TARGETS`, ex. `gemini` → dropped + warning) ; `link.overrides.<cible>.output` → `outputs` ; `link.coordinator` → `coordinator` via `name_matches_reference` (non résolu → warning, pas de champ).
  - `orchestration` : `coordinator`/`teams` → flow `orchestration` : steps nommées par id d'agent (coordinateur, chefs, membres), liens coordinateur → chef (ou membre sans chef), chef → membres ; références à des agents inconnus → warning et omises (jamais de flow invalide) ; `policy`/`free_agents` → `manifest.policy` ; `enabled`, `pattern`, `routes`, `max_*`, etc. non défauts → dropped ; `defaults`, `routing`, `registries`, `sources` non vides → dropped.
  - Agents de la bibliothèque utilisateur référencés (`AgentRef::Named`/`Registry` résolus hors du projet) : inclus comme les autres (le chargeur les a déjà lus), warning « copied from the user library ».
  - Clés inconnues : le chargeur d'ancien format est permissif ; tout ce qui n'est pas lu est invisible. Ajouter un `Report.warnings` générique : « fields not understood by the legacy loader are not detected; review the backup » (limite connue, assumée).

- [ ] **Step 1 : Fixtures** : créer `tests/fixtures/migrate/` avec six anciens projets minimaux fidèles (copier, sans les modifier, les starters `rust-dev`, `armadai-dev`, `fullstack`, `code-analysis-rust` depuis `crates/armadai-core/` ou `starters/` selon l'emplacement réel — vérifier avec `grep -rn "fullstack" --include=pack.yaml`), plus `declarative/` (avec `.armadai/agents.yaml` + fragments + `defaults`) et `demo-rust-team/` (copie de `examples/demo-rust-team`). Si un pack n'a pas de `armadai.yaml`, en écrire un qui référence ses agents.
- [ ] **Step 2 : Tests d'abord** (`convert.rs` module tests, projets construits en mémoire) : `slug_collision_gets_a_suffix_and_a_warning`, `default_temperature_is_not_written`, `latest_tier_maps_to_tier`, `latest_auto_is_dropped`, `concrete_model_goes_native_by_provider`, `unknown_provider_model_is_dropped`, `every_dead_field_is_reported`, `prompt_becomes_a_skill`, `gemini_target_is_dropped`, `orchestration_becomes_a_flow_and_policy`, `unknown_agent_in_orchestration_warns_and_is_omitted`, `declared_description_is_kept`.
- [ ] **Step 3 : Rouge, Step 4 : implémenter, Step 5 : vert.**
- [ ] **Step 6 : Mutations** : traiter `0.7` comme non défaut → rouge ; oublier un champ abandonné (`rate_limit`) → `every_dead_field_is_reported` rouge ; ne pas dédoublonner les slugs → rouge ; retirer le filtre d'agents inconnus → flow invalide détecté par `validate` rouge.
- [ ] **Step 7 : Barrière + commit** `feat(migrate): convert a legacy project to the canonical model in memory`

---

### Task 8 : `migrate` — écriture, sauvegarde, `--dry-run`, commande

**Files:**
- Create: `crates/armadai/src/migrate/write.rs`, `crates/armadai/src/cli/migrate.rs`, `crates/armadai/tests/migrate_cli.rs`
- Modify: `crates/armadai/src/cli/mod.rs` (variante `Migrate { path: Option<PathBuf>, dry_run: bool, force: bool }` avec aide en anglais + dispatch)

**Interfaces:**
- Consumes : Task 7 (`read_old`, `convert`, `Report`), `armadai_model::{write_project, validate, has_errors}`.
- Produces : `pub fn run(root: &Path, dry_run: bool, force: bool) -> anyhow::Result<Report>` dans `migrate/mod.rs`, appelée par `cli/migrate.rs` qui imprime `report.render()` (et `Dry run: nothing was written.` en tête si `--dry-run`).
- Séquence de `run` : (1) `project.yaml` existe et pas `--force` → erreur `already migrated (use --force to redo it)` ; (2) `read_old` + `convert` ; (3) **avant toute écriture** : copier `.armadai/{config.yaml,agents.yaml,agents/,prompts/}`, `armadai.yaml|yml`, et les fichiers d'agents locaux référencés dans `.armadai/.migrated/<secondes-unix>/` en conservant les chemins relatifs ; (4) supprimer de leur emplacement d'origine ce qui est sous `.armadai/` (`config.yaml`, `agents.yaml`, `agents/*`, `prompts/`) et `armadai.yaml|yml` ; laisser en place les fichiers situés ailleurs (ex. `agents/*.md` à la racine) avec un warning « legacy file left in place: … » ; (5) `write_project` + copie des `SkillCopy` ; (6) `validate` sur la sortie : s'il y a des erreurs, les afficher et **échouer** (la sauvegarde permet de revenir) ; (7) rapport. `--dry-run` exécute (1), (2), la conversion et `validate` sur un projet **temporaire** (`tempfile`) puis imprime le rapport sans rien écrire sous `root`.
- Rapport : ajoute la ligne `Note: 'armadai link' reads the new format from step 2 of the v1 work; the previous project is in .armadai/.migrated/<ts>/.`

- [ ] **Step 1 : Tests d'abord** (`tests/migrate_cli.rs`) : `dry_run_writes_nothing` (hash récursif de l'arbre avant/après identique, y compris absence de `.migrated`) ; `second_run_is_refused` ; `force_redoes_it` ; `backup_exists_and_contains_the_original_config` ; `output_validates_clean` (appelle `armadai validate` sur la sortie → exit 0) ; `legacy_file_outside_armadai_is_left_and_reported` ; `failure_after_backup_keeps_the_backup` (injecter un agent dont la conversion produit un id invalide impossible : forcer via fixture un `link.coordinator` irrésolu ne suffit pas ; utiliser un test unitaire de `run` avec une `Project` invalide injectée par une fonction interne `run_with(convert_fn)`).
- [ ] **Step 2–4 : rouge, implémenter, vert.**
- [ ] **Step 5 : Mutations** : écrire avant de sauvegarder → `backup_exists…` ou un test d'ordre rouge (vérifier que la sauvegarde précède la première écriture : test avec un `run_with` qui échoue après sauvegarde) ; `--dry-run` qui écrit le projet → rouge ; supprimer le refus de second passage → rouge.
- [ ] **Step 6 : Barrière + commit** `feat(cli): add armadai migrate with backup, report and dry-run`

---

### Task 9 : Étalons sur six projets réels, documentation, CI

**Files:**
- Create: `crates/armadai/tests/migrate_golden.rs`, `crates/armadai/tests/golden/migrate/<cas>/…` (sorties attendues), page wiki `docs/wiki/project-format.md` (nom réel à vérifier dans `docs/wiki/` et `.github/scripts/sync-wiki.test.sh`)
- Modify: `CHANGELOG.md`, `CLAUDE.md` (section Architecture : crate `armadai-model`, formats), `docs/wiki/_Sidebar.md` si présent, `docs/wiki/migration-v0-to-v1.md` (renvoi vers `migrate`)

**Interfaces:**
- Consumes : tout ce qui précède ; l'instantané = arbre `.armadai/` produit par `armadai migrate` sur chaque fixture (hors `.migrated/`), comparé fichier par fichier ; le rapport rendu est aussi un étalon (`report.txt`). Mécanisme de régénération : variable `UPDATE_GOLDEN=1` (comme `link_golden`, vérifier son nom réel dans `crates/armadai/tests/link_golden.rs`).

- [ ] **Step 1 : Tests d'abord** : un test paramétré sur les six fixtures de la Task 7 : migrer dans un tempdir, comparer à `golden/migrate/<cas>/`, puis `armadai validate` = exit 0 ; second passage refusé ; `armadai migrate --dry-run` sur la même copie ne change pas l'arbre.
- [ ] **Step 2 : Générer** les étalons avec `UPDATE_GOLDEN=1`, **les relire un par un** : chaque agent a un `description` non vide et un corps complet, le rapport liste les champs abandonnés attendus ; corriger la conversion (Task 7) plutôt que l'étalon si une valeur est fausse.
- [ ] **Step 3 : Mutation** : modifier `convert` pour perdre le corps `## Instructions` → au moins un étalon rouge ; restaurer.
- [ ] **Step 4 : Documentation** : page « Format du projet » = §4 de la spec + tableau des codes de diagnostic (code, sévérité, signification, correctif) + guide `armadai migrate` ; `CHANGELOG.md` section `Unreleased` : `armadai-model`, `validate`, `migrate`, schémas JSON ; `CLAUDE.md` : décrire `armadai-model`, l'état transitoire (`link` encore sur l'ancien modèle jusqu'à l'étape 2) et la commande de garde.
- [ ] **Step 5 : Vérifications transverses** : `cargo build -p armadai-core`, `cargo build -p armadai-model`, `(cd frozen/armadai-audit && cargo check --all-targets)`, `cargo test -p armadai --test link_golden` (instantanés `link` inchangés), barrière complète.
- [ ] **Step 6 : Commit** `docs(model): document the project format and add the migrate goldens`

---

## Auto-relecture

- **Couverture de la spec** : §4 (types T1, chargeur T2, flows `sequence` T2), §5 (T3), écriture/aller-retour critère 3 (T4), critère 4 (T5), `validate` critères 7 (T6), §6/critère 5 (T7–T9), critère 8 (T9), critère 6 (contraintes globales + T9 step 5), C1–C11 (T1, T2, T7, T6, T5, T3 hooks sans matcher).
- **Écarts assumés par rapport à la spec** : horodatage = secondes Unix ; position sémantique = première occurrence du jeton (pas de nœud YAML) ; `migrate` supprime l'ancienne config après sauvegarde, donc `link` est inutilisable sur un projet migré jusqu'à l'étape 2 (sans ça, l'ancien et le nouveau `.armadai/agents/` se mélangent).
- **Types** : `Project`, `Origins`, `Loaded`, `Diagnostic`, `Report`, `OldProject` définis une fois, mêmes noms dans les tâches consommatrices.
