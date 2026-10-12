# Audit d'agentique — Plan 4/5 : `--propose` — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `armadai audit --propose` génère `.armadai-proposal/` : un pack ArmadAI **standard et valide** (pack.yaml + agents convertis et corrigés + prompts mutualisés extraits des clusters A06 + skills copiés et réparés), installable via `armadai init --pack <chemin local>` (extension incluse). Invariant : le pack généré passe `pack_validation::validate_pack` sans erreur.

**Architecture:** Nouveau module `src/audit/proposal.rs`. La conversion vers le format agent ArmadAI (H1 + `## Metadata` liste + `## System Prompt`) **élimine structurellement les A01** (plus de frontmatter YAML). Corrections appliquées : modèles → alias résolus puis tiers portables `latest:*` (via `classify_model_tier` rendue `pub(crate)` + nouveau `tier_placeholder`), `paths:` → `scope:` (champ ArmadAI natif), skills `tools:` → `allowed-tools:` (le hint validé au Plan 3), descriptions `: ` re-quotées dans les SKILL.md copiés. Extraction A06 : plus long bloc de lignes commun à tous les membres d'un cluster → `prompts/shared-*.md` avec `apply_to`, bloc retiré des prompts des agents générés.

**Décisions actées** : `raw_frontmatter` NON ajouté (il servait au round-trip natif ; on génère du format ArmadAI, les champs parsés+salvagés suffisent) — YAGNI documenté. `- description:` émis dans `## Metadata` bien qu'ignoré par le parseur actuel (forward-compatible, self-documenting ; limitation consignée : le format agent ArmadAI n'a pas encore de champ description pour le routage).

**Tech Stack:** Rust édition 2024. **Aucune nouvelle dépendance.**

## Global Constraints

- Base : `origin/release/1.0.0` (@ 446a694). Branche : `feat/audit-propose`.
- Clippy `-D warnings` vert dans les DEUX modes après chaque commit ; `#[allow(dead_code)]` interdit ; pas d'`unwrap()`/`expect()` runtime (tests exceptés) ; pas de nouvelle dépendance ni feature flag ; Conventional Commits ; TDD.
- `--propose` n'écrit QUE dans `<root>/.armadai-proposal/` ; si le répertoire existe déjà → `bail!` explicite (jamais d'écrasement silencieux). Jamais de modification des configs natives.
- Les tests qui touchent l'installation utilisent des chemins tempdir — JAMAIS le vrai `~/.config/armadai`.
- Agents avec `ParseIssue` : convertis quand le salvage a récupéré l'essentiel (name + prompt non vide), sinon listés dans le récapitulatif comme « skipped (unreadable) ».

---

### Task 0: Branche

- [ ] **Step 1:**
```bash
cd /Users/bl209054/work/misc/armadai-audit-wt && git checkout -b feat/audit-propose origin/release/1.0.0
cargo test --no-default-features --features tui,providers-api 2>&1 | tail -1
```
Expected: `640 passed`.

---

### Task 1: `import_surfaces` — exposer l'ImportedConfig

**Files:**
- Modify: `src/audit/mod.rs`

**Interfaces:**
- Produces: `pub fn import_surfaces(root: &Path) -> (Vec<String>, reverse::ImportedConfig)` — la boucle détection/parse/fusion actuellement inline dans `run_audit` (mod.rs:15-33), extraite telle quelle. `run_audit` la consomme immédiatement (pas de dead code). `--propose` la réutilisera (double parse acceptable : ~30 fichiers).

- [ ] **Step 1: Test qui échoue** (dans `src/audit/mod.rs`, nouveau `#[cfg(test)]`)
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn import_surfaces_returns_detected_and_config() {
        let dir = tempfile::tempdir().unwrap();
        let agents = dir.path().join(".claude/agents");
        std::fs::create_dir_all(&agents).unwrap();
        std::fs::write(
            agents.join("a.md"),
            "---\nname: a\ndescription: d\n---\nBody",
        )
        .unwrap();
        let (detected, config) = import_surfaces(dir.path());
        assert_eq!(detected, vec!["claude".to_string()]);
        assert_eq!(config.agents.len(), 1);
    }
}
```
- [ ] **Step 2:** Run `cargo test --no-default-features --features tui,providers-api audit::tests` — FAIL.
- [ ] **Step 3:** Extraire de `run_audit` :
```rust
/// Detect and parse every native surface under `root`.
/// Shared by the audit run and `--propose` (which needs the raw imports).
pub fn import_surfaces(root: &Path) -> (Vec<String>, reverse::ImportedConfig) {
    let linkers: Vec<Box<dyn ReverseLinker>> = vec![Box::new(reverse::claude::ClaudeReverseLinker)];
    let mut detected = Vec::new();
    let mut config = reverse::ImportedConfig::default();
    for linker in &linkers {
        if linker.detect(root) {
            detected.push(linker.name().to_string());
            let parsed = linker.parse(root);
            config.agents.extend(parsed.agents);
            config.skills.extend(parsed.skills);
            if config.instructions.is_none() {
                config.instructions = parsed.instructions;
            }
        }
    }
    (detected, config)
}
```
et réécrire `run_audit` pour l'appeler (le reste inchangé).
- [ ] **Step 4:** Suite complète + clippy 2 modes — PASS.
- [ ] **Step 5:** `git add src/audit/ && git commit -m "refactor(audit): extract import_surfaces for reuse by propose"`

---

### Task 2: Conversion agent → format ArmadAI + tiers portables

**Files:**
- Modify: `src/linker/model_resolution.rs` (`classify_model_tier` passe `pub(crate)` ; nouveau `pub(crate) fn tier_placeholder(t: ModelTier) -> &'static str`)
- Create: `src/audit/proposal.rs` (+ `pub mod proposal;` dans `src/audit/mod.rs`)

**Interfaces:**
- Produces dans `proposal.rs` :
  - `pub(crate) fn portable_model(model: Option<&str>) -> String` — `None` → `"latest:pro"` ; `latest:*` → inchangé ; sinon `resolve_alias` transitivement puis `classify_model_tier(m, "anthropic")` → `tier_placeholder` ; si inclassable → le modèle d'origine tel quel.
  - `pub(crate) fn render_agent(agent: &ImportedAgent) -> String` — format ArmadAI exact (validé par `parser::validate_agent` en Task 5) :
```markdown
# {name}

> {description ou "Imported from native Claude Code configuration."}

## Metadata
- provider: claude
- model: {portable_model}
- description: {description}          # si présente — ignoré par le parseur actuel, forward-compatible
- tags: [imported]
- scope: [{scope_globs joints par ", "}]   # seulement si non vide

## System Prompt

{system_prompt}
```
  (le champ `tools` natif n'a pas d'équivalent ArmadAI — non émis ; `extra` hors `paths` non émis.)
- `tier_placeholder` : `Fast → "latest:fast"`, `Pro → "latest:pro"`, `Max → "latest:max"`.

- [ ] **Step 1: Tests qui échouent** (`proposal.rs`)
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::rules::test_support::agent;

    #[test]
    fn portable_model_maps_concrete_models_to_tiers() {
        assert_eq!(portable_model(Some("opus")), "latest:max");
        assert_eq!(portable_model(Some("claude-sonnet-5")), "latest:pro");
        assert_eq!(portable_model(Some("latest:fast")), "latest:fast");
        assert_eq!(portable_model(None), "latest:pro");
        // Deprecated alias resolved first, then classified.
        assert_eq!(portable_model(Some("gemini-3.0-pro")), "latest:pro");
    }

    #[test]
    fn render_agent_produces_armadai_format() {
        let mut a = agent("reviewer", "You review code.");
        a.metadata.model = Some("opus".to_string());
        a.metadata
            .extra
            .insert("paths".into(), serde_yaml_ng::Value::String("src/**".into()));
        let md = render_agent(&a);
        assert!(md.starts_with("# reviewer\n"));
        assert!(md.contains("## Metadata"));
        assert!(md.contains("- provider: claude"));
        assert!(md.contains("- model: latest:max"));
        assert!(md.contains("- scope: [src/**]"));
        assert!(md.contains("## System Prompt"));
        assert!(md.contains("You review code."));
    }
}
```
- [ ] **Step 2:** FAIL.
- [ ] **Step 3: Implémentation** — `model_resolution.rs` : changer `fn classify_model_tier` en `pub(crate) fn`, ajouter :
```rust
/// The portable placeholder string for a tier (inverse of parse_latest_placeholder).
pub(crate) fn tier_placeholder(tier: ModelTier) -> &'static str {
    match tier {
        ModelTier::Fast => "latest:fast",
        ModelTier::Pro => "latest:pro",
        ModelTier::Max => "latest:max",
    }
}
```
`proposal.rs` :
```rust
//! Generation of an ArmadAI proposal pack from imported native configs.
use std::fmt::Write as _;

use super::reverse::ImportedAgent;
use crate::linker::model_aliases::resolve_alias;
use crate::linker::model_resolution::{
    classify_model_tier, is_latest_placeholder, tier_placeholder,
};

/// Map a native model to a portable ArmadAI tier when possible.
pub(crate) fn portable_model(model: Option<&str>) -> String {
    let Some(model) = model else {
        return "latest:pro".to_string();
    };
    if is_latest_placeholder(model) {
        return model.to_string();
    }
    let resolved = resolve_alias(model).unwrap_or_else(|| model.to_string());
    if is_latest_placeholder(&resolved) {
        return resolved;
    }
    match classify_model_tier(&resolved, "anthropic") {
        Some(tier) => tier_placeholder(tier).to_string(),
        None => resolved,
    }
}

/// Render an imported agent in the ArmadAI agent format
/// (H1 + `## Metadata` list + `## System Prompt`).
pub(crate) fn render_agent(agent: &ImportedAgent) -> String {
    let mut md = String::new();
    let _ = writeln!(md, "# {}\n", agent.name);
    let description = agent
        .metadata
        .description
        .as_deref()
        .unwrap_or("Imported from native Claude Code configuration.");
    let _ = writeln!(md, "> {description}\n");
    let _ = writeln!(md, "## Metadata");
    let _ = writeln!(md, "- provider: claude");
    let _ = writeln!(md, "- model: {}", portable_model(agent.metadata.model.as_deref()));
    if agent.metadata.description.is_some() {
        // Ignored by today's parser (unknown key -> debug log); forward-compatible.
        let _ = writeln!(md, "- description: {description}");
    }
    let _ = writeln!(md, "- tags: [imported]");
    let globs = agent.metadata.scope_globs();
    if !globs.is_empty() {
        let _ = writeln!(md, "- scope: [{}]", globs.join(", "));
    }
    let _ = writeln!(md, "\n## System Prompt\n");
    let _ = writeln!(md, "{}", agent.system_prompt);
    md
}
```
Dans `audit/mod.rs` : `pub mod proposal;`. Note : `classify_model_tier(model, provider)` — vérifier l'ordre exact des paramètres dans model_resolution.rs:61 et s'y conformer.
- [ ] **Step 4:** Suite + clippy 2 modes — PASS. (`render_agent`/`portable_model` sont `pub(crate)` consommés par leurs tests ET par la Task 5 ; si clippy râle dead-code sur ce commit isolé, committer combiné avec la Task 3.)
- [ ] **Step 5:** `git add src/audit/ src/linker/ && git commit -m "feat(audit): propose converts agents to ArmadAI format with portable tiers"`

---

### Task 3: Extraction des prompts partagés (clusters A06)

**Files:**
- Modify: `src/audit/rules/similarity.rs` (exposer la découverte de clusters) + `src/audit/rules/mod.rs` (re-export)
- Modify: `src/audit/proposal.rs`

**Interfaces:**
- Dans `similarity.rs` : extraire de `a06_duplicated_blocks` une fonction réutilisable `pub(crate) fn duplication_clusters(agents: &[ImportedAgent]) -> Vec<Vec<usize>>` (calcul fenêtres + union-find, retourne les composantes ≥2 triées) ; `a06_duplicated_blocks` la consomme (pas de duplication de logique). Re-export dans `rules/mod.rs` : `pub(crate) use similarity::duplication_clusters;`.
- Dans `proposal.rs` :
  - `pub(crate) struct SharedFragment { pub name: String, pub apply_to: Vec<String>, pub body: String }`
  - `pub(crate) fn extract_shared_fragment(agents: &[&ImportedAgent], index: usize) -> Option<SharedFragment>` — plus long bloc contigu de lignes (trimées, non vides) présent chez TOUS les membres, longueur ≥ 8 ; `name = format!("shared-conventions-{}", index + 1)` ; `apply_to` = noms des membres ; `None` si aucun bloc commun ≥ 8.
  - `pub(crate) fn strip_fragment(prompt: &str, fragment_body: &str) -> String` — retire du prompt la première occurrence du bloc (correspondance sur lignes trimées non vides, suppression des lignes brutes correspondantes), et compacte les lignes vides triples résultantes.
  - `pub(crate) fn render_prompt(f: &SharedFragment) -> String` — format fragment ArmadAI :
```markdown
---
name: {name}
description: Shared conventions extracted from {n} agents by armadai audit --propose
apply_to:
  - {agent...}
---
{body}
```

- [ ] **Step 1: Tests qui échouent** (`proposal.rs`)
```rust
    fn block() -> String {
        (1..=10).map(|i| format!("Convention line {i}\n")).collect()
    }

    #[test]
    fn extract_shared_fragment_finds_longest_common_block() {
        let b = block();
        let a1 = agent("g1", &format!("Intro one.\n\n{b}Outro one."));
        let a2 = agent("g2", &format!("{b}Outro two."));
        let refs: Vec<&ImportedAgent> = vec![&a1, &a2];
        let f = extract_shared_fragment(&refs, 0).unwrap();
        assert_eq!(f.name, "shared-conventions-1");
        assert_eq!(f.apply_to, vec!["g1".to_string(), "g2".to_string()]);
        assert!(f.body.contains("Convention line 1"));
        assert!(f.body.contains("Convention line 10"));
        assert!(!f.body.contains("Intro"));
    }

    #[test]
    fn extract_returns_none_below_window() {
        let a1 = agent("s1", "short\ncommon\ntext");
        let a2 = agent("s2", "short\ncommon\ntext");
        let refs: Vec<&ImportedAgent> = vec![&a1, &a2];
        // 3 common lines < 8-line window: not worth a shared fragment.
        assert!(extract_shared_fragment(&refs, 0).is_none());
    }

    #[test]
    fn strip_fragment_removes_block_and_keeps_rest() {
        let b = block();
        let prompt = format!("Intro.\n\n{b}\nOutro.");
        let f_body = b.trim_end().to_string();
        let stripped = strip_fragment(&prompt, &f_body);
        assert!(stripped.contains("Intro."));
        assert!(stripped.contains("Outro."));
        assert!(!stripped.contains("Convention line 5"));
    }

    #[test]
    fn render_prompt_has_frontmatter_and_apply_to() {
        let f = SharedFragment {
            name: "shared-conventions-1".into(),
            apply_to: vec!["g1".into(), "g2".into()],
            body: "Some shared text.".into(),
        };
        let md = render_prompt(&f);
        assert!(md.starts_with("---\n"));
        assert!(md.contains("apply_to:\n  - g1\n  - g2"));
        assert!(md.contains("Some shared text."));
    }
```
- [ ] **Step 2:** FAIL.
- [ ] **Step 3: Implémentation** — algorithme d'extraction :
```rust
fn norm_lines(text: &str) -> Vec<&str> {
    text.lines().map(str::trim).filter(|l| !l.is_empty()).collect()
}

/// Longest contiguous run of normalized lines from `base` that appears
/// (as a contiguous run) in every other member. O(n²·m) on small prompts.
fn longest_common_block<'a>(members: &[Vec<&'a str>]) -> Vec<&'a str> {
    let Some((base, others)) = members.split_first() else {
        return Vec::new();
    };
    let mut best: (usize, usize) = (0, 0); // (start, len)
    let n = base.len();
    for start in 0..n {
        if n - start <= best.1 {
            break;
        }
        let mut len = n - start;
        while len > best.1 {
            let candidate = &base[start..start + len];
            if others.iter().all(|o| contains_run(o, candidate)) {
                best = (start, len);
                break;
            }
            len -= 1;
        }
    }
    base[best.0..best.0 + best.1].to_vec()
}

fn contains_run(haystack: &[&str], needle: &[&str]) -> bool {
    if needle.is_empty() || haystack.len() < needle.len() {
        return false;
    }
    haystack.windows(needle.len()).any(|w| w == needle)
}
```
`extract_shared_fragment` : construit `norm_lines` par membre, `longest_common_block`, `None` si `< 8` lignes, sinon body = lignes jointes par `\n`. `strip_fragment` : reconstruit l'indexation lignes brutes ↔ lignes normalisées du prompt, trouve la première fenêtre de lignes normalisées égale au bloc, supprime les lignes brutes correspondantes, puis remplace les séquences de ≥3 sauts de ligne par 2 :
```rust
pub(crate) fn strip_fragment(prompt: &str, fragment_body: &str) -> String {
    let needle: Vec<&str> = norm_lines(fragment_body);
    let raw: Vec<&str> = prompt.lines().collect();
    // raw indices of non-empty lines
    let idx: Vec<usize> = raw
        .iter()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty())
        .map(|(i, _)| i)
        .collect();
    let norm: Vec<&str> = idx.iter().map(|&i| raw[i].trim()).collect();
    let Some(pos) = norm
        .windows(needle.len().max(1))
        .position(|w| w == needle.as_slice())
    else {
        return prompt.to_string();
    };
    let (from, to) = (idx[pos], idx[pos + needle.len() - 1]);
    let kept: Vec<&str> = raw
        .iter()
        .enumerate()
        .filter(|(i, _)| *i < from || *i > to)
        .map(|(_, l)| *l)
        .collect();
    let mut out = kept.join("\n");
    while out.contains("\n\n\n") {
        out = out.replace("\n\n\n", "\n\n");
    }
    out
}
```
Perf : garde-fou dans l'appelant (Task 5) — extraction seulement pour les clusters dont les prompts font < 100 000 caractères chacun (sinon skip avec note dans le récapitulatif).
- [ ] **Step 4:** Suite + clippy 2 modes — PASS.
- [ ] **Step 5:** `git add src/audit/ && git commit -m "feat(audit): propose extracts shared prompt fragments from duplication clusters"`

---

### Task 4: Copie des skills + réparations SKILL.md

**Files:**
- Modify: `src/audit/proposal.rs`

**Interfaces:**
- Produces:
  - `pub(crate) fn fix_skill_md(content: &str) -> (String, Vec<&'static str>)` — deux réparations textuelles du frontmatter (entre les deux premiers `---`) : (a) renommer la clé `tools:` → `allowed-tools:` (le champ que Claude Code lit réellement — hint du Plan 3) ; (b) quoter les valeurs de `description:`/`name:` non quotées contenant `: `. Retourne le contenu réparé + la liste des fixes appliqués (`"tools->allowed-tools"`, `"quoted-value"`).
  - `pub(crate) fn copy_skill_dir(src: &Path, dest: &Path) -> anyhow::Result<Vec<&'static str>>` — copie récursive (profondeur ≤ 5), applique `fix_skill_md` au `SKILL.md` racine, retourne les fixes.

- [ ] **Step 1: Tests qui échouent**
```rust
    #[test]
    fn fix_skill_md_renames_tools_and_quotes_colons() {
        let content = "---\nname: triage\ndescription: triage is HUMAN — the skill : just assists\ntools: Read, Grep\n---\nBody";
        let (fixed, fixes) = fix_skill_md(content);
        assert!(fixed.contains("allowed-tools: Read, Grep"));
        assert!(fixed.contains("description: \"triage is HUMAN — the skill : just assists\""));
        assert!(!fixed.contains("\ntools:"));
        assert_eq!(fixes.len(), 2);
    }

    #[test]
    fn fix_skill_md_leaves_clean_files_alone() {
        let content = "---\nname: ok\ndescription: fine\nallowed-tools: Read\n---\nBody";
        let (fixed, fixes) = fix_skill_md(content);
        assert_eq!(fixed, content);
        assert!(fixes.is_empty());
    }

    #[test]
    fn copy_skill_dir_copies_recursively_and_fixes() {
        let src = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(src.path().join("references")).unwrap();
        std::fs::write(
            src.path().join("SKILL.md"),
            "---\nname: s\ndescription: d\ntools: Read\n---\nBody",
        )
        .unwrap();
        std::fs::write(src.path().join("references/ref.md"), "ref").unwrap();
        let dest = tempfile::tempdir().unwrap();
        let dest_dir = dest.path().join("s");
        let fixes = copy_skill_dir(src.path(), &dest_dir).unwrap();
        assert_eq!(fixes, vec!["tools->allowed-tools"]);
        assert!(dest_dir.join("references/ref.md").exists());
        let skill = std::fs::read_to_string(dest_dir.join("SKILL.md")).unwrap();
        assert!(skill.contains("allowed-tools: Read"));
    }
```
- [ ] **Step 2:** FAIL.
- [ ] **Step 3: Implémentation** — `fix_skill_md` opère ligne à ligne UNIQUEMENT à l'intérieur du premier bloc `---`…`---` :
```rust
pub(crate) fn fix_skill_md(content: &str) -> (String, Vec<&'static str>) {
    let mut fixes = Vec::new();
    let mut in_frontmatter = false;
    let mut seen_delims = 0u8;
    let lines: Vec<String> = content
        .lines()
        .map(|line| {
            if line.trim() == "---" && seen_delims < 2 {
                seen_delims += 1;
                in_frontmatter = seen_delims == 1;
                return line.to_string();
            }
            if !in_frontmatter || seen_delims != 1 {
                return line.to_string();
            }
            if let Some(rest) = line.strip_prefix("tools:") {
                fixes.push("tools->allowed-tools");
                return format!("allowed-tools:{rest}");
            }
            if let Some((key, value)) = line.split_once(':')
                && matches!(key.trim(), "description" | "name")
            {
                let v = value.trim();
                if v.contains(": ") && !v.starts_with('"') && !v.starts_with('\'') {
                    fixes.push("quoted-value");
                    return format!("{}: \"{v}\"", key.trim_end());
                }
            }
            line.to_string()
        })
        .collect();
    (lines.join("\n") + if content.ends_with('\n') { "\n" } else { "" }, fixes)
}
```
`copy_skill_dir` : récursion bornée avec `std::fs` (create_dir_all + copy), `SKILL.md` de la racine passé par `fix_skill_md` et écrit réparé.
- [ ] **Step 4:** Suite + clippy 2 modes — PASS. (Fonctions consommées par les tests + Task 5 ; même consigne combine-si-rouge.)
- [ ] **Step 5:** `git add src/audit/ && git commit -m "feat(audit): propose copies skills with allowed-tools and quoting fixes"`

---

### Task 5: Assemblage du pack + invariant validate

**Files:**
- Modify: `src/audit/proposal.rs`
- Modify: `src/core/pack_validation.rs` (si `validate_pack` porte encore `#[allow(dead_code)]` : le retirer — `--propose` devient son consommateur programmatique)

**Interfaces:**
- Produces:
```rust
pub struct ProposalSummary {
    pub out_dir: PathBuf,
    pub agents: usize,
    pub prompts: usize,
    pub skills: usize,
    pub skill_fixes: usize,
    pub skipped_agents: Vec<String>,
}

pub fn generate_proposal(
    root: &Path,
    config: &ImportedConfig,
) -> anyhow::Result<ProposalSummary>
```
Comportement : `out_dir = root.join(".armadai-proposal")` ; `bail!` si existe déjà. Étapes : (1) clusters A06 via `rules::duplication_clusters` (sur agents convertibles) → fragments extraits (garde-fou 100k chars/prompt) ; (2) agents rendus avec prompts strippés des fragments qui les concernent → `agents/{name}.md` (noms slugifiés via `crate::linker::slugify` si dispo, sinon nom tel quel — les noms importés sont déjà des slugs) ; agents illisibles (issues non vides ET (name vide OU prompt vide)) → skipped ; (3) fragments → `prompts/{name}.md` ; (4) skills avec `has_skill_md` → `copy_skill_dir` vers `skills/{name}/` ; (5) `pack.yaml` :
```yaml
name: {root dir name sanitisé}-agents
description: "Generated by `armadai audit --propose` from the native Claude Code configuration"
agents: [...]
prompts: [...]
skills: [...]
```
(6) **invariant** : `let issues = crate::core::pack_validation::validate_pack(&out_dir);` — si une issue `Severity::Error` → `bail!` en listant les erreurs (le générateur ne livre jamais un pack invalide).

- [ ] **Step 1: Test qui échoue** (e2e dans `proposal.rs`)
```rust
    #[test]
    fn generate_proposal_produces_valid_installable_pack() {
        let dir = tempfile::tempdir().unwrap();
        let agents = dir.path().join(".claude/agents");
        std::fs::create_dir_all(&agents).unwrap();
        let block = block();
        std::fs::write(
            agents.join("gate-a.md"),
            format!("---\nname: gate-a\ndescription: Gate A\nmodel: opus\n---\nIntro A.\n\n{block}"),
        )
        .unwrap();
        std::fs::write(
            agents.join("gate-b.md"),
            format!("---\nname: gate-b\ndescription: Gate B\n---\n{block}Outro B."),
        )
        .unwrap();
        let skill = dir.path().join(".claude/skills/deploy");
        std::fs::create_dir_all(&skill).unwrap();
        std::fs::write(
            skill.join("SKILL.md"),
            "---\nname: deploy\ndescription: Deploys\ntools: Read\n---\nSteps.",
        )
        .unwrap();

        let (_, config) = crate::audit::import_surfaces(dir.path());
        let summary = generate_proposal(dir.path(), &config).unwrap();
        assert_eq!(summary.agents, 2);
        assert_eq!(summary.prompts, 1);
        assert_eq!(summary.skills, 1);
        assert_eq!(summary.skill_fixes, 1);

        let out = dir.path().join(".armadai-proposal");
        // The generated agents parse with the real ArmadAI parser.
        for name in ["gate-a", "gate-b"] {
            let parsed =
                crate::parser::validate_agent(&out.join(format!("agents/{name}.md"))).unwrap();
            assert_eq!(parsed.metadata.provider, "claude");
        }
        // Shared block was factored out of the agents.
        let a = std::fs::read_to_string(out.join("agents/gate-a.md")).unwrap();
        assert!(!a.contains("Convention line 5"));
        let p = std::fs::read_to_string(out.join("prompts/shared-conventions-1.md")).unwrap();
        assert!(p.contains("Convention line 5"));
        // Second run refuses to overwrite.
        assert!(generate_proposal(dir.path(), &config).is_err());
    }
```
(Vérifier la signature réelle de `crate::parser::validate_agent` — le module parser l'expose pour la commande validate ; adapter l'appel si elle prend `&Path`.)
- [ ] **Step 2:** FAIL.
- [ ] **Step 3:** Implémentation complète :
```rust
pub struct ProposalSummary {
    pub out_dir: std::path::PathBuf,
    pub agents: usize,
    pub prompts: usize,
    pub skills: usize,
    pub skill_fixes: usize,
    pub skipped_agents: Vec<String>,
}

const MAX_PROMPT_CHARS_FOR_EXTRACTION: usize = 100_000;

pub fn generate_proposal(
    root: &std::path::Path,
    config: &super::reverse::ImportedConfig,
) -> anyhow::Result<ProposalSummary> {
    let out_dir = root.join(".armadai-proposal");
    if out_dir.exists() {
        anyhow::bail!(
            "{} already exists — remove it first (the proposal never overwrites)",
            out_dir.display()
        );
    }
    // Convertible agents: salvage must have recovered the essentials.
    let mut skipped_agents = Vec::new();
    let convertible: Vec<&super::reverse::ImportedAgent> = config
        .agents
        .iter()
        .filter(|a| {
            let ok = !a.name.is_empty() && !a.system_prompt.is_empty();
            if !ok {
                skipped_agents.push(a.name.clone());
            }
            ok
        })
        .collect();

    // Shared fragments from duplication clusters (guard very large prompts).
    let owned: Vec<super::reverse::ImportedAgent> =
        convertible.iter().map(|a| (*a).clone()).collect();
    let clusters = crate::audit::rules::duplication_clusters(&owned);
    let mut fragments: Vec<SharedFragment> = Vec::new();
    for (i, members) in clusters.iter().enumerate() {
        let refs: Vec<&super::reverse::ImportedAgent> =
            members.iter().map(|&m| &owned[m]).collect();
        if refs
            .iter()
            .any(|a| a.system_prompt.len() > MAX_PROMPT_CHARS_FOR_EXTRACTION)
        {
            continue;
        }
        if let Some(f) = extract_shared_fragment(&refs, i) {
            fragments.push(f);
        }
    }

    std::fs::create_dir_all(out_dir.join("agents"))?;
    // Agents: render with their shared fragments stripped out.
    for a in &owned {
        let mut agent = a.clone();
        for f in &fragments {
            if f.apply_to.iter().any(|n| n == &agent.name) {
                agent.system_prompt = strip_fragment(&agent.system_prompt, &f.body);
            }
        }
        let file = out_dir
            .join("agents")
            .join(format!("{}.md", crate::linker::slugify(&agent.name)));
        std::fs::write(file, render_agent(&agent))?;
    }
    // Prompts.
    if !fragments.is_empty() {
        std::fs::create_dir_all(out_dir.join("prompts"))?;
        for f in &fragments {
            std::fs::write(
                out_dir.join("prompts").join(format!("{}.md", f.name)),
                render_prompt(f),
            )?;
        }
    }
    // Skills.
    let mut skill_fixes = 0usize;
    let installable_skills: Vec<&super::reverse::ImportedSkill> =
        config.skills.iter().filter(|s| s.has_skill_md).collect();
    if !installable_skills.is_empty() {
        std::fs::create_dir_all(out_dir.join("skills"))?;
        for s in &installable_skills {
            // source_path points at SKILL.md; the skill dir is its parent.
            let src = s
                .source_path
                .parent()
                .map(std::path::Path::to_path_buf)
                .unwrap_or_else(|| s.source_path.clone());
            let dest = out_dir.join("skills").join(&s.name);
            skill_fixes += copy_skill_dir(&src, &dest)?.len();
        }
    }
    // pack.yaml
    let pack_name = root
        .file_name()
        .map(|n| crate::linker::slugify(&n.to_string_lossy()))
        .unwrap_or_else(|| "imported".to_string());
    let mut pack = String::new();
    use std::fmt::Write as _;
    let _ = writeln!(pack, "name: {pack_name}-agents");
    let _ = writeln!(
        pack,
        "description: \"Generated by `armadai audit --propose` from the native Claude Code configuration\""
    );
    let _ = writeln!(pack, "agents:");
    for a in &owned {
        let _ = writeln!(pack, "  - {}", crate::linker::slugify(&a.name));
    }
    if !fragments.is_empty() {
        let _ = writeln!(pack, "prompts:");
        for f in &fragments {
            let _ = writeln!(pack, "  - {}", f.name);
        }
    }
    if !installable_skills.is_empty() {
        let _ = writeln!(pack, "skills:");
        for s in &installable_skills {
            let _ = writeln!(pack, "  - {}", s.name);
        }
    }
    std::fs::write(out_dir.join("pack.yaml"), pack)?;

    // Invariant: the generator never ships an invalid pack.
    let errors: Vec<String> = crate::core::pack_validation::validate_pack(&out_dir)
        .into_iter()
        .filter(|i| matches!(i.severity, crate::core::pack_validation::Severity::Error))
        .map(|i| format!("{}: {}", i.location, i.message))
        .collect();
    if !errors.is_empty() {
        anyhow::bail!("generated proposal failed validation:\n{}", errors.join("\n"));
    }

    Ok(ProposalSummary {
        out_dir,
        agents: owned.len(),
        prompts: fragments.len(),
        skills: installable_skills.len(),
        skill_fixes,
        skipped_agents,
    })
}
```
Prérequis de visibilité : `duplication_clusters` re-exporté `pub(crate)` par `rules/mod.rs` (Task 3) ; `crate::linker::slugify` est `pub` (linker/mod.rs:99) ; retirer les `#[allow(dead_code)]` devenus faux dans `pack_validation.rs` (au minimum `validate_pack` et `Severity`/`ValidationIssue`). Attention au conflit de nom : `pack_validation::Severity` ≠ `rules::Severity` — utiliser les chemins qualifiés comme ci-dessus.
- [ ] **Step 4:** Suite + clippy 2 modes — PASS.
- [ ] **Step 5:** `git add src/audit/ src/core/ && git commit -m "feat(audit): generate validated ArmadAI proposal pack"`

---

### Task 6: CLI `--propose` + `init --pack` chemin local

**Files:**
- Modify: `src/cli/mod.rs` (flag sur `Audit` ; help de `--pack` sur Init)
- Modify: `src/cli/audit.rs`
- Modify: `src/cli/init.rs` (résolution chemin local)
- Modify: `src/audit/report.rs` (message funnel : retirer « (coming soon) »)

**Interfaces:**
- `Audit` gagne `#[arg(long)] propose: bool` ; `execute(path, report, min_severity, quiet, propose)`. Flow : audit normal, puis si `propose` : `let (_, config) = import_surfaces(&root);` → `generate_proposal` → affiche le récapitulatif + la commande d'adoption exacte :
```
  Proposal written to {out}/
    {n} agent(s), {n} shared prompt(s), {n} skill(s) ({n} fixed)
  Install it with:
    armadai init --pack {out} --project
```
  Le `bail!` sur findings critiques reste APRÈS la génération (un repo à criticals peut justement vouloir la proposition).
- `init --pack` : dans `install_pack` (init.rs:66), AVANT `find_pack_dir` :
```rust
    let candidate = std::path::Path::new(name);
    let pack_dir = if candidate.join("pack.yaml").is_file() {
        candidate.to_path_buf()
    } else {
        match find_pack_dir(name) { ... existant ... }
    };
```
- Funnel (`report.rs`) : la ligne `Run \`armadai audit --propose\` (coming soon)...` devient `Run \`armadai audit --propose\` to generate the config.`

- [ ] **Step 1: Tests qui échouent**
Dans `cli/audit.rs` :
```rust
    #[tokio::test]
    async fn execute_with_propose_writes_proposal() {
        let dir = tempfile::tempdir().unwrap();
        let agents = dir.path().join(".claude/agents");
        std::fs::create_dir_all(&agents).unwrap();
        std::fs::write(
            agents.join("ok.md"),
            "---\nname: ok\ndescription: Fine\nmodel: latest:pro\ntools: Read\n---\nShort prompt.",
        )
        .unwrap();
        let result = execute(
            Some(dir.path().to_path_buf()),
            None,
            "info".to_string(),
            false,
            true,
        )
        .await;
        assert!(result.is_ok());
        assert!(dir.path().join(".armadai-proposal/pack.yaml").is_file());
        assert!(dir.path().join(".armadai-proposal/agents/ok.md").is_file());
    }
```
Dans `cli/init.rs` (test de la résolution locale — extraire la résolution en `pub(crate) fn resolve_pack_dir(name: &str) -> Option<PathBuf>` pour la tester sans installer) :
```rust
    #[test]
    fn resolve_pack_dir_accepts_local_path() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("pack.yaml"),
            "name: local-pack\ndescription: d\n",
        )
        .unwrap();
        let resolved = resolve_pack_dir(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(resolved, dir.path());
    }
```
Dans `report.rs` : adapter l'assertion funnel si un test contient « coming soon ».
- [ ] **Step 2:** FAIL.
- [ ] **Step 3:** Implémenter (les 4 tests e2e existants de `cli/audit.rs` gagnent l'argument `false` pour `propose`). `install_pack` utilise `resolve_pack_dir`.
- [ ] **Step 4:** Suite + clippy 2 modes — PASS. Mettre à jour le help clap de `--pack` : « Starter pack name, or path to a directory containing pack.yaml ».
- [ ] **Step 5:** `git add src/cli/ src/audit/ && git commit -m "feat(cli): audit --propose generates pack, init --pack accepts local paths"`

---

### Task 7: Vérification finale, démo cls, PR

- [ ] **Step 1:** fmt + clippy 2 modes + tests 2 modes — tout vert (~660+ tests).
- [ ] **Step 2: Démo bout-en-bout sur le repo réel**
```bash
cargo run --no-default-features --features tui -- audit /Users/bl209054/work/blg/applications/frontlibreservice/components/cls-monorepo --propose 2>&1 | tail -20
ls /Users/bl209054/work/blg/applications/frontlibreservice/components/cls-monorepo/.armadai-proposal/
cargo run --no-default-features --features tui -- validate /Users/bl209054/work/blg/applications/frontlibreservice/components/cls-monorepo/.armadai-proposal
```
Consigner : nb d'agents convertis (~22 + 2 salvagés), fragments extraits (≥1, le cluster des 4 gates), skills copiés (34) et fixes (≥13 tools→allowed-tools + 1 quote). **Puis supprimer `.armadai-proposal/` du repo cls** (c'était une démo, pas une adoption) :
```bash
rm -rf /Users/bl209054/work/blg/applications/frontlibreservice/components/cls-monorepo/.armadai-proposal
```
- [ ] **Step 3:** `git push -u origin feat/audit-propose` + PR vers `release/1.0.0` : `feat(audit): --propose generates a validated ArmadAI pack from native configs`.

---

## Hors périmètre (rappel)

- `raw_frontmatter` (round-trip natif) : non nécessaire pour la conversion ArmadAI — attendra un vrai besoin de régénération native.
- Champ `description` de première classe dans le format agent ArmadAI (limitation documentée — émis en clé ignorée).
- Plan 5 : `--deep`. Backlog robustesse restant : agrégation C03/C05, C04 code-fences, offset file_line, Lot 3 propale (commands/, settings hooks).
