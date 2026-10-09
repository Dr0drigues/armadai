# Étape 0 — Retrait des surfaces d'exécution et redéfinition de l'équipe

Date : 2026-10-09 · Statut : brouillon soumis à relecture · Étape précédente : étape -1 (CI d'audit réparée, Dependabot, terminée le 2026-10-08)

## 1. Contexte et objectif

ArmadAI se recentre : une couche **déclarative** (agents, skills, flows en graphe, gates, hooks) qui se **compile** vers Claude Code, Codex, OpenCode et Copilot via `armadai link`. L'exécution (`armadai run`, orchestration, TUI, web, shell) n'est plus un objectif de la v1. Les décisions de conception sont consignées dans la note de mémoire `project_cap_v1_declaratif_2026-10-08`.

L'étape 0 **ne crée aucune fonctionnalité**. Elle réduit le dépôt (environ 96 000 lignes, dont environ la moitié pour les surfaces retirées) pour que le remplacement du modèle (étape 1) et la bascule de `link` (étape 2) se fassent sur un code petit, sans code mort conservé.

**Principe** : le comportement des commandes conservées reste identique, sauf les changements listés en section 4. Le code retiré reste récupérable par le tag Git `pre-v1-declaratif`.

## 2. Périmètre

Les chiffres et les dépendances viennent du relevé en lecture seule du 2026-10-09. `[L]` = lu dans le code, `[D]` = déduit.

### 2.1 Retiré (tag `pre-v1-declaratif`)

| Zone | Contenu retiré |
|---|---|
| Exécution | `cli/run.rs` (5 224 lignes), `run_es_record`, `run_replay`, `history`, `costs`, `projections`, `up`/`down`, `db.rs`, `es_log.rs` |
| Moteur | `armadai-core/src/orchestration/` sauf `mod.rs` (scindé) et `policy.rs` ; `events.rs` ; `provider.rs` |
| Interfaces | `shell/` (10 995 lignes, wizard compris), `tui/`, `theme.rs`, `web/` et le front `web/ui` avec son `dist/` commité, `build.rs` du binaire |
| Observabilité | `cli/watch.rs`, `claude_adapter/{mapper,transcript,session_index,mod}`, plugin `assets/claude-plugin`, commande cachée `__claude-register-session` |
| Registres | `cli/registry.rs` (après extraction de `effective_starter_sources`), `src/registry/` |
| Linker Gemini | `linker/gemini.rs`, `LinkTarget::Gemini`, `gemini_cli_e2e.sh` |
| Crates | `armadai-storage`, `armadai-fake` ; dans `armadai-providers` : `api/`, `cli.rs`, `json_runner`, `proxy`, `rate_limiter`, `create_provider` |
| Tests | `gaveldrop.rs`, `tests/cases/`, `gaveldrop.yaml`, `run_*`, `budget_usage_warning`, `pipe_declarative_agents` |
| Infra | `docker-compose.yml`, `Dockerfile` (à réécrire sans features), tâche `mise` `web`, features `tui`/`web`/`storage`/`providers-api`/`e2e-fake` |

### 2.2 Gardé, inchangé ou presque

`link`, `unlink`, `list`, `inspect`, `validate`, `new`, `init`, `prompts`, `skills`, `extract`, `models`, `config`, `update`, `completion`, `skills_registry`, `starters_registry`, `armadai-secrets`, `templates/`, les linkers Claude / Codex / OpenCode / Copilot et `manifest`, `parser/`, `agent.rs`, `agent_source.rs`, `agent_decl.rs`, `project.rs`, `model_resolution`, `model_updater`, `pack_validation`, `test_support.rs`.

**Gardé à l'étape 0 : le hook `__claude-policy-gate`** (`claude_adapter/policy_gate.rs`, 388 lignes). Il ne dépend que de `orchestration::policy` et de `project::find_project_config_from` [L]. Il sera réécrit sur les liens du graphe à l'étape 2.

**Types d'orchestration conservés** (exigés par le parser, `inspect`, `pack_validation` et le hook) [L] : `OrchestrationPattern`, `OrchestrationConfig`, `TeamConfig`, `NestedPattern`, `TriggerConfig`, `AgentRingConfig`, `PipelineConfig`, `validate_config`, `policy.rs`. Les champs `orchestration`, `triggers` et `ring_config` de `AgentMetadata` **ne sont pas retirés** à l'étape 0 : des constructeurs littéraux les utilisent à plusieurs endroits [L]. Ils disparaîtront avec l'ancien modèle à l'étape 2.

### 2.3 Isolé (`audit`)

`cli/audit.rs` (760 lignes), `src/audit/` (11 904 lignes) et `tests/audit_*` (1 778 lignes) quittent le binaire. Ils forment un crate gelé, hors des membres du workspace par défaut et hors de la CI.

**Risque connu (voir R3)** : `audit` dépend de `cli::style` (27 références), de `linker::{slugify, LinkAgent, Linker, model_resolution}`, de `claude_adapter::transcript`, de `armadai_providers::{model_registry::fetch, factory}` et de `core::provider` [L]. Pour qu'il compile seul, il lui faut des copies figées de tout cela. C'est plus que « quelques types ». La décision du 2026-10-09 (crate isolé avec ses copies) est conservée ; si le coût de ces copies s'avère trop élevé, le repli est un dossier source non compilé accompagné d'un README et du tag, sous réserve d'accord explicite.

## 3. Séquence de livraison

Chaque PR est un squash merge sur `master`, avec la CI verte et une revue indépendante. À chaque PR le code compile, les tests restants passent et la CI correspond aux features qui existent encore.

| PR | Contenu | Pourquoi dans cet ordre |
|---|---|---|
| **P0** | Tag `pre-v1-declaratif` ; test d'étalon de la sortie de `link` pour les 4 cibles conservées sur un projet témoin (voir 5) | Filet de sécurité avant toute suppression |
| **P1** | Tests gaveldrop, feature `e2e-fake`, crate `armadai-fake`, tests `run_*` et le test de `prompt_section_boundaries` qui lance `run`, jobs et étape d'upload de la CI correspondants | Feuilles pures : rien n'en dépend |
| **P2** | Web (`src/web`, `web/ui`, `build.rs`, feature `web`, `axum`/`tower-http`) ; `watch`, `claude_adapter` hors `policy_gate`, plugin, `register-session` ; adaptation de `hook_stdout.rs` | `web` dépend de `shell`, pas l'inverse ; `watch` dépend du shell |
| **P3** | `run`, `history`, `costs`, `projections`, `up`/`down`, `db.rs`, `es_log.rs`, crate `armadai-storage`, feature `storage`, `Dockerfile`, `docker-compose.yml` ; puis `shell/`, `tui/`, `theme.rs`, features `tui` | `run` référence le shell : il part en premier. `storage` part avec ses seuls consommateurs |
| **P4** | Isolation d'`audit` (crate gelé) ; extraction de `effective_starter_sources` vers `starters_registry`, puis retrait de `cli/registry.rs` et de `src/registry/` ; correction des messages de `models.rs` | `audit` doit être copié avant de perdre `style`, `linker` et `transcript` |
| **P5** | Linker Gemini et tests associés ; tests `provider_inventories` ; providers d'exécution, feature `providers-api` ; dans le core : `events.rs`, `provider.rs`, `orchestration/` (scission de `mod.rs`, `policy.rs` conservé), `routing.rs` réduit, dépendances `tokio-stream`/`async-trait`/`futures-util` | Dépend de tout ce qui consomme ces types |
| **P6** | Documentation, `CLAUDE.md` (racine et `.claude/`), `README`, wiki, `mise.toml`, `SUMMARY.md`, redéfinition de l'équipe (section 6) | Dernière, pour décrire l'état final |

Chaque PR met à jour la CI en même temps que ses suppressions (une feature supprimée avant son `[[test]]`, son `[[bin]]` ou son job donne « package does not have feature »).

## 4. Changements de comportement assumés

1. **`armadai` sans sous-commande** affiche l'aide (`arg_required_else_help`) au lieu de lancer le shell. Choix par défaut, révocable.
2. **Gemini n'est plus une cible** : `link --target gemini` et `unlink --target gemini` ne sont plus possibles. Un projet déjà lié à Gemini est à défaire avant la mise à jour, ou nettoyé à la main. Le chemin d'`unlink` par manifeste est générique, mais je n'ai pas vérifié ce qu'il fait d'un manifeste Gemini : à tester en P5.
3. **`link` ne touche plus jamais le réseau.** La feature `providers-api` disparaît, donc la branche qui interrogeait le catalogue en ligne (`prompt_model_interactive` via `load_models_online`) est retirée. `models update` reste la seule commande réseau, explicite. `reqwest` reste une dépendance directe pour `starters_registry` et `models update`.
4. **Table de modèles de repli figée** : sans rafraîchissement, `link` résout `latest:*` avec `fallback_model_for_tier`, dont le contenu date (par exemple `claude-sonnet-4-5-20250929`) [L]. Le contenu de la table n'est **pas** mis à jour à l'étape 0 ; le suivi est inscrit en section 8 et doit être traité avant l'étape 2.
5. Messages de `models.rs` (« run `armadai run` ») corrigés ; `models list/check --all` ne voit plus que les projets liés.

## 5. Critères d'acceptation

Vérifiables, chacun avec une commande ou une observation :

1. Le tag `pre-v1-declaratif` existe sur le dernier commit d'avant P1.
2. Le test d'étalon de P0 (sortie de `link` pour Claude, Codex, OpenCode, Copilot sur un projet témoin) est **inchangé** après P1 à P6, hors suppression du cas Gemini. Il est écrit avant les suppressions et doit échouer si une sortie change (vérifié par mutation).
3. `cargo fmt -- --check`, `cargo clippy --all-targets -- -D warnings` et `cargo test` passent sans `--features`, ainsi que `cargo build -p armadai-core`.
4. `cargo tree -i ratatui`, `-i axum`, `-i rusqlite` et `-i portable-pty` ne trouvent rien.
5. Aucune occurrence de `cfg(feature = "tui")`, `"web"`, `"storage"`, `"providers-api"` ni `"e2e-fake"` dans l'arbre.
6. La CI passe de 9 combinaisons de features (5 clippy, 4 test) à 1 clippy et 1 test.
7. `cargo audit` ne signale aucune vulnérabilité ; `lru` (via `ratatui`) a disparu du lockfile.
8. Le hook `__claude-policy-gate` passe ses trois tests existants.
9. Aucun avertissement de code mort sous `-D warnings` (le risque R2 est traité en supprimant, pas en ajoutant `allow(dead_code)`).
10. Le wiki, le `README` et les `CLAUDE.md` ne décrivent plus `run`, `tui`, `web`, `shell`, `registry` ni Gemini comme fonctionnalités.

## 6. Équipe d'agents redéfinie

### 6.1 Constats sur l'équipe actuelle
- Modèles figés sur des versions datées : `claude-sonnet-4-5-20250929` pour 4 agents, `claude-haiku-4-5-20251001` pour `qa-specialist` et `ui-specialist`. Le `qa-specialist` signe donc ses commits « Haiku 4.5 ».
- Le bloc « non-négociables » est copié dans chaque fichier et contient des éléments devenus faux (5 modes de features, gaveldrop 13 cas, 83/83).
- Les règles se contredisent : « ouvrir la PR, ne jamais fusionner » face à la fusion demandée à l'étape -1 ; trailer `Claude Opus 5` codé en dur alors qu'aucun agent ne tourne sur ce modèle.
- `ui-specialist` n'a plus de périmètre ; `core-specialist` et `provider-specialist` décrivent le moteur et les providers retirés.

### 6.2 Nouvelle équipe (5 agents)

| Agent | Périmètre | Remplace |
|---|---|---|
| `dev-lead` | Découpe, délégation, synthèse ; ne fusionne que sur autorisation explicite du brief | `dev-lead` |
| `schema-specialist` | Modèle canonique, schéma versionné, validateur et table des symboles, `migrate`, parser, `project.rs`, starters, skills | `core-specialist` |
| `adapter-specialist` | Linkers par cible, rapport de conformité, hooks et gates compilés, table de modèles ; **vérifie chaque format natif dans la documentation officielle avant de l'écrire** (leçon du spike : trois cibles sur quatre recevaient des fichiers ignorés) | `provider-specialist` |
| `cli-specialist` | Commandes, création guidée (`new`, `init`), modèles de flows, UX et messages | `cli-specialist` |
| `qa-specialist` | Tests, étalons de sortie par cible, CI, discipline de mutation, revue indépendante | `qa-specialist` |

`ui-specialist` est supprimé. Le rôle « UI no-code » reviendra avec le chantier post-v1 qui le justifie.

### 6.3 Règles communes
- Le bloc « non-négociables » quitte les fichiers d'agents pour **`.claude/CLAUDE.md`**, chargé par défaut par les sous-agents (sauf `omitClaudeMd`, non utilisé) : une seule copie. Il est réécrit : pas de tâche en arrière-plan ; mesurer par mutation ; une correction de revue est une hypothèse ; pièges de mesure (`--lib` sur le crate binaire, `--no-fail-fast`, `--exact`, verrou d'environnement non réentrant) ; rapporter honnêtement.
- **Barrière avant push** : `fmt`, `clippy -D warnings`, `test` sans features, `cargo build -p armadai-core`. Les mentions de 5 modes, de 4 modes et de gaveldrop disparaissent.
- **Fusion** : un agent ouvre la PR et ne fusionne pas, sauf si le brief de Dimitri l'autorise expressément (fusion sur CI verte après revue indépendante, comme à l'étape -1).
- **Trailer de commit** : l'agent reprend la ligne d'attribution fournie par la session ; aucun nom de modèle n'est codé en dur dans les fichiers d'agents.
- **Langue** : code, commentaires et commits en anglais ; corps de PR en français.
- **Modèles** : alias, jamais de version datée. Par défaut, `qa-specialist` et `dev-lead` en `opus` (revue et coordination), les trois autres en `sonnet`. Choix par défaut, révocable.

## 7. Risques

| # | Risque | Mitigation |
|---|---|---|
| R1 | `cli/registry.rs` est mixte : il contient `effective_starter_sources` (utilisé par `init.rs:102`) et affiche `MODELS_DEV_URL` [L] | Extraire d'abord, supprimer ensuite (P4) |
| R2 | Code mort sous `-D warnings` : tout ce qui n'était utilisé que par les surfaces retirées devient mort (`preview_model_resolution`, `style::agent()`, `ShellConfig`, `StorageConfig`, `CostsConfig`…) | Critère 9 : supprimer, ne pas masquer |
| R3 | Le crate `audit` « gelé » ne compile pas seul sans copies de `style`, `linker`, `transcript`, `factory` et `provider` | Copie figée en P4 ; repli à valider avec Dimitri si le coût est excessif |
| R4 | Features, `[[test]]`, `[[bin]]`, CI, `mise`, `Dockerfile` désynchronisés : build rouge | Chaque PR met la CI à jour avec ses suppressions (section 3) |
| R5 | Comportement silencieux différent : table de repli figée, plus de shell par défaut, plus de Gemini | Section 4 ; étalon de sortie (critère 2) ; note de version |

## 8. Hors périmètre, suivi

- Mise à jour du contenu de la table de modèles de repli (avant l'étape 2).
- Étapes 1 à 4 : modèle et validateur, bascule de `link`, création guidée, Codex / OpenCode / Copilot.
- Post-v1 : reconstruire `audit` multi-cibles ; Workroom et moteur alternatif à OpenCode ; import communautaire (ex-`registry`) ; suivi d'état à l'exécution ; UI no-code ; réécriture sans perte des commentaires.
- Création des issues GitHub correspondantes : **sur accord de Dimitri**.
