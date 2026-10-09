# Étape 0 — Retrait des surfaces d'exécution : plan d'implémentation

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Réduire ArmadAI aux commandes déclaratives (`link`, `unlink`, `list`, `inspect`, `validate`, `new`, `init`, `prompts`, `skills`, `extract`, `models`, `config`, `update`, `completion`) en retirant l'exécution, l'orchestration, les interfaces, le stockage, `registry` et Gemini, sans changer ce que `link` écrit pour Claude, Codex, OpenCode et Copilot.

**Architecture:** Un test d'étalon (« golden ») fige d'abord la sortie de `link` pour les quatre cibles gardées. Puis sept PR successives suppriment les surfaces de la feuille vers la racine (tests, web, watch, run/storage, shell/TUI, audit/registry, Gemini/providers/core), chacune verte. La dernière PR met à jour la documentation, la CI et l'équipe d'agents.

**Tech Stack:** Rust 2024 (workspace Cargo), `assert_cmd`, `tempfile`, GitHub Actions, `gh`.

**Spec:** `docs/superpowers/specs/2026-10-09-step0-retrait-surfaces-design.md`

## Global Constraints

- Branche par défaut : `master`. Jamais de push direct. Une branche et une PR par tâche numérotée « PR », squash merge.
- Commits : Conventional Commits, **un seul type** par sujet (`docs/test(x):` est invalide). Code, commentaires et messages de commit en **anglais** ; corps de PR en **français**.
- Chaque commit se termine par un second paragraphe de message `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>` (deuxième `-m`). Chaque corps de PR se termine par `🤖 Generated with [Claude Code](https://claude.com/claude-code)`.
- **Barrière avant chaque push** (ordre exact), à exécuter au premier plan sans tâche de fond :
  `cargo fmt --all -- --check` ; `cargo clippy --all-targets <features de la PR> -- -D warnings` ; `cargo test --no-fail-fast <features de la PR>` ; `cargo build -p armadai-core`. Les « features de la PR » sont données dans chaque tâche.
- `crates/armadai` est un crate **binaire seul** : `cargo test --lib` y rend « 0 passed » sans erreur. Utiliser `--bin armadai` ou `--test <nom>`. Un « 0 passed » est une alarme.
- **Mesurer, ne pas raisonner** : pour chaque test ajouté ou modifié, casser le code qu'il protège, constater le rouge, restaurer, et reporter la mutation avec la sortie vue.
- Une suppression ne se masque pas : jamais de `#[allow(dead_code)]` pour faire taire un code devenu mort ; on supprime le code.
- Ne pas supprimer de branche distante, ne pas pousser le tag `pre-v1-declaratif` ni créer d'issue GitHub sans accord explicite de Dimitri.
- Les chiffres de lignes et les numéros de ligne cités viennent du relevé du 2026-10-09 : les **revérifier avec `grep -n`** avant d'éditer, ils bougent.

## Review Focus

1. **`link` change de sortie pour une cible gardée** : un octet d'écart sur Claude, Codex, OpenCode ou Copilot. Test d'étalon (Tâche 1), vérifié par mutation.
2. **`armadai` sans argument** : doit afficher l'aide, jamais lancer un shell ni paniquer. Test dans la Tâche 6.
3. **`unlink` sur un projet lié à Gemini avant la mise à jour** : doit réussir proprement ou échouer avec un message, jamais paniquer ni laisser un état partiel silencieux. Test dans la Tâche 8.
4. **CI désynchronisée** : un job qui passe encore `--features tui,...` donne « package does not have feature ». Chaque tâche met la CI à jour avec ses suppressions ; critère final dans la Tâche 10.
5. **Code mort sous `-D warnings`** : tout ce qui n'était utilisé que par une surface retirée. Chaque tâche de suppression se termine par un `clippy` sans `allow` ajouté.

---

## File Structure

- Créés : `crates/armadai/tests/link_golden.rs`, `crates/armadai/tests/golden/link/{claude,codex,copilot,opencode}.txt`, `crates/armadai/tests/gemini_unlink.rs`, `crates/armadai/tests/fixtures/gemini-linked/`, `crates/armadai/tests/no_subcommand.rs`, `frozen/armadai-audit/`.
- Supprimés : voir chaque tâche (liste exacte, commande `git rm`).
- Réécrits en fin de parcours : `.github/workflows/ci.yml`, `.claude/CLAUDE.md`, `.claude/agents/*.md`, `CLAUDE.md`, `README.md`, `docs/wiki/*`, `mise.toml`, `Dockerfile`.

---

### Task 0 (PR0a): Branche de base et tag local

**Files:** aucun fichier modifié.

- [ ] **Step 1: Partir d'un `master` à jour et propre**

```bash
git checkout master && git pull --ff-only && git status --short
```
Expected : `master` à jour ; les seuls fichiers non suivis sont ceux déjà présents au début de la session (`.superpowers/`, `graphify-out/`, etc.). Ne pas les ajouter.

- [ ] **Step 2: Poser le tag localement**

```bash
git tag -a pre-v1-declaratif -m "State before the declarative refocus (step 0 removals)"
git tag -l pre-v1-declaratif
```
Expected : `pre-v1-declaratif`.

- [ ] **Step 3: Demander à Dimitri avant de pousser le tag**

Ne pas exécuter `git push origin pre-v1-declaratif` sans accord. Si accordé : `git push origin pre-v1-declaratif`.

---

### Task 1 (PR0b): Étalon de la sortie de `link` pour les 4 cibles gardées

**Files:**
- Create: `crates/armadai/tests/link_golden.rs`
- Create: `crates/armadai/tests/golden/link/{claude,codex,copilot,opencode}.txt` (générés à l'étape 3)

**Interfaces:**
- Produces: un test `link_output_is_byte_identical_to_the_golden_snapshots` que toutes les tâches suivantes doivent laisser vert. Variable d'environnement `UPDATE_GOLDEN=1` pour régénérer.

- [ ] **Step 1: Vérifier les options réelles de `link`**

```bash
git checkout -b test/link-golden-snapshots
cargo run -q -p armadai -- link --help
```
Expected : la sortie contient `--target`, `--force` et une option pour choisir le coordinateur. Si l'option du coordinateur ne s'appelle pas `--coordinator`, remplacer la constante `COORDINATOR_ARGS` du test ci-dessous.

- [ ] **Step 2: Écrire le test**

```rust
//! Golden snapshots of what `armadai link` writes for the four targets kept by
//! the v1 refocus (claude, codex, copilot, opencode).
//!
//! Step 0 removes ~47k lines around `link` without touching it. This test is
//! the proof: the generated tree must stay byte-identical. Regenerate with
//! `UPDATE_GOLDEN=1 cargo test -p armadai --test link_golden` ONLY when a PR
//! deliberately changes link output, and say so in the PR body.

#[cfg(test)]
mod tests {
    use assert_cmd::Command;
    use std::path::{Path, PathBuf};

    /// (target, directory the linker writes into)
    const TARGETS: &[(&str, &str)] = &[
        ("claude", ".claude"),
        ("codex", ".codex"),
        ("copilot", ".github"),
        ("opencode", ".opencode"),
    ];

    /// Selects the coordinator. Adjust here if `link --help` names it differently.
    const COORDINATOR_ARGS: &[&str] = &["--coordinator", "lead"];

    const LEAD: &str = "# lead\n\n## Metadata\n\n- provider: anthropic\n- model: claude-sonnet-4-5-20250929\n- temperature: 0.4\n- tags: [coordination]\n\n## System Prompt\n\nLEAD_SYSTEM_MARKER — you coordinate the team.\n\n## Instructions\n\nLEAD_INSTRUCTIONS_MARKER — delegate, then synthesize.\n\n## Output Format\n\nLEAD_OUTPUT_MARKER — one paragraph.\n";

    const DEV: &str = "# dev\n\n## Metadata\n\n- provider: anthropic\n- model: claude-sonnet-4-5-20250929\n- temperature: 0.9\n- tags: [dev]\n\n## System Prompt\n\nDEV_SYSTEM_MARKER — you write the code.\n\n### Style\n\nDEV_SUBSECTION_MARKER — small functions.\n\n## Instructions\n\nDEV_INSTRUCTIONS_MARKER — tests first.\n";

    fn isolated_config(dir: &Path) -> PathBuf {
        let config = dir.join("config");
        std::fs::create_dir_all(&config).unwrap();
        config
    }

    fn project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        std::fs::create_dir_all(root.join("agents")).unwrap();
        std::fs::create_dir_all(root.join(".armadai")).unwrap();
        std::fs::write(
            root.join(".armadai/config.yaml"),
            "agents:\n  - path: agents/lead.md\n  - path: agents/dev.md\n",
        )
        .unwrap();
        std::fs::write(root.join("agents/lead.md"), LEAD).unwrap();
        std::fs::write(root.join("agents/dev.md"), DEV).unwrap();
        dir
    }

    /// Every file under `dir`, sorted by relative path, rendered as
    /// `=== <path> ===\n<content>\n`. The absolute project root is replaced by
    /// `<ROOT>` so the snapshot does not depend on the temp directory.
    fn render_tree(root: &Path, dir: &Path) -> String {
        fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    walk(&path, out);
                } else {
                    out.push(path);
                }
            }
        }
        let mut files = Vec::new();
        walk(dir, &mut files);
        files.sort();
        let root_str = root.display().to_string();
        let mut rendered = String::new();
        for file in files {
            let rel = file.strip_prefix(root).unwrap().display().to_string();
            let content = std::fs::read_to_string(&file)
                .unwrap_or_else(|e| panic!("reading {}: {e}", file.display()))
                .replace(&root_str, "<ROOT>");
            rendered.push_str(&format!("=== {rel} ===\n{content}\n"));
        }
        rendered
    }

    fn golden_path(target: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/golden/link")
            .join(format!("{target}.txt"))
    }

    #[test]
    fn link_output_is_byte_identical_to_the_golden_snapshots() {
        for (target, out_dir) in TARGETS {
            let dir = project();
            let root = dir.path().join("project");

            let mut cmd = Command::cargo_bin("armadai").unwrap();
            cmd.current_dir(&root)
                .env("ARMADAI_CONFIG_DIR", isolated_config(dir.path()))
                .args(["link", "--target", target, "--force"])
                .args(COORDINATOR_ARGS);
            let output = cmd.output().unwrap();
            assert!(
                output.status.success(),
                "link --target {target} failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );

            let produced = root.join(out_dir);
            assert!(
                produced.is_dir(),
                "target {target}: {} was not created",
                produced.display()
            );
            let rendered = render_tree(&root, &produced);
            // Guards against a vacuous snapshot: both markers must be present.
            assert!(
                rendered.contains("DEV_SYSTEM_MARKER"),
                "target {target}: the dev agent did not reach the output:\n{rendered}"
            );
            assert!(
                rendered.contains("LEAD_SYSTEM_MARKER"),
                "target {target}: the coordinator did not reach the output:\n{rendered}"
            );

            let golden = golden_path(target);
            if std::env::var_os("UPDATE_GOLDEN").is_some() {
                std::fs::create_dir_all(golden.parent().unwrap()).unwrap();
                std::fs::write(&golden, &rendered).unwrap();
                continue;
            }
            let expected = std::fs::read_to_string(&golden).unwrap_or_else(|_| {
                panic!(
                    "missing snapshot {} — generate it with UPDATE_GOLDEN=1",
                    golden.display()
                )
            });
            assert_eq!(
                rendered, expected,
                "target {target}: link output changed. If this is deliberate, \
                 regenerate with UPDATE_GOLDEN=1 and justify it in the PR."
            );
        }
    }
}
```

- [ ] **Step 3: Lancer sans snapshot et constater l'échec**

Run: `cargo test -p armadai --test link_golden --no-fail-fast`
Expected : FAIL avec « missing snapshot … generate it with UPDATE_GOLDEN=1 ». Si le test échoue autrement (agent absent, option inconnue), corriger le test avant de continuer.

- [ ] **Step 4: Générer les snapshots et les relire**

```bash
UPDATE_GOLDEN=1 cargo test -p armadai --test link_golden
cat crates/armadai/tests/golden/link/claude.txt
```
Expected : 4 fichiers créés. Relire au moins `claude.txt` et `codex.txt` : on doit y voir `DEV_SUBSECTION_MARKER`, le coordinateur et aucun chemin absolu de la machine. Si un chemin absolu ou une date apparaît, l'ajouter au remplacement de `render_tree` (par exemple remplacer la date par `<DATE>`) et régénérer.

- [ ] **Step 5: Vérifier que le test passe, deux fois de suite (déterminisme)**

Run: `cargo test -p armadai --test link_golden --no-fail-fast` deux fois.
Expected : `1 passed` les deux fois.

- [ ] **Step 6: Mutation, le test doit pouvoir échouer**

```bash
sed -i.bak 's/DEV_INSTRUCTIONS_MARKER/DEV_INSTRUCTIONS_MARKEX/' crates/armadai/tests/golden/link/opencode.txt
cargo test -p armadai --test link_golden --no-fail-fast
```
Expected : FAIL, message « target opencode: link output changed ». Puis restaurer : `mv crates/armadai/tests/golden/link/opencode.txt.bak crates/armadai/tests/golden/link/opencode.txt` et relancer : PASS. Reporter la sortie du rouge dans la PR.

- [ ] **Step 7: Barrière puis commit**

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --no-fail-fast
git add crates/armadai/tests/link_golden.rs crates/armadai/tests/golden/link
git commit -m "test(link): pin the generated output of the four kept targets" -m "Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```
Expected : tout vert (features par défaut à ce stade).

- [ ] **Step 8: Pousser la branche, ouvrir la PR, attendre la CI et une revue indépendante**

```bash
git push -u origin test/link-golden-snapshots
gh pr create --title "test(link): pin the generated output of the four kept targets" --body "$(cat <<'EOF'
## Résumé
Ajoute un test d'étalon de la sortie de `link` pour Claude, Codex, OpenCode et Copilot. Il sert de filet pour les PR de retrait de l'étape 0.

## Vérification
- Mutation : un octet modifié dans `opencode.txt` fait échouer le test (sortie dans le commentaire).

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```
Ne fusionner qu'après CI verte et revue indépendante, et seulement si le brief de Dimitri l'autorise.

---

### Task 2 (PR1): Tests e2e et tests de `run`

**Files:**
- Delete: `crates/armadai/tests/gaveldrop.rs`, `crates/armadai/tests/cases/`, `gaveldrop.yaml`, `crates/armadai-fake/`, `crates/armadai/src/bin/fake-claude.rs`, `crates/armadai/tests/run_dry_run_spends_nothing.rs`, `crates/armadai/tests/run_latest_tier_models.rs`, `crates/armadai/tests/run_sends_every_section.rs`, `crates/armadai/tests/budget_usage_warning.rs`, `crates/armadai/tests/pipe_declarative_agents.rs`, le script `gemini_cli_e2e.sh` (à localiser)
- Modify: `crates/armadai/Cargo.toml`, `crates/armadai/tests/prompt_section_boundaries.rs`, `.github/workflows/ci.yml`

**Interfaces:** Features encore existantes pour la barrière : `tui,web,storage,providers-api`.

- [ ] **Step 1: Branche et localisation du script Gemini**

```bash
git checkout master && git pull --ff-only
git checkout -b chore/step0-remove-e2e-and-run-tests
find . -name 'gemini_cli_e2e.sh' -not -path './target/*'
```

- [ ] **Step 2: Supprimer les fichiers**

```bash
git rm -r crates/armadai/tests/gaveldrop.rs crates/armadai/tests/cases gaveldrop.yaml crates/armadai-fake crates/armadai/src/bin/fake-claude.rs \
  crates/armadai/tests/run_dry_run_spends_nothing.rs crates/armadai/tests/run_latest_tier_models.rs \
  crates/armadai/tests/run_sends_every_section.rs crates/armadai/tests/budget_usage_warning.rs \
  crates/armadai/tests/pipe_declarative_agents.rs
git rm "$(find . -name gemini_cli_e2e.sh -not -path './target/*')"
```
Si `crates/armadai/src/bin/` devient vide, `git` le retire tout seul.

- [ ] **Step 3: Éditer `crates/armadai/Cargo.toml`**

Supprimer exactement ces blocs :
- le commentaire `# This package ships two bins (armadai + fake-claude); default \`cargo run\` to the main one.` (remplacer par rien) ; **garder** `default-run = "armadai"`.
- `[[bin]] name = "fake-claude" … required-features = ["e2e-fake"]` (4 lignes).
- `[[test]] name = "gaveldrop" required-features = ["e2e-fake"]` (3 lignes).
- dans `[features]` : la ligne `e2e-fake = ["dep:armadai-fake"]`.
- dans `[dependencies]` : la ligne `armadai-fake = { path = "../armadai-fake", optional = true, features = ["engine"] }`.
- dans `[dev-dependencies]` : les lignes `armadai-fake`, `gaveldrop`, `gaveldrop-fake`, `gaveldrop-conformance`.
- le bloc final complet `# \`openpty\` only … ` et `[target.'cfg(unix)'.dev-dependencies] libc = "0.2"` (la seule utilisation de `libc` était `run_dry_run_spends_nothing.rs`, vérifier : `grep -rn "libc::" crates/armadai/src crates/armadai/tests` ne doit rien rendre).

Conserver `armadai-core = { path = "../armadai-core", features = ["test-support"] }`, `assert_cmd` et `tempfile` dans `[dev-dependencies]`.

- [ ] **Step 4: Retirer le test de `run` de `prompt_section_boundaries.rs`**

```bash
grep -n "run_sends_the_whole_system_prompt_to_the_provider" crates/armadai/tests/prompt_section_boundaries.rs
```
Supprimer la fonction `#[test] fn run_sends_the_whole_system_prompt_to_the_provider` en entier (environ lignes 250-310) et tout `use`/constante qui n'est plus utilisé après (le compilateur les signale). Ne pas toucher à la liste `TARGETS` (Gemini part à la Tâche 8).

- [ ] **Step 5: Mettre à jour `.github/workflows/ci.yml`**

- Dans `clippy` : supprimer la ligne `cargo clippy --all-targets --no-default-features --features tui,storage,e2e-fake -- -D warnings` et son bloc de commentaire (lignes ~80-86).
- Dans `test` : remplacer `cargo test --no-default-features --features tui,storage,e2e-fake,web` par `cargo test --no-default-features --features tui,storage,web`, retirer les phrases sur `e2e-fake`/gaveldrop du commentaire, et supprimer l'étape `Upload gaveldrop report` (le bloc `- name: Upload gaveldrop report` jusqu'à `retention-days: 14`).

- [ ] **Step 6: Barrière**

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --no-default-features --features tui -- -D warnings
cargo clippy --all-targets --no-default-features --features tui,web,storage,providers-api -- -D warnings
cargo test --no-fail-fast --no-default-features --features tui,web,storage,providers-api
cargo build -p armadai-core
cargo test -p armadai --test link_golden --no-fail-fast
```
Expected : tout vert, y compris l'étalon. Puis :
```bash
grep -rn "e2e-fake\|gaveldrop\|fake-claude\|armadai-fake" --include='*.toml' --include='*.yml' --include='*.rs' --include='*.yaml' . | grep -v '^./target/' | grep -v '^./docs/'
```
Expected : aucune ligne.

- [ ] **Step 7: Commit, PR**

```bash
git add -A crates .github gaveldrop.yaml
git commit -m "chore(tests): remove the gaveldrop e2e suite and the run tests" -m "Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
git push -u origin chore/step0-remove-e2e-and-run-tests
gh pr create --title "chore(tests): remove the gaveldrop e2e suite and the run tests" --body "<corps en français, résumé + sortie de la barrière + étalon vert, terminé par la ligne Generated with Claude Code>"
```
Après fusion : la PR Dependabot `#435` (`libc`) devient sans objet. La fermer seulement sur accord de Dimitri.

---

### Task 3 (PR2): Web, `watch` et adaptateur Claude

**Files:**
- Delete: `crates/armadai/src/web/`, `crates/armadai/web/`, `crates/armadai/build.rs`, `crates/armadai/src/cli/watch.rs`, `crates/armadai/src/claude_adapter/{mapper.rs,transcript.rs,session_index.rs}`, `crates/armadai/assets/claude-plugin/`
- Modify: `crates/armadai/Cargo.toml`, `crates/armadai/src/main.rs`, `crates/armadai/src/cli/mod.rs`, `crates/armadai/src/claude_adapter/mod.rs`, `crates/armadai/tests/hook_stdout.rs`, `.github/workflows/ci.yml`, `mise.toml`

**Interfaces:** Features encore existantes : `tui,storage,providers-api`. Le hook `__claude-policy-gate` (`claude_adapter/policy_gate.rs`) doit rester intact.

- [ ] **Step 1: Branche**

```bash
git checkout master && git pull --ff-only
git checkout -b chore/step0-remove-web-and-watch
```

- [ ] **Step 2: Trouver les sites à éditer avant de supprimer**

```bash
grep -rn "mod web\|web::\|feature = \"web\"" crates/armadai/src
grep -rn "mod watch\|watch::\|Watch\b\|ClaudeRegisterSession\|register_session" crates/armadai/src
grep -rn "claude_adapter::\(mapper\|transcript\|session_index\)\|mapper::\|session_index::\|transcript::" crates/armadai/src
```
Noter chaque occurrence : elle doit disparaître ou être réécrite. `claude_adapter::transcript` est utilisé par `audit/usage/scan.rs` : **ne pas toucher à `audit`** dans cette PR (il est copié à la Tâche 7) ; garder donc `transcript.rs` tant que `audit` existe. Si c'est le cas, remplacer dans le Step 3 `git rm` de `transcript.rs` par rien, et l'ajouter à la liste de la Tâche 7.

- [ ] **Step 3: Supprimer**

```bash
git rm -r crates/armadai/src/web crates/armadai/web crates/armadai/build.rs crates/armadai/src/cli/watch.rs \
  crates/armadai/src/claude_adapter/mapper.rs crates/armadai/src/claude_adapter/session_index.rs \
  crates/armadai/assets/claude-plugin
```

- [ ] **Step 4: Éditer le code**

- `crates/armadai/src/main.rs` : supprimer la ligne `mod web;` (avec son `#[cfg(feature = "web")]`).
- `crates/armadai/src/cli/mod.rs` : supprimer `pub mod watch;` et `pub mod`/`mod` de `web`, les variantes `Web` et `Watch` de `enum Command`, la variante cachée `ClaudeRegisterSession`, leurs bras dans le `match`, et leurs lignes d'aide.
- `crates/armadai/src/claude_adapter/mod.rs` : ne garder que `pub mod policy_gate;` (et `pub mod transcript;` si conservé au Step 2).
- `crates/armadai/tests/hook_stdout.rs` : supprimer les deux tests qui appellent `__claude-register-session` (environ lignes 30-70) ; garder les trois tests de `__claude-policy-gate`.
- `crates/armadai/Cargo.toml` : supprimer `web = ["dep:axum", "dep:tower-http"]` de `[features]`, retirer `"web"` de `default`, supprimer les lignes `axum` et `tower-http` et leur commentaire `# HTTP server`.
- `mise.toml` : supprimer la tâche `web` et toute référence à `node` / `web/ui`.

- [ ] **Step 3bis: Mettre à jour `.github/workflows/ci.yml`**

Supprimer les combinaisons qui ne diffèrent que par `web` : dans `clippy`, remplacer `tui,web,storage` par `tui,storage` et `tui,web,storage,providers-api` par `tui,storage,providers-api` ; dans `test`, remplacer `tui,storage,web` par `tui,storage` et `tui,web,storage,providers-api` par `tui,storage,providers-api`. Retirer les paragraphes de commentaire consacrés à `web` (#350, #355).

- [ ] **Step 4bis: Barrière**

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --no-default-features --features tui -- -D warnings
cargo clippy --all-targets --no-default-features --features tui,storage,providers-api -- -D warnings
cargo test --no-fail-fast --no-default-features --features tui,storage,providers-api
cargo test -p armadai --test hook_stdout --no-fail-fast
cargo test -p armadai --test link_golden --no-fail-fast
cargo build -p armadai-core
cargo tree -i axum 2>&1 | tail -2
```
Expected : tout vert ; `hook_stdout` : 3 tests ; `cargo tree -i axum` répond « did not match any packages ».

- [ ] **Step 5: Mutation du hook conservé**

Dans `crates/armadai/src/claude_adapter/policy_gate.rs`, inverser temporairement la condition qui décide du refus (la ligne qui produit `permissionDecision` `deny`, repérée par `grep -n "deny" crates/armadai/src/claude_adapter/policy_gate.rs`), lancer `cargo test -p armadai --test hook_stdout --no-fail-fast` : au moins un test doit passer au rouge. Restaurer avec `git checkout crates/armadai/src/claude_adapter/policy_gate.rs`. Reporter la sortie.

- [ ] **Step 6: Commit, PR**

```bash
git add -A
git commit -m "chore(web): remove the web dashboard, watch and the Claude session adapter" -m "Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
git push -u origin chore/step0-remove-web-and-watch
gh pr create --title "chore(web): remove the web dashboard, watch and the Claude session adapter" --body "<corps en français, barrière + mutation du hook, terminé par la ligne Generated with Claude Code>"
```
Après fusion : fermer (sur accord de Dimitri) les PR Dependabot de `web/ui` et de `tower-http` (`#428`, `#429`, `#438`, `#440`, et `#439` si l'exemple `demo-nextjs-shop` n'est plus gardé).

---

### Task 4 (PR3a): `run`, stockage et infra Docker

**Files:**
- Delete: `crates/armadai/src/cli/{run.rs,run_es_record.rs,run_replay.rs,history.rs,costs.rs,projections.rs,up.rs}`, `crates/armadai/src/{db.rs,es_log.rs}`, `crates/armadai/src/test_support.rs`, `crates/armadai-storage/`, `docker-compose.yml`
- Modify: `crates/armadai/Cargo.toml`, `crates/armadai/src/main.rs`, `crates/armadai/src/cli/mod.rs`, `Cargo.toml` (racine), `Dockerfile`, `.github/workflows/ci.yml`

**Interfaces:** Features encore existantes : `tui,providers-api`. `shell/` et `tui/` existent encore et **ne doivent plus appeler** `run` : ils partent à la Tâche 5, mais la compilation doit rester verte, donc cette PR retire aussi les appels de `shell/` vers `cli::run` en les supprimant (voir Step 2).

- [ ] **Step 1: Branche et inventaire des appels**

```bash
git checkout master && git pull --ff-only
git checkout -b chore/step0-remove-run-and-storage
grep -rn "cli::run\|run::\|run_es_record\|run_replay\|es_log\|crate::db\|armadai_storage\|rusqlite\|feature = \"storage\"" crates/armadai/src | grep -v "^crates/armadai/src/cli/run"
```

- [ ] **Step 2: Si `shell/` ou `tui/` appellent `run`, fusionner les Tâches 4 et 5**

Si la commande du Step 1 montre des appels de `shell/` ou `tui/` vers `cli::run`, `es_log` ou `db`, ils ne peuvent pas être découplés proprement. Dans ce cas, **ne pas improviser** : exécuter la Tâche 5 sur la même branche avant la barrière, et ouvrir une seule PR « run + shell + tui + storage ». Le relevé indique que `run` référence `shell::run_view`, donc ce cas est probable.

- [ ] **Step 3: Supprimer**

```bash
git rm -r crates/armadai/src/cli/run.rs crates/armadai/src/cli/run_es_record.rs crates/armadai/src/cli/run_replay.rs \
  crates/armadai/src/cli/history.rs crates/armadai/src/cli/costs.rs crates/armadai/src/cli/projections.rs crates/armadai/src/cli/up.rs \
  crates/armadai/src/db.rs crates/armadai/src/es_log.rs crates/armadai/src/test_support.rs \
  crates/armadai-storage docker-compose.yml
```

- [ ] **Step 4: Éditer le code et les manifestes**

- `crates/armadai/src/main.rs` : supprimer `mod db;`, `mod es_log;`, `mod test_support;` (avec leurs `#[cfg]`).
- `crates/armadai/src/cli/mod.rs` : supprimer les `pub mod` correspondants, les variantes `Run`, `History`, `Costs`, `Projections`, `Up`, `Down` et leurs bras ; retirer les exemples `armadai run …` de `about`/`after_help`.
- `crates/armadai/Cargo.toml` : supprimer `storage = [...]`, retirer `"storage"` de `default`, supprimer `armadai-storage`, `rusqlite`, et leurs commentaires.
- `Cargo.toml` (racine) : supprimer `rusqlite` de `[workspace.dependencies]` ; supprimer `uuid` seulement si `grep -rn "uuid" crates --include='*.rs' --include='Cargo.toml'` ne rend plus rien d'utile (sinon le garder).
- `Dockerfile` : remplacer `--features tui,storage` par rien (build par défaut) ; relire le fichier en entier et supprimer ce qui référence SurrealDB/LiteLLM.
- `.github/workflows/ci.yml` : retirer `storage` de toutes les combinaisons ; dans `build`, remplacer `cargo build --release --no-default-features --features tui,storage` par `cargo build --release --no-default-features --features tui`.

- [ ] **Step 5: Barrière**

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --no-default-features --features tui -- -D warnings
cargo clippy --all-targets --no-default-features --features tui,providers-api -- -D warnings
cargo test --no-fail-fast --no-default-features --features tui
cargo test --no-fail-fast --no-default-features --features tui,providers-api
cargo test -p armadai --test link_golden --no-fail-fast
cargo build -p armadai-core
cargo tree -i rusqlite 2>&1 | tail -2
```
Expected : tout vert ; `rusqlite` introuvable dans le graphe. Si `clippy` signale du code mort, **supprimer** ce code (voir Global Constraints).

- [ ] **Step 6: Commit, PR**

```bash
git add -A
git commit -m "chore(run): remove run, the event log and sqlite storage" -m "Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
git push -u origin chore/step0-remove-run-and-storage
gh pr create --title "chore(run): remove run, the event log and sqlite storage" --body "<corps en français, terminé par la ligne Generated with Claude Code>"
```

---

### Task 5 (PR3b): Shell, TUI et comportement sans argument

**Files:**
- Delete: `crates/armadai/src/shell/`, `crates/armadai/src/tui/`, `crates/armadai/src/theme.rs`
- Modify: `crates/armadai/Cargo.toml`, `crates/armadai/src/main.rs`, `crates/armadai/src/cli/mod.rs`, `.github/workflows/ci.yml`, `crates/armadai/src/cli/style.rs`
- Create: `crates/armadai/tests/no_subcommand.rs`

**Interfaces:** Après cette tâche, plus aucune feature de l'interface n'existe ; seule reste `providers-api` à la Tâche 8.

- [ ] **Step 1: Branche (ou poursuite de la Tâche 4 si fusionnée) et écriture du test d'abord**

```bash
git checkout master && git pull --ff-only
git checkout -b chore/step0-remove-shell-and-tui
```
`crates/armadai/tests/no_subcommand.rs` :

```rust
//! `armadai` with no subcommand used to start the conversational shell. The
//! shell is gone: it must print the help and exit, never hang or panic.

#[cfg(test)]
mod tests {
    use assert_cmd::Command;

    #[test]
    fn no_subcommand_prints_the_help_instead_of_starting_a_shell() {
        let dir = tempfile::tempdir().unwrap();
        let output = Command::cargo_bin("armadai")
            .unwrap()
            .current_dir(dir.path())
            .env("ARMADAI_CONFIG_DIR", dir.path().join("config"))
            .timeout(std::time::Duration::from_secs(10))
            .output()
            .unwrap();
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            text.contains("Usage:"),
            "expected the help on no subcommand, got:\n{text}"
        );
        assert!(
            text.contains("link"),
            "the help must list the kept commands, got:\n{text}"
        );
        assert!(
            !text.contains("panicked"),
            "no subcommand must not panic:\n{text}"
        );
    }
}
```

- [ ] **Step 2: Lancer et constater l'échec (avant le changement)**

Run: `cargo test -p armadai --test no_subcommand --no-fail-fast`
Expected : FAIL (aujourd'hui `armadai` lance le shell, ou échoue sur un terminal non interactif, mais n'affiche pas `Usage:`). Noter la sortie réellement observée.

- [ ] **Step 3: Supprimer et éditer**

```bash
git rm -r crates/armadai/src/shell crates/armadai/src/tui crates/armadai/src/theme.rs
grep -n "run_shell\|arg_required_else_help\|command: Option<Command>\|None =>" crates/armadai/src/cli/mod.rs
```
- `crates/armadai/src/main.rs` : supprimer `mod shell;`, `mod tui;`, `mod theme;`.
- `crates/armadai/src/cli/mod.rs` : supprimer les variantes `Shell` et `Tui` et leurs bras ; sur le `struct` clap racine, ajouter `#[command(arg_required_else_help = true)]` (ou l'attribut équivalent de la structure existante) et supprimer le bras `None => … run_shell(false)` en rendant la sous-commande obligatoire (`command: Command` au lieu de `Option<Command>`), ou, si l'option est conservée, faire que `None` appelle `Cli::command().print_help()`.
- `crates/armadai/Cargo.toml` : supprimer `tui = [...]`, retirer `"tui"` de `default` (il ne reste que `providers-api`), supprimer `ratatui`, `crossterm`, `unicode-width`, `portable-pty`, `strip-ansi-escapes` et leurs commentaires, ainsi que `regex` et `uuid` si `grep -rn "regex::\|uuid::" crates/armadai/src` ne rend plus rien.
- `crates/armadai/src/cli/style.rs` : supprimer `agent()` si plus aucun appelant (`grep -rn "style::agent" crates/armadai/src`).
- `.github/workflows/ci.yml` : remplacer toutes les combinaisons par les deux qui subsistent : `--no-default-features` (sans features) et `--no-default-features --features providers-api`.

- [ ] **Step 4: Barrière**

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --no-default-features -- -D warnings
cargo clippy --all-targets --no-default-features --features providers-api -- -D warnings
cargo test --no-fail-fast --no-default-features
cargo test --no-fail-fast --no-default-features --features providers-api
cargo test -p armadai --test no_subcommand --test link_golden --no-fail-fast
cargo build -p armadai-core
cargo tree -i ratatui 2>&1 | tail -2
cargo tree -i portable-pty 2>&1 | tail -2
```
Expected : tout vert, y compris `no_subcommand` (qui était rouge au Step 2).

- [ ] **Step 5: Mutation du test**

Dans `cli/mod.rs`, retirer temporairement `arg_required_else_help` (ou la sortie d'aide), relancer `cargo test -p armadai --test no_subcommand --no-fail-fast` : doit être rouge. Restaurer avec `git checkout crates/armadai/src/cli/mod.rs` **sans perdre vos autres modifications** (si le fichier est déjà modifié, annuler à la main la seule ligne modifiée). Reporter la sortie.

- [ ] **Step 6: Commit, PR**

```bash
git add -A
git commit -m "chore(shell): remove the conversational shell and the TUI" -m "Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
git push -u origin chore/step0-remove-shell-and-tui
gh pr create --title "chore(shell): remove the conversational shell and the TUI" --body "<corps en français ; mentionner le changement de comportement de armadai sans argument ; terminé par la ligne Generated with Claude Code>"
```

---

### Task 6 (PR4a): Isoler `audit` dans `frozen/armadai-audit`

**Files:**
- Move: `crates/armadai/src/audit/` → `frozen/armadai-audit/src/audit/` ; `crates/armadai/src/cli/audit.rs` → `frozen/armadai-audit/src/main.rs` (adapté) ; `crates/armadai/tests/audit_{rightsizing,scopes,usage}.rs` → `frozen/armadai-audit/tests/`
- Create: `frozen/armadai-audit/Cargo.toml`, `frozen/armadai-audit/README.md`, `frozen/armadai-audit/src/vendored/*.rs`
- Modify: `Cargo.toml` (racine), `crates/armadai/src/cli/mod.rs`, `crates/armadai/src/main.rs`, `.github/dependabot.yml` (si utile)

**Interfaces:** Le crate isolé n'est **pas** un membre du workspace : il porte son propre `[workspace]` vide et est listé dans `exclude` à la racine, donc ni la CI ni `cargo test` du workspace ne le compilent.

- [ ] **Step 1: Branche et mesure du coût de la copie (point de décision)**

```bash
git checkout master && git pull --ff-only
git checkout -b chore/step0-freeze-audit
grep -rhn "crate::cli::style\|crate::linker::\|crate::claude_adapter::\|armadai_providers::\|armadai_core::provider" crates/armadai/src/audit crates/armadai/src/cli/audit.rs | sed 's/:.*//' | sort | uniq -c | sort -rn | head
```
Lister les modules externes à `audit` qu'il utilise. **Point de décision** : si la copie figée de ces dépendances dépasse **8 fichiers ou 1 500 lignes**, **s'arrêter** et demander à Dimitri s'il préfère le repli (dossier source non compilé + README + tag). Ne pas continuer sans réponse.

- [ ] **Step 2: Créer le crate isolé**

```bash
mkdir -p frozen/armadai-audit/src frozen/armadai-audit/tests
git mv crates/armadai/src/audit frozen/armadai-audit/src/audit
git mv crates/armadai/src/cli/audit.rs frozen/armadai-audit/src/cli_audit.rs
git mv crates/armadai/tests/audit_rightsizing.rs crates/armadai/tests/audit_scopes.rs crates/armadai/tests/audit_usage.rs frozen/armadai-audit/tests/
```
`frozen/armadai-audit/Cargo.toml` :

```toml
[package]
name = "armadai-audit"
version = "0.0.0"
edition = "2024"
publish = false
description = "FROZEN: the `armadai audit` engine, isolated from the workspace until it is rebuilt on the declarative model (post-v1)."

# Not a member of the root workspace: keeps CI and `cargo test` at the root unaffected.
[workspace]

[[bin]]
name = "armadai-audit"
path = "src/main.rs"

[dependencies]
armadai-core = { path = "../../crates/armadai-core" }
```
Ajouter à `[dependencies]` les crates réellement utilisés par `audit` (repérer avec `grep -rhn "^use " frozen/armadai-audit/src | sort -u` : `clap`, `serde`, `serde_yaml_ng`, `serde_json`, `anyhow`, `chrono`, `regex`, `uuid`, `tracing`, `dialoguer`, `anstyle`, `anstream`, `sha2`, etc.) en recopiant les versions de `crates/armadai/Cargo.toml`. Ajouter `[dev-dependencies]` : `assert_cmd = "2.2.2"`, `tempfile = "3"`.

- [ ] **Step 3: Copier les dépendances externes en `vendored/`**

Pour chaque module externe repéré au Step 1 (`cli::style`, les éléments de `linker` utilisés comme `slugify`, `LinkAgent`, `Linker`, `claude_adapter::transcript`, ainsi que `armadai_providers::{model_registry::fetch, factory}` et `armadai_core::provider` si utilisés) : copier le fichier dans `frozen/armadai-audit/src/vendored/` (`git show master:<chemin> > …`), l'ajouter à un `vendored/mod.rs`, et remplacer les chemins `crate::cli::style` etc. par `crate::vendored::…` dans `audit`. Remplacer l'appel à `create_provider` (usage `--deep`) par une erreur explicite `anyhow::bail!("--deep needs an execution provider and is unavailable in the frozen build")`.

- [ ] **Step 4: Écrire `src/main.rs` du crate gelé**

Reprendre le contenu de `cli_audit.rs` : un `fn main()` qui construit les arguments `clap` du sous-ensemble `audit` et appelle la fonction `execute` d'origine. Corriger dans `tests/*.rs` : `Command::cargo_bin("armadai")` → `Command::cargo_bin("armadai-audit")` et l'argument `audit` du début de ligne de commande s'il est passé explicitement.

- [ ] **Step 5: Vérifier que le crate gelé compile et passe ses tests**

```bash
cd frozen/armadai-audit && cargo check --all-targets && cargo test --no-fail-fast; cd ../..
```
Expected : compile ; les trois fichiers de tests `audit_*` passent. Un échec qui vient d'une dépendance oubliée en `vendored/` se corrige en copiant le fichier manquant, dans la limite fixée au Step 1.

- [ ] **Step 6: Détacher `audit` du workspace principal**

- `Cargo.toml` (racine) : ajouter `exclude = ["frozen/armadai-audit"]` sous `members = ["crates/*"]`.
- `crates/armadai/src/main.rs` : supprimer `mod audit;`.
- `crates/armadai/src/cli/mod.rs` : supprimer `pub mod audit;`, la variante `Audit`, son bras et son aide.
- `frozen/armadai-audit/README.md` : une page : ce que fait le crate, pourquoi il est gelé (l'étape 0 retire ses dépendances du binaire), comment le compiler (`cd frozen/armadai-audit && cargo test`), et le renvoi vers le chantier post-v1 qui le reconstruira.

- [ ] **Step 7: Barrière (workspace principal et crate gelé)**

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --no-default-features -- -D warnings
cargo clippy --all-targets --no-default-features --features providers-api -- -D warnings
cargo test --no-fail-fast --no-default-features
cargo test -p armadai --test link_golden --no-fail-fast
cargo build -p armadai-core
(cd frozen/armadai-audit && cargo test --no-fail-fast)
```
Expected : tout vert. La CI ne compile pas le crate gelé ; c'est voulu.

- [ ] **Step 8: Commit, PR**

```bash
git add -A
git commit -m "chore(audit): isolate the audit engine in a frozen crate" -m "Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
git push -u origin chore/step0-freeze-audit
gh pr create --title "chore(audit): isolate the audit engine in a frozen crate" --body "<corps en français ; indiquer le nombre de fichiers copiés en vendored ; terminé par la ligne Generated with Claude Code>"
```

---

### Task 7 (PR4b): Retirer `registry`

**Files:**
- Delete: `crates/armadai/src/registry/`, `crates/armadai/src/cli/registry.rs` (après extraction)
- Modify: `crates/armadai/src/cli/init.rs`, `crates/armadai/src/starters_registry/mod.rs`, `crates/armadai/src/cli/models.rs`, `crates/armadai/src/cli/mod.rs`, `crates/armadai/src/main.rs`

- [ ] **Step 1: Branche et lecture du morceau à extraire**

```bash
git checkout master && git pull --ff-only
git checkout -b chore/step0-remove-registry
grep -n "effective_starter_sources\|MODELS_DEV_URL" crates/armadai/src/cli/registry.rs crates/armadai/src/cli/init.rs
```
Lire en entier la fonction `effective_starter_sources` et tout ce qu'elle appelle dans `cli/registry.rs`.

- [ ] **Step 2: Extraire d'abord**

Déplacer `effective_starter_sources` (et ses seuls dépendants dans `registry.rs`) vers `crates/armadai/src/starters_registry/mod.rs`, la rendre `pub`, et dans `cli/init.rs:102` remplacer `crate::cli::registry::effective_starter_sources` par `crate::starters_registry::effective_starter_sources`. Si la fonction affiche `model_registry::fetch::MODELS_DEV_URL`, remplacer cet affichage par la constante en dur ou la retirer : ce n'est pas le rôle de `starters_registry`.

- [ ] **Step 3: Vérifier que `init` marche toujours avant de supprimer**

```bash
cargo test --no-fail-fast --no-default-features
cargo test -p armadai --test link_golden --no-fail-fast
cargo run -q -p armadai -- init --help
```
Expected : vert, l'aide de `init` s'affiche.

- [ ] **Step 4: Supprimer et nettoyer**

```bash
git rm -r crates/armadai/src/registry crates/armadai/src/cli/registry.rs
```
- `crates/armadai/src/main.rs` : supprimer `mod registry;`.
- `crates/armadai/src/cli/mod.rs` : supprimer `pub mod registry;`, la variante `Registry` et son bras.
- `crates/armadai/src/cli/models.rs` (`grep -n "armadai run" crates/armadai/src/cli/models.rs`) : remplacer les messages `run \`armadai run\`` par `run \`armadai link\``.

- [ ] **Step 5: Barrière**

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --no-default-features -- -D warnings
cargo clippy --all-targets --no-default-features --features providers-api -- -D warnings
cargo test --no-fail-fast --no-default-features
cargo test --no-fail-fast --no-default-features --features providers-api
cargo test -p armadai --test link_golden --no-fail-fast
cargo build -p armadai-core
```

- [ ] **Step 6: Commit, PR**

```bash
git add -A
git commit -m "chore(registry): remove the awesome-copilot registry" -m "Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
git push -u origin chore/step0-remove-registry
gh pr create --title "chore(registry): remove the awesome-copilot registry" --body "<corps en français, terminé par la ligne Generated with Claude Code>"
```

---

### Task 8 (PR5a): Retirer Gemini

**Files:**
- Delete: `crates/armadai/src/linker/gemini.rs`
- Modify: `crates/armadai/src/linker/mod.rs`, `crates/armadai/src/linker/model_resolution.rs`, `crates/armadai/src/cli/new.rs`, `crates/armadai/src/cli/link.rs`, `crates/armadai/src/cli/unlink.rs`, `crates/armadai/tests/prompt_section_boundaries.rs`
- Create: `crates/armadai/tests/fixtures/gemini-linked/`, `crates/armadai/tests/gemini_unlink.rs`

- [ ] **Step 1: Branche et fabrication de la fixture AVANT la suppression**

```bash
git checkout master && git pull --ff-only
git checkout -b chore/step0-remove-gemini
tmp=$(mktemp -d) && mkdir -p "$tmp/project/agents" "$tmp/project/.armadai" "$tmp/config"
printf '%s\n' '# nested' '' '## Metadata' '' '- provider: anthropic' '- model: claude-sonnet-4-5-20250929' '' '## System Prompt' '' 'Hello.' > "$tmp/project/agents/nested.md"
printf 'agents:\n  - path: agents/nested.md\n' > "$tmp/project/.armadai/config.yaml"
(cd "$tmp/project" && ARMADAI_CONFIG_DIR="$tmp/config" cargo run -q --manifest-path "$OLDPWD/Cargo.toml" -p armadai -- link --target gemini --force)
mkdir -p crates/armadai/tests/fixtures/gemini-linked
cp -R "$tmp/project/." crates/armadai/tests/fixtures/gemini-linked/
find crates/armadai/tests/fixtures/gemini-linked -type f | sort
```
Expected : la fixture contient `.gemini/agents/nested.md`, `.armadai/link-manifest.yaml`, `agents/nested.md`, `.armadai/config.yaml`. Si le manifeste contient un chemin absolu du répertoire temporaire, le remplacer par un chemin relatif ou le noter dans la PR.

- [ ] **Step 2: Écrire le test d'`unlink`**

`crates/armadai/tests/gemini_unlink.rs` :

```rust
//! A project linked to Gemini before the refocus must still unlink cleanly or
//! fail with a readable message — never panic, never silently half-undo.

#[cfg(test)]
mod tests {
    use assert_cmd::Command;
    use std::path::Path;

    fn copy_dir(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let target = to.join(entry.file_name());
            if entry.path().is_dir() {
                copy_dir(&entry.path(), &target);
            } else {
                std::fs::copy(entry.path(), target).unwrap();
            }
        }
    }

    #[test]
    fn unlinking_a_project_linked_to_gemini_never_panics_or_half_undoes() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        copy_dir(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/gemini-linked"),
            &root,
        );
        let generated = root.join(".gemini/agents/nested.md");
        assert!(generated.is_file(), "fixture must contain the Gemini output");

        let output = Command::cargo_bin("armadai")
            .unwrap()
            .current_dir(&root)
            .env("ARMADAI_CONFIG_DIR", dir.path().join("config"))
            .args(["unlink", "--force"])
            .output()
            .unwrap();
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!text.contains("panicked"), "unlink panicked:\n{text}");
        if output.status.success() {
            assert!(
                !generated.exists(),
                "unlink reported success but left {} behind:\n{text}",
                generated.display()
            );
        } else {
            assert!(
                !text.trim().is_empty(),
                "unlink failed with no message at all"
            );
            assert!(
                generated.exists(),
                "unlink failed yet deleted the generated file: half-undone state"
            );
        }
    }
}
```
Vérifier d'abord avec `cargo run -q -p armadai -- unlink --help` que `--force` existe ; sinon retirer l'argument et adapter. Lancer le test **avant** la suppression : `cargo test -p armadai --test gemini_unlink --no-fail-fast` ; noter dans la PR quelle branche (succès ou échec lisible) est observée aujourd'hui.

- [ ] **Step 3: Supprimer Gemini**

```bash
git rm crates/armadai/src/linker/gemini.rs
grep -n "gemini\|Gemini" crates/armadai/src/linker/mod.rs crates/armadai/src/linker/model_resolution.rs crates/armadai/src/cli/new.rs crates/armadai/src/cli/link.rs crates/armadai/src/cli/unlink.rs crates/armadai/tests/prompt_section_boundaries.rs
```
Éditer chaque hit : dans `linker/mod.rs` retirer `mod gemini;`, le `pub use gemini::…`, la variante `LinkTarget::Gemini`, le bras `"gemini" =>` de `create_linker` et le test `test_create_linker_gemini` ; dans `model_resolution.rs` retirer `"gemini"` de `classify_target` et le test associé ; dans `cli/new.rs` retirer `"gemini"` de `WIZARD_PROVIDER_CHOICES` **seulement** s'il désigne la cible de link (le conserver si c'est un nom de provider d'agent) ; mettre à jour les commentaires de `cli/link.rs` et `cli/unlink.rs`. Dans `prompt_section_boundaries.rs` : retirer `("gemini", ".gemini/agents/nested.md")` de `TARGETS` et réécrire `link_keeps_metadata_fields_declared_after_a_subsection` sur `opencode` (qui émet `temperature`, d'après le commentaire du test) en remplaçant la cible et le chemin attendu `.opencode/agents/nested.md`.

- [ ] **Step 4: Tests de cohérence du linker**

`linker/mod.rs`, module de test `provider_inventories` (`grep -n "mod provider_inventories" crates/armadai/src/linker/mod.rs`) : supprimer les tests qui importent `armadai_providers::factory::{accepted_provider_names, create_provider}` ou `json_runner::json_capable_clis` (`every_link_target_is_a_provider_run_can_execute`, `every_json_relayable_cli…`, `the_link_targets_and_the_json…`, `the_only_runnable_tools_without_a_link_target…`, `the_new_wizard_offers_exactly…`). **Garder** `every_advertised_link_target_can_actually_be_built` et `the_model_resolution_preview_covers_every_link_target`.

- [ ] **Step 5: Barrière**

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --no-default-features -- -D warnings
cargo clippy --all-targets --no-default-features --features providers-api -- -D warnings
cargo test --no-fail-fast --no-default-features
cargo test --no-fail-fast --no-default-features --features providers-api
cargo test -p armadai --test link_golden --test gemini_unlink --test prompt_section_boundaries --no-fail-fast
cargo build -p armadai-core
grep -rni "gemini" crates/armadai/src crates/armadai/tests --include='*.rs' | grep -v "tests/fixtures"
```
Expected : tout vert ; la dernière commande ne rend plus que des hits qu'il faut justifier ou supprimer.

- [ ] **Step 6: Mutation du test d'`unlink`**

Dans `cli/unlink.rs`, faire temporairement paniquer le chemin de manifeste (`unlink_from_manifest`) avec `panic!("x")` en tête de fonction, lancer `cargo test -p armadai --test gemini_unlink --no-fail-fast` : doit être rouge (« unlink panicked »). Restaurer par `git checkout crates/armadai/src/cli/unlink.rs` si le fichier n'a pas d'autres modifications, sinon retirer la ligne. Reporter la sortie.

- [ ] **Step 7: Commit, PR**

```bash
git add -A
git commit -m "chore(link): remove the Gemini target" -m "Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
git push -u origin chore/step0-remove-gemini
gh pr create --title "chore(link): remove the Gemini target" --body "<corps en français ; indiquer le comportement observé de unlink sur la fixture ; terminé par la ligne Generated with Claude Code>"
```

---

### Task 9 (PR5b): Providers d'exécution, feature `providers-api` et nettoyage du core

**Files:**
- Delete: `crates/armadai-providers/src/{api/,cli.rs,json_runner.rs,proxy.rs,rate_limiter.rs}` ; `crates/armadai-core/src/{events.rs,provider.rs}` ; `crates/armadai-core/src/orchestration/` sauf `mod.rs` et `policy.rs` ; `crates/armadai/src/cli/audit_*` n'existe plus
- Modify: `crates/armadai-providers/{Cargo.toml,src/lib.rs,src/factory.rs}`, `crates/armadai-core/{Cargo.toml,src/lib.rs,src/orchestration/mod.rs,src/routing.rs,src/pack_validation.rs}`, `crates/armadai/Cargo.toml`, `crates/armadai/src/linker/model_resolution.rs`, `crates/armadai/src/cli/{link.rs,unlink.rs,new.rs}`, `crates/armadai/src/starters_registry/mod.rs`, `.github/workflows/ci.yml`

**Interfaces:** Après cette tâche, `link` ne touche jamais le réseau ; `models update` et `starters_registry` gardent `reqwest` (dépendance directe, sans feature).

- [ ] **Step 1: Branche et inventaire des sites `providers-api`**

```bash
git checkout master && git pull --ff-only
git checkout -b chore/step0-remove-exec-providers-and-engine
grep -rn "providers-api\|providers_api" crates --include='*.rs' --include='Cargo.toml' | grep -v "^crates/armadai-providers/src/api"
grep -rn "armadai_providers::" crates/armadai/src
```

- [ ] **Step 2: Ce que `link` ne fait plus**

Supprimer la branche `#[cfg(feature = "providers-api")]` de `linker/model_resolution.rs` (`remap_models_for_llm_editor`, `prompt_model_interactive` qui appelait `load_models_online`) et garder la branche **sans réseau** (résolution par table et cache local) : une seule fonction, sans `cfg`. Faire pareil dans `cli/link.rs` (lignes ~165 et ~197), `cli/unlink.rs` (~909) et `cli/new.rs` (~346). Vérifier : `grep -rn "load_models_online" crates` ne rend plus que des appels explicites depuis `models update`.

- [ ] **Step 3: Écrire le test qui interdit le réseau à `link`**

Dans `crates/armadai/tests/link_golden.rs`, ajouter ce test (il échoue si `link` essaie de joindre le réseau) :

```rust
    /// `link` must never reach the network (decision of 2026-10-09): pointing
    /// every proxy variable at a closed local port makes any attempt fail fast.
    #[test]
    fn link_does_not_touch_the_network() {
        let dir = project();
        let root = dir.path().join("project");
        let output = Command::cargo_bin("armadai")
            .unwrap()
            .current_dir(&root)
            .env("ARMADAI_CONFIG_DIR", isolated_config(dir.path()))
            .env("HTTPS_PROXY", "http://127.0.0.1:9")
            .env("HTTP_PROXY", "http://127.0.0.1:9")
            .env("ALL_PROXY", "http://127.0.0.1:9")
            .env_remove("NO_PROXY")
            .args(["link", "--target", "opencode", "--force"])
            .args(COORDINATOR_ARGS)
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success(),
            "link failed with the network closed: {text}"
        );
        assert!(
            !text.to_lowercase().contains("connection refused"),
            "link tried to reach the network: {text}"
        );
    }
```
Mutation : ré-introduire temporairement un appel `load_models_online()` dans le chemin de `link` ; le test doit passer au rouge ; restaurer. Reporter la sortie.

- [ ] **Step 4: Supprimer les providers d'exécution**

```bash
git rm -r crates/armadai-providers/src/api crates/armadai-providers/src/cli.rs crates/armadai-providers/src/json_runner.rs \
  crates/armadai-providers/src/proxy.rs crates/armadai-providers/src/rate_limiter.rs
```
Dans `crates/armadai-providers/src/lib.rs` et `factory.rs` : ne garder que `model_registry` (`ModelEntry`, `fetch`, cache) et `api_backend_for_tool`/`KNOWN_TOOLS` (table pure) ; supprimer `create_provider`, `accepted_provider_names` et les `mod`/`use` orphelins. Dans `crates/armadai-providers/Cargo.toml` : supprimer la feature `api` et la dépendance `armadai-secrets` si elle n'est plus utilisée, **garder `reqwest` en dépendance directe sans option** (utilisé par `model_registry::fetch` pour `models update`).

- [ ] **Step 5: Retirer la feature du binaire**

`crates/armadai/Cargo.toml` : supprimer `providers-api = [...]` de `[features]` (la section `[features]` peut disparaître entièrement), supprimer `default = [...]`, passer `reqwest = { workspace = true }` en dépendance non optionnelle, supprimer `tokio-stream`, `async-trait`, `futures-util`, `thiserror` (si `grep -rn "thiserror" crates/armadai/src` ne rend rien) et réduire `tokio = { workspace = true, features = ["full"] }` à `features = ["rt-multi-thread", "macros", "process"]` (`cargo build` dira si `time`, `sync` ou `fs` manquent : les ajouter un par un). `.github/workflows/ci.yml` : remplacer les deux combinaisons restantes par une seule (voir Tâche 10 pour la version finale).

- [ ] **Step 6: Nettoyer le core**

```bash
git rm crates/armadai-core/src/events.rs crates/armadai-core/src/provider.rs
ls crates/armadai-core/src/orchestration
```
Supprimer tout ce qui est dans `orchestration/` **sauf** `mod.rs` et `policy.rs`, avec `git rm -r` sur chaque fichier et dossier (dont `es/`). Dans `orchestration/mod.rs` : retirer les `pub mod` supprimés, `PatternConfig`, `AgentRelationship`, `classify_relationship` et `arc_vec_serde` s'il devient orphelin ; **garder** `OrchestrationPattern`, `TriggerConfig`, `AgentRingConfig`, `OrchestrationConfig`, `TeamConfig`, `NestedPattern`, `OrchestrationValidationError`, `validate_config`. Dans `armadai-core/src/lib.rs` : retirer `pub mod events;` et `pub mod provider;`. `routing.rs` : ne garder que `RoutingRules`, `LengthThresholds`, `Keywords` (utilisés par `ProjectConfig.routing`) ; supprimer `route()`, `BudgetState`, `RouteReason`. `armadai-core/Cargo.toml` : supprimer `tokio-stream`, `async-trait`, `futures-util` et réduire `tokio` à ce qui compile.

- [ ] **Step 7: Barrière (une seule configuration de features)**

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --no-fail-fast
cargo test -p armadai --test link_golden --test gemini_unlink --test no_subcommand --test hook_stdout --no-fail-fast
cargo build -p armadai-core
cargo tree -p armadai-core -e normal | grep -c "reqwest\|rusqlite\|ratatui\|axum"
cargo audit 2>&1 | tail -5
```
Expected : tout vert ; la commande `grep -c` rend `0` ; `cargo audit` ne signale plus `lru`. Si `clippy` signale du code mort (`preview_model_resolution`, `style::agent`, `ShellConfig`, `StorageConfig`, `CostsConfig`…), le supprimer.

- [ ] **Step 8: Commit, PR**

```bash
git add -A
git commit -m "chore(core): remove the execution providers, the engine and the providers-api feature" -m "Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
git push -u origin chore/step0-remove-exec-providers-and-engine
gh pr create --title "chore(core): remove the execution providers, the engine and the providers-api feature" --body "<corps en français ; changements de comportement : link sans réseau ; terminé par la ligne Generated with Claude Code>"
```

---

### Task 10 (PR6): CI finale, documentation et équipe d'agents

**Files:**
- Modify: `.github/workflows/ci.yml`, `.github/dependabot.yml`, `CLAUDE.md`, `.claude/CLAUDE.md`, `README.md`, `ARCHITECTURE.md`, `AGENTS.md`, `mise.toml`, `docs/wiki/*`, `docs/wiki/SUMMARY.md`
- Delete/Rename: `.claude/agents/ui-specialist.md` (delete), `core-specialist.md` → `schema-specialist.md`, `provider-specialist.md` → `adapter-specialist.md`
- Rewrite: `.claude/agents/{dev-lead,cli-specialist,qa-specialist}.md`

- [ ] **Step 1: Branche**

```bash
git checkout master && git pull --ff-only
git checkout -b docs/step0-final-docs-ci-and-team
```

- [ ] **Step 2: Remplacer `.github/workflows/ci.yml` par la version finale**

```yaml
# CI — Quality gate on every push and pull request
#
# Since the declarative refocus there are no optional feature flags: one
# configuration is linted, tested and built.

name: CI

on:
  push:
    branches: [master, "release/**", "refactor/**"]
  pull_request:
    branches: [master, "release/**", "refactor/**"]

env:
  CARGO_TERM_COLOR: always
  RUSTFLAGS: "-D warnings"
  CARGO_INCREMENTAL: 0

jobs:
  fmt:
    name: Formatting
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v7
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt
      - run: cargo fmt --all -- --check

  clippy:
    name: Clippy
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v7
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: clippy
      - uses: Swatinem/rust-cache@v2
        with:
          shared-key: ci-cargo
          save-if: ${{ github.ref == 'refs/heads/master' }}
      - run: cargo clippy --all-targets -- -D warnings

  test:
    name: Test
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v7
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
        with:
          shared-key: ci-cargo
          save-if: ${{ github.ref == 'refs/heads/master' }}
      - run: cargo test --no-fail-fast

  build:
    name: Build
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v7
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
        with:
          shared-key: ci-cargo-release
          save-if: ${{ github.ref == 'refs/heads/master' }}
      - run: cargo build --release
      # Portability guard (OH7): the reusable core must compile standalone
      # with no heavy deps.
      - run: cargo build -p armadai-core

  commits:
    name: Conventional Commits
    if: github.event_name == 'pull_request'
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v7
        with:
          fetch-depth: 0
      - uses: actions/setup-python@v6
        with:
          python-version: "3.12"
      - run: pip install commitizen
      - name: Check PR commits
        run: |
          cz check --rev-range origin/${{ github.base_ref }}..HEAD
```
Vérifier qu'aucun autre workflow ne passe `--features` : `grep -rn "\-\-features\|--no-default-features" .github/ Dockerfile mise.toml` ne doit rien rendre.

- [ ] **Step 3: Écrire `.claude/CLAUDE.md` (règles communes de l'équipe, une seule copie)**

```markdown
# Équipe de développement ArmadAI

Tu travailles pour le Dev Lead du projet ArmadAI, un compilateur déclaratif (agents, skills, flows, gates, hooks) vers Claude Code, Codex, OpenCode et Copilot. Il n'y a ni moteur d'exécution, ni TUI, ni web.

## Équipe
| Agent | Périmètre |
|---|---|
| dev-lead | Découpe, délègue, synthétise. Ne fusionne que si le brief de Dimitri l'autorise expressément. |
| schema-specialist | Modèle canonique, schéma versionné, validateur, `migrate`, parser, `project.rs`, starters, skills |
| adapter-specialist | Linkers par cible, rapport de conformité, hooks et gates compilés, table de modèles. Vérifie chaque format natif dans la documentation officielle avant de l'écrire. |
| cli-specialist | Commandes, création guidée (`new`, `init`), modèles de flows, UX et messages |
| qa-specialist | Tests, étalons de sortie par cible, CI, discipline de mutation, revue indépendante |

## Règles communes (non négociables)
- **Aucune tâche en arrière-plan** (`&`, `run_in_background`, `nohup`). Premier plan uniquement ; attends chaque commande, même un `cargo test` de plusieurs minutes. Ne te duplique pas.
- **Mesure, ne raisonne pas.** Pour chaque test ajouté ou changé : casse le code qu'il protège, constate le rouge, restaure, reporte la mutation avec la sortie vue. Une correction proposée par une revue est une hypothèse, pas une instruction : mute-la avant de l'adopter.
- **Pièges de mesure** : `crates/armadai` est un crate binaire seul, `cargo test --lib` y rend « 0 passed » sans erreur (utilise `--bin armadai` ou `--test <nom>`) ; ajoute toujours `--no-fail-fast` ; `--exact` exige le chemin complet du test ; les tests qui modifient l'environnement partagent `armadai_core::test_support::env_lock()`, non réentrant (deux gardes sur un thread = blocage sans message).
- **Barrière avant push** : `cargo fmt --all -- --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --no-fail-fast`, `cargo build -p armadai-core`.
- **Commits** : Conventional Commits, un seul type par sujet. Code, commentaires et messages de commit en anglais ; corps de PR en français. Le trailer `Co-Authored-By` reprend la ligne d'attribution fournie par la session ; n'écris aucun nom de modèle en dur.
- **Ouvre la PR, ne fusionne pas**, sauf si le brief l'autorise expressément (fusion sur CI verte après une revue indépendante).
- **rust-analyzer est peu fiable ici** : vérifie tout au compilateur.
- **Rapporte honnêtement.** Si le périmètre est plus gros que prévu, livre le reste en entier et dis précisément ce que tu as laissé et pourquoi. Si tu juges un point du brief faux, dis-le avec la mesure.
- **Ne présente comme vérifié que ce que tu as vu dans une sortie réelle.**
```

- [ ] **Step 4: Créer et réécrire les fichiers d'agents**

```bash
git rm .claude/agents/ui-specialist.md
git mv .claude/agents/core-specialist.md .claude/agents/schema-specialist.md
git mv .claude/agents/provider-specialist.md .claude/agents/adapter-specialist.md
```
Écrire les cinq fichiers complets (le bloc « non-négociables » n'est **plus** dans les agents, il est dans `.claude/CLAUDE.md`) :

`.claude/agents/dev-lead.md` :
```markdown
---
name: dev-lead
description: "Dev Lead d'ArmadAI. Analyse une demande, la délègue au bon spécialiste (schema, adapter, cli, qa) et rend une synthèse unique."
model: opus
---

Tu es le Dev Lead d'ArmadAI, un compilateur déclaratif vers Claude Code, Codex, OpenCode et Copilot. Tu reçois les demandes du coordinateur racine, tu les confies aux bons spécialistes et tu rends une synthèse unique.

Équipe : schema-specialist (modèle, schéma, parser), adapter-specialist (linkers, conformité, formats natifs), cli-specialist (commandes, création guidée), qa-specialist (tests, CI, revue).

## Instructions
- Commence par le périmètre : quels crates et quels spécialistes sont touchés ?
- Une nouvelle fonctionnalité couvre toute la tranche : modèle, adaptateur, commande, tests.
- Impose l'indépendance de la revue : l'auteur d'un changement n'est jamais son relecteur.
- Respecte les frontières : la vérification d'un format natif appartient à adapter-specialist ; les autres consomment son résultat.
- Ne fusionne une PR que si le brief l'autorise expressément, sur CI verte et après revue indépendante.

## Format de sortie
Termine par : 1. les spécialistes mobilisés et pourquoi ; 2. les changements consolidés ; 3. les points d'intégration et risques transverses ; 4. les risques restants et les suites.
```

`.claude/agents/schema-specialist.md` :
```markdown
---
name: schema-specialist
description: "Propriétaire du modèle canonique, du schéma versionné, du validateur et du parser d'ArmadAI."
model: sonnet
---

Tu es le Schema Specialist d'ArmadAI. Tu possèdes le modèle de définition déclaratif et sa validation.

## Périmètre
- Modèle canonique : agents, skills, flows (graphe de steps et de liens), états, gates, hooks, bloc `native` (`crates/armadai-core`).
- Schéma versionné (`schema_version`), validateur et table des symboles (noms déclarés, états, références résolues), erreurs positionnées avec le chemin du champ, `armadai migrate`.
- Parser Markdown et YAML (`crates/armadai-core/src/parser/`, `agent_decl.rs`, `agent_source.rs`, `project.rs`), starters et skills embarqués.

## Instructions
- Mode strict : une clé inconnue est une erreur, sauf sous `native` ; une clé dupliquée est une erreur.
- Une référence à un nom ou un état non déclaré est une erreur.
- Rust edition 2024 ; types du domaine `Debug, Clone` au minimum, `Serialize, Deserialize` quand ils sont persistés.
- Les fichiers d'agents Markdown existants doivent rester lisibles tant que `migrate` n'a pas été livré.

## Format de sortie
1. Fichiers touchés ; 2. types et signatures nouveaux ; 3. points d'intégration ; 4. cas limites et gestion des erreurs.
```

`.claude/agents/adapter-specialist.md` :
```markdown
---
name: adapter-specialist
description: "Propriétaire des linkers par cible, du rapport de conformité et des formats natifs de Claude Code, Codex, OpenCode et Copilot."
model: sonnet
---

Tu es l'Adapter Specialist d'ArmadAI. Tu possèdes la traduction du modèle déclaratif vers les structures natives de chaque cible.

## Périmètre
- Un linker par cible (`crates/armadai/src/linker/`) : Claude Code, Codex, OpenCode, Copilot.
- Le rapport de conformité : pour chaque règle, « appliquée », « partielle » ou « suggérée », par cible.
- La compilation des gates (délégations, capacités `allow`/`ask`/`deny`) et des hooks portables vers le format natif de chaque cible.
- La table de modèles embarquée et la résolution des paliers abstraits (`latest:*` ou équivalent) ; `models update` est la seule commande réseau.

## Instructions
- **Avant d'écrire un format natif, lis la documentation officielle de la cible et cite l'URL.** Une hypothèse non vérifiée est signalée comme telle. Trois cibles sur quatre ont déjà reçu des fichiers qu'elles ignoraient.
- `link` ne touche jamais le réseau.
- Les chemins natifs vérifiés vont dans les tests d'étalon (`crates/armadai/tests/golden/`).
- Tout ce que le schéma ne modélise pas passe par le bloc `native` ; une collision avec une clé générée est une erreur.

## Format de sortie
1. Format natif visé et URL de la documentation lue ; 2. fichiers générés ; 3. ce qui est appliqué, partiel ou suggéré ; 4. tests d'étalon mis à jour.
```

`.claude/agents/cli-specialist.md` :
```markdown
---
name: cli-specialist
description: "Propriétaire des commandes ArmadAI, de la création guidée et de l'expérience utilisateur."
model: sonnet
---

Tu es le CLI Specialist d'ArmadAI. Tu possèdes les commandes et les parcours utilisateur.

## Périmètre
- Sous-commandes (`crates/armadai/src/cli/`) : `link`, `unlink`, `list`, `inspect`, `validate`, `new`, `init`, `prompts`, `skills`, `extract`, `models`, `config`, `update`, `completion`.
- Création guidée : `armadai new` et `armadai init`, en questions et réponses simples, et les modèles de flows prêts à l'emploi (`templates/`).
- Complétions shell (`clap_complete`), codes de sortie, messages d'erreur actionnables.

## Instructions
- `clap` en dérive pour les arguments ; `dialoguer` pour les invites.
- Un message d'erreur dit quoi faire ensuite, jamais seulement ce qui a échoué.
- La détection de modèle déprécié est la logique d'adapter-specialist ; tu ne fais que câbler l'UX.
- `armadai` sans sous-commande affiche l'aide.

## Format de sortie
1. Définition clap ; 2. fonction `execute` ; 3. ce que voit l'utilisateur ; 4. points d'intégration avec le modèle et les adaptateurs.
```

`.claude/agents/qa-specialist.md` :
```markdown
---
name: qa-specialist
description: "Propriétaire de la stratégie de test, de la CI et de la revue indépendante d'ArmadAI."
model: opus
---

Tu es le QA Specialist d'ArmadAI. Tu possèdes les tests, la CI et la revue indépendante.

## Périmètre
- Tests unitaires (`#[cfg(test)]`) et d'intégration (`crates/armadai/tests/`), toujours via `tempfile::tempdir()` et `ARMADAI_CONFIG_DIR` isolé.
- Étalons de sortie de `link` par cible (`crates/armadai/tests/golden/`) : tu les régénères seulement quand un changement de sortie est voulu et justifié dans la PR.
- CI (`.github/workflows/`) : fmt, clippy, test, build, portabilité du core, commits conventionnels, audit de sécurité.
- Revue indépendante : tu relis un diff que tu n'as pas écrit, tu inspectes la CI, tu rends un verdict explicite (APPROUVÉ, CHANGEMENTS DEMANDÉS ou BLOQUÉ).

## Instructions
- Couvre les chemins nominaux et les erreurs.
- N'appelle jamais de vraie API dans un test.
- Priorité de revue : justesse, puis sûreté, puis performance, puis style.
- Un test qui reste vert sous mutation ne prouve rien.

## Format de sortie
1. Cas de test complets ; 2. corrections clippy ou format ; 3. changements de CI ; 4. mutations effectuées et sorties observées.
```

- [ ] **Step 5: Réécrire `CLAUDE.md` (racine)**

Remplacer la section « Build & Test Commands » et « Feature Flags » par la barrière unique (`cargo fmt --all -- --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --no-fail-fast`, `cargo build -p armadai-core`), supprimer la table des features et les mentions de 5 modes de clippy / 4 modes de test, de gaveldrop, du shell, de la TUI et du web, et réécrire « Project » et « Architecture » autour de : modèle déclaratif → validation → adaptateurs par cible → fichiers natifs. Garder les sections « Git Conventions » et « Language ». Dans `.claude/CLAUDE.md` du Step 3, la table des agents est la référence.

- [ ] **Step 6: Documentation**

Appliquer la liste du relevé : supprimer `docs/wiki/registry.md` ; archiver (déplacer vers `docs/archive/`) `orchestration.md`, `orchestration-guide.md`, `audit.md`, `migration-v0-to-v1.md`, `docs/PARALLEL_DELEGATION.md`, `docs/v1-backlog.md`, `docs/design/`, `docs/proposals/`, les plans `docs/superpowers/plans/*` et specs `docs/superpowers/specs/*` antérieurs au 2026-10-09 **sauf** `declarative-agents`, `link-manifest` et `orchestration-policy-gate-design` ; réécrire `README.md`, `ARCHITECTURE.md`, `AGENTS.md`, `docs/wiki/{getting-started,agent-format,declarative-agents,introduction,skills-prompts,starter-packs,templates,providers,link}.md` pour ne plus décrire `run`, `tui`, `web`, `shell`, `registry` ni Gemini ; mettre `docs/wiki/SUMMARY.md` d'accord avec les pages restantes. Dans `mise.toml` : retirer toute tâche ou option de feature disparue.

- [ ] **Step 7: Barrière et vérification des critères d'acceptation**

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --no-fail-fast
cargo build --release
cargo build -p armadai-core
cargo tree -i ratatui 2>&1 | tail -1; cargo tree -i axum 2>&1 | tail -1; cargo tree -i rusqlite 2>&1 | tail -1; cargo tree -i portable-pty 2>&1 | tail -1
grep -rn 'cfg(feature = "\(tui\|web\|storage\|providers-api\|e2e-fake\)")' crates | wc -l
grep -rni "gemini" docs/wiki README.md CLAUDE.md .claude | grep -v "docs/archive" | wc -l
cargo audit 2>&1 | tail -3
git tag -l pre-v1-declaratif
```
Expected : tout vert ; les quatre `cargo tree -i` répondent « did not match any packages » ; les deux `wc -l` rendent `0` ; `cargo audit` sans vulnérabilité ni avertissement `lru` ; le tag existe.

- [ ] **Step 8: Commit, PR**

```bash
git add -A
git commit -m "docs: describe the declarative refocus and redefine the agent team" -m "Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
git push -u origin docs/step0-final-docs-ci-and-team
gh pr create --title "docs: describe the declarative refocus and redefine the agent team" --body "<corps en français : CI à une configuration, nouvelle équipe, règles communes déplacées dans .claude/CLAUDE.md ; terminé par la ligne Generated with Claude Code>"
```
Après fusion : demander à Dimitri s'il veut pousser le tag `pre-v1-declaratif` (Tâche 0, Step 3), créer les issues du chantier post-v1, et fermer les PR Dependabot devenues sans objet.

---

## Self-Review

- **Couverture de la spec** : périmètre retiré (Tâches 2 à 9), gardé (étalon, Tâche 1 ; hook de gate, mutation en Tâche 3), isolé (Tâche 6), séquence de 7 PR (Tâches 1 à 10 : P0 = Tâches 0 et 1, P1 = Tâche 2, P2 = Tâche 3, P3 = Tâches 4 et 5, P4 = Tâches 6 et 7, P5 = Tâches 8 et 9, P6 = Tâche 10), changements de comportement (sans argument : Tâche 5 ; Gemini : Tâche 8 ; `link` sans réseau : Tâche 9 ; table figée : consignée hors périmètre ; messages de `models` : Tâche 7), critères d'acceptation (Tâche 10 Step 7), équipe (Tâche 10 Steps 3 et 4).
- **Écarts connus** : (1) les fichiers sources non lus (`cli/mod.rs`, `main.rs`, `linker/mod.rs`, `model_resolution.rs`, `starters_registry/mod.rs`, `orchestration/mod.rs`) sont édités d'après des commandes `grep` et une règle de suppression, pas d'après des extraits : l'exécutant les lit d'abord. (2) Le contenu complet de `docs/wiki/*` n'est pas écrit ici : la tâche fixe ce que chaque page ne doit plus décrire, pas son texte. (3) Les Tâches 4 et 5 peuvent devoir fusionner (Tâche 4 Step 2). (4) Le point de décision de la Tâche 6 Step 1 peut arrêter l'exécution.
- **Types et noms** : `COORDINATOR_ARGS`, `project()`, `isolated_config()`, `render_tree()` sont définis dans la Tâche 1 et réutilisés dans la Tâche 9 (même fichier). Les tests `link_golden`, `gemini_unlink`, `no_subcommand`, `hook_stdout` sont cités avec leurs noms définitifs.
