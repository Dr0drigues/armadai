# Audit d'agentique — Plan 5/5 : `--deep` — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `armadai audit --deep` : passe LLM optionnelle qui envoie les configs importées + findings statiques + collisions à un agent auditeur embarqué (via un CLI LLM détecté), parse ses findings D01-D05 et les fusionne dans le rapport. Erreur explicite si aucun provider CLI n'est disponible ; dégradation propre (section brute) si le JSON de retour est invalide.

**Architecture:** Nouveau module `src/audit/deep.rs`. L'appel LLM est **injecté via une closure** `run: impl Fn(&str) -> Result<String>` → payload et parsing sont testables sans provider réel ; seule la glue CLI est non testée en CI. Instructions de l'auditeur embarquées via `include_str!` (aucune installation). Le CLI provider ignore le system_prompt → instructions + données vont dans le message. Chemin CLI disponible sans `providers-api` (bon pour le mode CI). DTOs sérialisables dédiés (les types `Imported*` ne dérivent pas Serde).

**Décisions actées (utilisateur)** : scope complet D01-D05 ; **erreur explicite** si `--deep` demandé sans provider CLI détecté (pas de skip silencieux) ; JSON de retour invalide → section « Deep analysis (unparsed) » + pas d'échec (spec §7/§9).

**Tech Stack:** Rust édition 2024, `serde`/`serde_json` (déjà en deps, non gated), `async-trait`, `include_str!`. **Aucune nouvelle dépendance.**

## Global Constraints

- Base : `origin/release/1.0.0` (@ 7953702). Branche : `feat/audit-deep`.
- Clippy `-D warnings` vert dans les DEUX modes (`--features tui` et `tui,providers-api`) après chaque commit ; `#[allow(dead_code)]` interdit ; pas d'`unwrap()`/`expect()` runtime (tests exceptés) ; pas de nouvelle dépendance ni feature flag ; Conventional Commits ; TDD.
- `--deep` NE tourne JAMAIS en CI (nécessite un CLI LLM) : tout le testable passe par une closure injectée ; la détection CLI + l'invocation réelle sont la seule partie non couverte, et elle doit rester minime.
- Détection provider : self-contained dans `deep.rs` (`which`-check sur claude/gemini/codex/copilot/opencode/aider) — pas de couplage au module `shell/` (dont le gating est incertain).
- Findings D0x : `Finding.rule` est `&'static str` → utiliser des constantes `"D01".."D05"` (jamais de `Box::leak`, jamais de string dynamique).
- Le payload envoyé au LLM tronque chaque system prompt à `deep_prompt_truncation` caractères (nouveau champ `AuditSettings`, défaut 2000).

---

### Task 0: Branche

- [ ] **Step 1:**
```bash
cd /Users/bl209054/work/misc/armadai-audit-wt && git checkout -b feat/audit-deep origin/release/1.0.0
cargo test --no-default-features --features tui,providers-api 2>&1 | tail -1
```
Expected: `667 passed`.

---

### Task 1: `deep_prompt_truncation` + DTOs + `build_payload`

**Files:**
- Modify: `src/audit/rules/mod.rs` (`AuditSettings.deep_prompt_truncation`, from_project + tests)
- Create: `src/audit/deep.rs` (+ `pub mod deep;` dans `src/audit/mod.rs`)

**Interfaces:**
- `AuditSettings` gagne `pub deep_prompt_truncation: usize` (défaut **2000**), lu depuis `armadai.yaml > audit:` comme les autres.
- Dans `deep.rs` : DTOs `#[derive(Serialize)]` : `PayloadAgent { name, description: Option<String>, model: Option<String>, tools: Option<Vec<String>>, scope: Vec<String>, prompt_excerpt: String }`, `PayloadFinding { rule, severity, file, message }`, `DeepPayload { agents: Vec<PayloadAgent>, skills: Vec<PayloadSkill { name, description }>, instructions_excerpt: Option<String>, static_findings: Vec<PayloadFinding> }`.
- `pub(crate) fn build_payload(config: &ImportedConfig, findings: &[Finding], truncation: usize) -> String` — sérialise un `DeepPayload` en JSON compact ; `prompt_excerpt` = les `truncation` premiers caractères du system_prompt (respecter les frontières de char : `chars().take(truncation).collect()`), `instructions_excerpt` idem sur CLAUDE.md. Agents avec `ParseIssue` inclus (le LLM peut commenter), prompt tronqué quand même.

- [ ] **Step 1: Tests qui échouent**
Dans `rules/mod.rs` (étendre les 2 tests `from_project`) :
```rust
        // dans from_project_reads_audit_section (armadai.yaml enrichi):
        //   audit:\n  ...\n  deep_prompt_truncation: 500\n
        assert_eq!(s.deep_prompt_truncation, 500);
        // dans from_project_defaults_without_config:
        assert_eq!(s.deep_prompt_truncation, 2000);
```
Dans `deep.rs` :
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::rules::test_support::{agent, config_with};
    use crate::audit::rules::{Finding, Severity};

    #[test]
    fn build_payload_truncates_prompts_and_includes_findings() {
        let a = agent("reviewer", &"x".repeat(5000));
        let config = config_with(vec![a]);
        let findings = vec![Finding {
            rule: "A08",
            severity: Severity::Info,
            file: ".claude/agents/reviewer.md".into(),
            related: vec![],
            message: "inherits all tools".into(),
            suggestion: None,
        }];
        let json = build_payload(&config, &findings, 100);
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["agents"][0]["name"], "reviewer");
        assert_eq!(v["agents"][0]["prompt_excerpt"].as_str().unwrap().chars().count(), 100);
        assert_eq!(v["static_findings"][0]["rule"], "A08");
    }
}
```
- [ ] **Step 2:** Run `cargo test --no-default-features --features tui,providers-api audit::` — FAIL.
- [ ] **Step 3:** Ajouter le champ `deep_prompt_truncation` à `AuditSettings` + `Default` (2000) + parsing `from_project` (nouveau champ `Option<usize>` dans `AuditSection`). Créer `deep.rs` avec les DTOs `#[derive(serde::Serialize)]` et `build_payload` (utilise `chars().take(truncation).collect::<String>()`, `serde_json::to_string`). `pub mod deep;` dans `mod.rs`.
  Anti-dead-code : `build_payload` est `pub(crate)`, consommé par son test ET par la Task 3 ; si clippy dead-code bloque ce commit isolé, combiner avec la Task 2.
- [ ] **Step 4:** Suite + clippy 2 modes — PASS.
- [ ] **Step 5:** `git add src/audit/ && git commit -m "feat(audit): deep-pass payload builder and truncation setting"`

---

### Task 2: Instructions auditeur embarquées + `parse_deep_response`

**Files:**
- Create: `src/audit/deep_auditor.md` (instructions de l'agent auditeur, embarquées via `include_str!`)
- Modify: `src/audit/deep.rs`

**Interfaces:**
- `const AUDITOR_INSTRUCTIONS: &str = include_str!("deep_auditor.md");`
- `pub(crate) const DEEP_RULES: [&str; 5] = ["D01", "D02", "D03", "D04", "D05"];`
- `pub(crate) fn build_prompt(payload_json: &str) -> String` — `AUDITOR_INSTRUCTIONS` + un bloc « INPUT JSON: » + `payload_json` + rappel du format de sortie attendu (JSON `{"findings":[{"kind":"D01","severity":"warning","file":"...","message":"...","suggestion":"..."}]}`).
- `pub(crate) enum DeepOutcome { Findings(Vec<Finding>), Raw(String) }`
- `pub(crate) fn parse_deep_response(text: &str) -> DeepOutcome` — extrait le premier bloc JSON de `text` (le CLI peut entourer de prose / fences ```json) ; si parse OK → mappe chaque item vers un `Finding` (rule = constante D0x correspondant à `kind` en majuscules, sinon l'item est ignoré ; severity : "critical"→Critical, "warning"→Warning, sinon Info ; `related` vide ; message préfixé `[deep] `) ; si aucun JSON parsable → `Raw(text.trim().to_string())`.

Fichier `deep_auditor.md` — instructions concises demandant : analyser les agents/skills/instructions fournis + les findings statiques, et retourner UNIQUEMENT du JSON avec des findings de type D01 (overlap de rôles entre agents), D02 (system prompt flou/contradictoire), D03 (mutualisation sémantique au-delà de la duplication littérale), D04 (topologie d'équipe suggérée : coordinator/teams), D05 (directives de CLAUDE.md contredisant un agent). Chaque finding : `kind`, `severity` (critical|warning|info), `file`, `message`, `suggestion`. Interdiction de répéter les findings statiques fournis.

- [ ] **Step 1: Tests qui échouent** (`deep.rs`)
```rust
    #[test]
    fn parse_deep_response_maps_valid_json_to_findings() {
        let text = "Here is my analysis:\n```json\n{\"findings\":[{\"kind\":\"D01\",\"severity\":\"warning\",\"file\":\"a.md\",\"message\":\"roles overlap\",\"suggestion\":\"merge\"}]}\n```\nDone.";
        let DeepOutcome::Findings(f) = parse_deep_response(text) else {
            panic!("expected findings");
        };
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].rule, "D01");
        assert_eq!(f[0].severity, crate::audit::rules::Severity::Warning);
        assert!(f[0].message.starts_with("[deep] "));
    }

    #[test]
    fn parse_deep_response_unknown_kind_is_dropped() {
        let text = "{\"findings\":[{\"kind\":\"D99\",\"severity\":\"info\",\"file\":\"a\",\"message\":\"m\"}]}";
        let DeepOutcome::Findings(f) = parse_deep_response(text) else {
            panic!("expected findings (possibly empty)");
        };
        assert!(f.is_empty());
    }

    #[test]
    fn parse_deep_response_invalid_json_falls_back_to_raw() {
        let text = "The config looks fine overall, no structured output.";
        let DeepOutcome::Raw(r) = parse_deep_response(text) else {
            panic!("expected raw");
        };
        assert!(r.contains("looks fine"));
    }
```
- [ ] **Step 2:** FAIL.
- [ ] **Step 3:** Implémenter. Extraction du JSON : chercher la première `{` et la dernière `}` (ou un bloc ```json … ```), tenter `serde_json::from_str::<DeepResponse>` sur la sous-chaîne ; `DeepResponse { findings: Vec<DeepItem> }`, `DeepItem { kind, severity, file, message, suggestion: Option<String> }` (`#[derive(Deserialize)]`). Mapper kind→constante via `DEEP_RULES.iter().find(|r| **r == kind.to_uppercase())`. Écrire `deep_auditor.md`.
- [ ] **Step 4:** Suite + clippy 2 modes — PASS.
- [ ] **Step 5:** `git add src/audit/ && git commit -m "feat(audit): embedded auditor instructions and deep response parser"`

---

### Task 3: `run_deep` orchestrateur + détection CLI

**Files:**
- Modify: `src/audit/deep.rs`

**Interfaces:**
- `pub(crate) fn available_cli() -> Option<&'static str>` — retourne le premier de `["claude", "gemini", "codex", "copilot", "opencode", "aider"]` trouvé via un `which`-check (`std::process::Command::new("which").arg(c)...` sous unix ; `where` sous windows). Self-contained.
- `pub(crate) fn run_deep(config: &ImportedConfig, findings: &[Finding], truncation: usize, run: impl Fn(&str) -> anyhow::Result<String>) -> anyhow::Result<DeepOutcome>` — `build_payload` → `build_prompt` → `run(&prompt)` → `parse_deep_response`. Propage l'erreur de `run` (échec d'invocation LLM = erreur, distinct du JSON invalide qui donne `Raw`).
- La construction du provider réel et le wiring CLI sont en Task 5 (ici, `run` est injecté → testable sans provider).

- [ ] **Step 1: Tests qui échouent** (`deep.rs`)
```rust
    #[test]
    fn run_deep_with_fake_runner_returns_findings() {
        let config = config_with(vec![agent("a", "prompt")]);
        let run = |_prompt: &str| {
            Ok("{\"findings\":[{\"kind\":\"D02\",\"severity\":\"info\",\"file\":\"a.md\",\"message\":\"vague\"}]}".to_string())
        };
        let outcome = run_deep(&config, &[], 2000, run).unwrap();
        let DeepOutcome::Findings(f) = outcome else { panic!() };
        assert_eq!(f[0].rule, "D02");
    }

    #[test]
    fn run_deep_propagates_runner_error() {
        let config = config_with(vec![agent("a", "prompt")]);
        let run = |_: &str| Err(anyhow::anyhow!("cli not found"));
        assert!(run_deep(&config, &[], 2000, run).is_err());
    }
```
- [ ] **Step 2:** FAIL.
- [ ] **Step 3:** Implémenter `run_deep` et `available_cli`.
- [ ] **Step 4:** Suite + clippy 2 modes — PASS.
- [ ] **Step 5:** `git add src/audit/ && git commit -m "feat(audit): deep orchestrator with injectable runner and CLI detection"`

---

### Task 4: Rendu des findings D0x + section brute

**Files:**
- Modify: `src/audit/report.rs`

**Interfaces:**
- `AuditReport` gagne `pub deep_raw: Option<String>` (analyse brute quand le JSON était invalide). Les findings D0x sont ajoutés à `self.findings` (ils se rendent dans les groupes de sévérité normaux ; leur `rule` D0x + le préfixe `[deep]` du message les distinguent, et le breakdown les compte).
- `to_markdown`/`to_html`/`print_terminal` : après les findings (et la collision matrix), si `deep_raw.is_some()`, rendre une section « ## Deep analysis (unstructured) » avec le texte brut (échappé en HTML). `critical_count` compte déjà tout Critical, y compris D0x.

- [ ] **Step 1: Tests qui échouent** (`report.rs`)
```rust
    #[test]
    fn deep_raw_renders_section() {
        let mut r = report_with(vec![]);
        r.deep_raw = Some("Free-form deep notes.".into());
        let md = r.to_markdown();
        assert!(md.contains("Deep analysis"));
        assert!(md.contains("Free-form deep notes."));
        let html = r.to_html();
        assert!(html.contains("Deep analysis"));
    }

    #[test]
    fn deep_findings_appear_in_severity_groups() {
        let r = report_with(vec![finding("D01", Severity::Warning)]);
        let md = r.to_markdown();
        assert!(md.contains("| D01 |"));
    }
```
(Adapter le helper `report_with`/`AuditReport` littéral partout : ajouter `deep_raw: None`.)
- [ ] **Step 2:** FAIL.
- [ ] **Step 3:** Ajouter le champ `deep_raw: Option<String>` (défaut `None` dans `run_audit` de `mod.rs` et tous les littéraux de test), rendre la section dans les 3 formats (échappement HTML pour le brut). Le `md_cell`/`html_escape` existants s'appliquent.
- [ ] **Step 4:** Suite + clippy 2 modes — PASS.
- [ ] **Step 5:** `git add src/audit/ && git commit -m "feat(audit): render deep findings and unstructured deep analysis section"`

---

### Task 5: CLI `--deep` + wiring provider réel

**Files:**
- Modify: `src/cli/mod.rs` (flag `deep` sur `Audit`), `src/cli/audit.rs`

**Interfaces:**
- `Audit` gagne `#[arg(long)] deep: bool` ; `execute(path, report, min_severity, quiet, propose, deep)`.
- Flow `--deep` : APRÈS `run_audit` (findings statiques prêts) et AVANT le rendu : si `deep`, réutiliser `import_surfaces(&root)` (le config), détecter `available_cli()` ; si `None` → `anyhow::bail!("--deep requires an LLM CLI (claude, gemini, codex, copilot, opencode, aider); none found in PATH")` (erreur explicite, choix utilisateur) ; sinon construire un `Agent` auditeur en mémoire (`crate::core::agent::Agent` avec `metadata.provider = <cli détecté>`, `system_prompt` vide, `name` "deep-auditor") et une closure `run` qui : `create_provider(&agent)` → `CompletionRequest { model: detect_model_name-ou-"latest:pro", system_prompt: "", messages: vec![ChatMessage{role:"user", content: prompt}], temperature: 0.2, max_tokens: None }` → `provider.complete(req).await` (bloquant via le runtime tokio courant) → `Ok(resp.content)`. Passer cette closure à `run_deep`. Fusionner : `DeepOutcome::Findings(v)` → `audit.findings.extend(v); audit.findings.sort_by(...)` (re-trier par sévérité) ; `DeepOutcome::Raw(s)` → `audit.deep_raw = Some(s)`.
- Note async : `run_deep` prend une closure sync `Fn(&str) -> Result<String>` mais `complete` est async. Solution : la closure bloque sur le futur via un handle du runtime (`tokio::runtime::Handle::current().block_on(...)`) — OU rendre `run_deep` async et la closure async. Choix le plus simple compatible test sync : garder `run_deep` sync, la closure CLI utilise `tokio::task::block_in_place` + `Handle::current().block_on`. Vérifier que `execute` tourne dans un runtime multi-thread (main tokio) ; sinon, alternative : rendre `run_deep` générique async. **Décision d'implémentation à la charge de l'implémenteur** : privilégier `run_deep` async prenant une closure retournant un `Future` si `block_in_place` pose problème — l'important est que les tests des Tasks 1-3 restent sync et verts. Documenter le choix retenu.
- exit code inchangé : `critical_count() > 0` bail après fusion (un D0x Critical peut donc faire échouer — cohérent).

- [ ] **Step 1: Tests qui échouent** (`cli/audit.rs`)
```rust
    #[tokio::test]
    async fn deep_without_cli_errors_explicitly() {
        // Force PATH empty so no CLI is found.
        let dir = tempfile::tempdir().unwrap();
        let agents = dir.path().join(".claude/agents");
        std::fs::create_dir_all(&agents).unwrap();
        std::fs::write(agents.join("a.md"), "---\nname: a\ndescription: d\n---\nP.").unwrap();
        // SAFETY: single-threaded test tweaking PATH; restore after.
        let old = std::env::var_os("PATH");
        unsafe { std::env::set_var("PATH", "") };
        let result = execute(Some(dir.path().to_path_buf()), None, "info".into(), false, false, true).await;
        unsafe { if let Some(p) = old { std::env::set_var("PATH", p) } }
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("--deep requires an LLM CLI"));
    }
```
(Les autres tests e2e de `audit.rs` gagnent l'argument `false` pour `deep`.)
- [ ] **Step 2:** FAIL.
- [ ] **Step 3:** Implémenter le flag, le dispatch (mod.rs), le wiring closure/provider. Aide clap `--deep` : « Run an optional LLM pass (needs an installed CLI: claude, gemini...) for semantic findings ».
- [ ] **Step 4:** Suite + clippy 2 modes — PASS (le test PATH-vide vérifie l'erreur explicite ; l'appel LLM réel n'est pas couvert en CI).
- [ ] **Step 5:** `git add src/cli/ src/audit/ && git commit -m "feat(cli): audit --deep runs an optional LLM pass via a detected CLI"`

---

### Task 6: Vérif finale, démo (si CLI dispo), revue, PR

- [ ] **Step 1:** fmt + clippy 2 modes + tests 2 modes — tout vert.
- [ ] **Step 2: Démo réelle si un CLI LLM est installé** (sinon documenter le skip)
```bash
which claude gemini 2>/dev/null | head -1
# si présent :
cargo run --no-default-features --features tui -- audit /Users/bl209054/work/blg/applications/frontlibreservice/components/cls-monorepo --deep --quiet 2>&1 | tail -30
```
Consigner : findings D0x obtenus (types, exemples), ou section brute si JSON invalide, ou — si aucun CLI — noter que l'erreur explicite a été vérifiée par le test unitaire. NE PAS committer d'artefact.
- [ ] **Step 3:** Revue de branche adversariale (dispatch fable) sur `origin/release/1.0.0..HEAD` : focus sur l'injection du prompt (fuite de secrets vers le CLI ? les prompts tronqués peuvent contenir des secrets A11 → à noter), le parsing JSON robuste (findings malformés, kind manquant, tableau vide), le blocage async, l'absence de régression sur le chemin non-`--deep`.
- [ ] **Step 4:** Corriger les findings bloquants, re-revue, puis `git push -u origin feat/audit-deep` + PR vers `release/1.0.0` : `feat(audit): --deep optional LLM pass (D01-D05)`.

---

## Hors périmètre / notes

- **Fuite de secrets** : le payload envoie des extraits de prompts qui peuvent contenir des secrets (que A11 détecte justement). À surveiller en revue — au minimum documenter que `--deep` transmet le contenu au CLI LLM configuré. Un masquage des motifs A11 avant envoi est un candidat backlog.
- **Providers API** : le wiring détecte un CLI ; l'usage via un provider API pur (sans CLI) n'est pas exposé par un flag dédié (cohérent avec « erreur explicite si pas de CLI »). Extension backlog si besoin.
- Fin de la série de plans d'audit (1 socle, 2 signal, 3 collisions, 4 propose, 5 deep). Backlog robustesse restant : agrégation C03/C05, C04 code-fences, offset file_line, Lot 3 propale (commands/, settings hooks), masquage secrets `--deep`.
