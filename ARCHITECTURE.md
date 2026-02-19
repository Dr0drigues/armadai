# Architecture - ArmadAI

## Vision

ArmadAI est un orchestrateur d'agents IA en ligne de commande. Il permet de
définir, gérer et exécuter une flotte d'agents spécialisés, chacun configuré via un
simple fichier Markdown. L'outil est agnostique vis-à-vis des modèles (Claude, GPT,
Gemini...) et des modes d'exécution (API HTTP, proxy, CLI tools).

---

## Choix techniques

| Aspect              | Choix                          | Justification                                            |
| ------------------- | ------------------------------ | -------------------------------------------------------- |
| Langage             | Rust                           | Performance, binaire unique, écosystème CLI/TUI mature   |
| Runtime async       | tokio                          | Standard de facto, écosystème massif (reqwest, axum...)   |
| Format agent        | Markdown (agents.md)           | Lisible, versionnable, compatible avec le standard ouvert |
| Orchestration       | Hub & spoke                    | Un coordinateur dispatch aux agents spécialisés          |
| Interface           | CLI + TUI (ratatui)            | CLI scriptable + TUI riche pour le monitoring/interaction |
| Stockage            | SQLite embarqué (rusqlite)     | Zéro config, in-process, léger, fiable                   |
| Secrets             | SOPS + age                     | Chiffrement champ par champ, diff-friendly, moderne      |
| Portabilité         | Docker Compose (optionnel)     | Pour l'infra (SurrealDB serveur, proxy LiteLLM)          |
| Extensibilité       | Fichiers config uniquement     | Un agent = un fichier .md. Simplicité maximale           |

---

## Architecture d'exécution

```
┌─ HOST MACHINE ─────────────────────────────────────────────┐
│                                                             │
│  armadai (binaire natif Rust)                              │
│  ├── CLI (clap)            commandes : run, new, list...   │
│  ├── TUI (ratatui)         monitoring, interaction, logs   │
│  ├── Core                  parsing .md, orchestration      │
│  ├── Providers             API, proxy, CLI tools           │
│  │   ├── ApiProvider  ──── HTTP ──▶  OpenAI / Anthropic / Google
│  │   ├── ProxyProvider ─── HTTP ──▶  LiteLLM / OpenRouter
│  │   └── CliProvider  ──── spawn ─▶  claude / aider / any CLI
│  ├── Storage ──────────────────────▶  SQLite (embarqué)
│  └── Secrets (SOPS+age)                                    │
│                                                             │
│  ┌─ docker-compose (OPTIONNEL) ──────────────────────┐     │
│  │  litellm-proxy   :4000   (proxy unifié)           │     │
│  └───────────────────────────────────────────────────┘     │
└─────────────────────────────────────────────────────────────┘
```

**Le binaire `armadai` tourne toujours nativement sur la machine hôte**, jamais dans Docker.
Cela garantit :
- L'accès direct au terminal (TUI)
- L'accès aux CLI tools installés sur la machine (claude, aider, etc.)
- L'accès au système de fichiers local (projets, repos git)

Docker Compose est **optionnel** et ne sert qu'à l'infrastructure :
- Proxy LiteLLM (normalisation multi-providers)

---

## Modèle d'orchestration : Hub & Spoke

```
                    ┌──────────────┐
           ┌───────│ Coordinator  │───────┐
           │       │  (hub agent) │       │
           │       └──────┬───────┘       │
           ▼              ▼               ▼
    ┌────────────┐ ┌────────────┐ ┌────────────┐
    │  Agent A   │ │  Agent B   │ │  Agent C   │
    │ (reviewer) │ │ (test gen) │ │ (doc gen)  │
    └────────────┘ └────────────┘ └────────────┘
```

Le **Coordinator** :
- Reçoit la tâche utilisateur
- Analyse et décompose en sous-tâches
- Dispatch aux agents spécialisés appropriés
- Agrège les résultats et fournit la réponse finale

Le **Pipeline mode** (complémentaire) :
- Enchaînement séquentiel : output A → input B → input C
- Déclaré dans la config de l'agent ou via CLI

---

## Abstraction des providers

```rust
#[async_trait]
pub trait Provider: Send + Sync {
    /// Envoie un prompt et retourne la réponse complète
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse>;

    /// Envoie un prompt et retourne un stream de tokens
    async fn stream(&self, request: CompletionRequest) -> Result<TokenStream>;

    /// Retourne les métadonnées du provider (nom, modèles supportés, limites)
    fn metadata(&self) -> ProviderMetadata;
}
```

### Trois implémentations

| Type          | Transport       | Exemples                              |
| ------------- | --------------- | ------------------------------------- |
| ApiProvider   | HTTP (reqwest)  | OpenAI, Anthropic, Google AI          |
| ProxyProvider | HTTP (reqwest)  | LiteLLM, OpenRouter                   |
| CliProvider   | Process (spawn) | claude, aider, tout CLI configurable  |

Le `CliProvider` est **générique** : il spawne n'importe quelle commande configurée,
capture stdout/stderr, et gère le streaming via les pipes du process.

---

## Format d'un agent (.md)

Chaque agent est un fichier Markdown dans le dossier `agents/`. Le parser extrait
les sections par convention de nommage des headings.

```markdown
# Code Reviewer

## Metadata
- provider: anthropic
- model: claude-sonnet-4-5-20250929
- temperature: 0.3
- max_tokens: 4096
- tags: [dev, review, quality]
- stacks: [rust, typescript, java, python]
- cost_limit: 0.50

## System Prompt

Tu es un expert en revue de code. Tu analyses le code en profondeur
pour identifier les bugs, failles de sécurité, problèmes de performance
et violations des conventions du projet.

## Instructions

1. Comprendre le contexte du changement
2. Identifier les bugs potentiels et failles de sécurité
3. Évaluer la lisibilité et la maintenabilité
4. Fournir un feedback constructif avec des suggestions concrètes

## Output Format

Revue structurée en sections : bugs, sécurité, performance, style.
Chaque point inclut : sévérité, localisation, suggestion de fix.
```

### Exemple avec CliProvider

```markdown
# Claude Code Agent

## Metadata
- provider: cli
- command: claude
- args: ["-p", "--model", "sonnet", "--output-format", "json"]
- timeout: 300
- tags: [dev, multi-purpose]

## System Prompt

Tu es un assistant de développement polyvalent.

## Instructions

Exécute la tâche demandée en utilisant tes outils disponibles.
```

### Sections reconnues

| Section          | Obligatoire | Description                                            |
| ---------------- | ----------- | ------------------------------------------------------ |
| `# Titre`        | oui         | Nom de l'agent (heading H1)                            |
| `## Metadata`    | oui         | Configuration technique (provider, model, params...)   |
| `## System Prompt` | oui       | Prompt système envoyé au modèle                        |
| `## Instructions` | non        | Étapes / guidelines pour l'agent                       |
| `## Output Format` | non       | Format attendu de la sortie                            |
| `## Pipeline`    | non         | Déclaration de chaînage avec d'autres agents           |
| `## Context`     | non         | Fichiers, URLs ou données à injecter dans le contexte  |

### Champs Metadata reconnus

| Champ          | Type          | Description                                         |
| -------------- | ------------- | --------------------------------------------------- |
| `provider`     | string        | Nom du provider (`anthropic`, `openai`, `google`, `cli`, `proxy`) |
| `model`        | string        | Identifiant du modèle                               |
| `command`      | string        | Commande CLI (si provider=cli)                      |
| `args`         | list[string]  | Arguments CLI (si provider=cli)                     |
| `temperature`  | float         | Température de sampling (0.0 - 2.0)                 |
| `max_tokens`   | int           | Limite de tokens en sortie                          |
| `timeout`      | int           | Timeout en secondes                                 |
| `tags`         | list[string]  | Tags pour filtrage et organisation                  |
| `stacks`       | list[string]  | Stacks techniques supportées                        |
| `cost_limit`   | float         | Limite de coût par exécution (USD)                  |
| `rate_limit`   | string        | Limite de requêtes (ex: "10/min")                   |
| `context_window` | int         | Override de la taille du contexte du modèle         |

---

## Structure du projet

```
armadai/
├── agents/                      # Définitions d'agents
│   └── _coordinator.md          # Agent hub (orchestrateur)
├── starters/                    # Packs d'agents pré-configurés
│   ├── rust-dev/
│   │   ├── pack.yaml
│   │   ├── agents/
│   │   └── prompts/
│   └── fullstack/
│       ├── pack.yaml
│       └── agents/
├── templates/                   # Templates pour scaffolding
│   ├── basic.md
│   ├── dev-review.md
│   ├── dev-test.md
│   └── cli-generic.md
├── config/
│   ├── providers.sops.yaml      # Clés API chiffrées (SOPS + age)
│   ├── providers.yaml           # Config providers (endpoints, modèles disponibles)
│   └── settings.yaml            # Config globale (defaults, rate limits, storage)
├── src/
│   ├── main.rs                  # Point d'entrée, setup tokio
│   ├── cli/                     # Commandes CLI (clap)
│   │   ├── mod.rs
│   │   ├── run.rs               # armadai run <agent> [input]
│   │   ├── new.rs               # armadai new --template <tpl> <name>
│   │   ├── list.rs              # armadai list [--tags ...] [--stack ...]
│   │   ├── inspect.rs           # armadai inspect <agent>
│   │   ├── history.rs           # armadai history [--agent ...] [--replay id]
│   │   ├── init.rs              # armadai init (bootstrap config)
│   │   ├── up.rs                # armadai up (lance docker-compose)
│   │   ├── config.rs            # armadai config (providers, secrets, starters-dir)
│   │   └── validate.rs          # armadai validate [agent] (dry-run)
│   ├── tui/                     # Interface TUI
│   │   ├── mod.rs
│   │   ├── app.rs               # État global de l'app TUI
│   │   ├── views/
│   │   │   ├── dashboard.rs     # Vue principale : liste agents, statuts
│   │   │   ├── execution.rs     # Vue exécution : streaming output
│   │   │   ├── history.rs       # Vue historique : runs passés
│   │   │   └── costs.rs         # Vue coûts : tracking par agent
│   │   └── widgets/
│   │       ├── agent_list.rs
│   │       ├── log_viewer.rs
│   │       └── cost_chart.rs
│   ├── core/                    # Domaine métier
│   │   ├── mod.rs
│   │   ├── agent.rs             # Struct Agent, chargement depuis .md
│   │   ├── config.rs            # Config centralisée, résolution XDG, AppPaths, save_user_config()
│   │   ├── coordinator.rs       # Hub & spoke : décomposition et dispatch
│   │   ├── pipeline.rs          # Mode pipeline : chaînage séquentiel
│   │   ├── task.rs              # Définition d'une tâche + résultat
│   │   ├── context.rs           # Gestion du contexte partagé entre agents
│   │   ├── project.rs          # Config projet (.armadai/config.yaml ou armadai.yaml), résolution 3 niveaux
│   │   ├── embedded.rs         # Versioning des ressources embedded (.armadai-version)
│   │   ├── fleet.rs            # Définitions de flottes, liaison projets-agents
│   │   ├── prompt.rs           # Fragments de prompts composables (YAML frontmatter)
│   │   ├── skill.rs            # Skills (standard SKILL.md)
│   │   └── starter.rs          # Starter packs, all_starters_dirs(), ARMADAI_STARTERS_DIRS
│   ├── parser/                  # Parsing Markdown → Agent
│   │   ├── mod.rs
│   │   ├── markdown.rs          # Parsing headings, sections, metadata
│   │   ├── metadata.rs          # Parsing de la section Metadata (YAML-like)
│   │   └── frontmatter.rs      # Extraction YAML frontmatter générique
│   ├── providers/               # Abstraction LLM
│   │   ├── mod.rs
│   │   ├── traits.rs            # Provider trait + types communs
│   │   ├── api/
│   │   │   ├── mod.rs
│   │   │   ├── openai.rs
│   │   │   ├── anthropic.rs
│   │   │   └── google.rs
│   │   ├── proxy.rs             # LiteLLM / OpenRouter
│   │   └── cli.rs               # CliProvider générique (spawn process)
│   ├── web/                   # Interface Web (Axum)
│   │   ├── mod.rs             # Serveur HTTP, routes, embedded HTML
│   │   ├── api.rs             # Handlers JSON (/api/agents, /api/skills, /api/starters...)
│   │   └── index.html         # SPA embarquée (dashboard web)
│   ├── linker/                # Génération de configs natives
│   │   ├── mod.rs             # Trait Linker + dispatch
│   │   ├── claude.rs          # .claude/agents/*.md
│   │   └── copilot.rs         # .github/agents/*.agent.md
│   ├── registry/              # Intégration awesome-copilot
│   │   ├── mod.rs
│   │   ├── sync.rs            # Clone/pull du repo registry
│   │   ├── cache.rs           # Index JSON, scanning fichiers
│   │   ├── search.rs          # Recherche multi-mots-clés avec scoring
│   │   └── convert.rs         # Conversion Copilot → ArmadAI
│   ├── skills_registry/       # Découverte de skills GitHub
│   │   ├── mod.rs
│   │   ├── sync.rs            # Clone/pull de repos de skills
│   │   ├── cache.rs           # Index JSON des SKILL.md découverts
│   │   └── search.rs          # Recherche multi-mots-clés avec scoring
│   ├── model_registry/        # Catalogue de modèles models.dev
│   │   ├── mod.rs             # Types (ModelEntry, ModelCost, ModelLimits)
│   │   └── fetch.rs           # Fetch HTTP + cache JSON local (24h TTL)
│   ├── storage/                 # Couche persistance (SQLite)
│   │   ├── mod.rs
│   │   ├── schema.rs            # Définition de la table runs
│   │   └── queries.rs           # Requêtes CRUD : historique, coûts, métriques
│   └── secrets/                 # Gestion des secrets
│       ├── mod.rs
│       └── sops.rs              # Déchiffrement SOPS + age
├── Cargo.toml
├── Dockerfile                   # Build du binaire (multi-stage)
├── docker-compose.yml           # Infra optionnelle (SurrealDB, LiteLLM)
├── AGENTS.md                    # Instructions pour les agents IA travaillant sur CE projet
└── ARCHITECTURE.md              # Ce fichier
```

---

## Dépendances Rust (Cargo.toml)

| Crate             | Version | Usage                                    |
| ----------------- | ------- | ---------------------------------------- |
| `tokio`           | 1.x     | Runtime async                            |
| `clap`            | 4.x     | Parsing CLI avec derive                  |
| `ratatui`         | 0.30+   | Framework TUI                            |
| `crossterm`       | 0.29+   | Backend terminal pour ratatui            |
| `reqwest`         | 0.12+   | Client HTTP async (appels API)           |
| `serde`           | 1.x     | Sérialisation/désérialisation            |
| `serde_yaml_ng`   | 0.10+   | Parsing YAML (configs)                   |
| `serde_json`      | 1.x     | Parsing JSON (réponses API)              |
| `rusqlite`        | 0.34+   | Base de données SQLite embarquée (bundled) |
| `pulldown-cmark`  | 0.13+   | Parsing Markdown (fichiers agents)       |
| `tracing`         | 0.1+    | Logging structuré & instrumentation      |
| `tracing-subscriber` | 0.3+ | Collecteur de logs                       |
| `async-trait`     | 0.1+    | Traits async (provider abstraction)      |
| `tokio-stream`    | 0.1+    | Streaming async (réponses LLM)           |
| `anyhow`          | 1.x     | Gestion d'erreurs simplifiée             |
| `thiserror`       | 2.x     | Erreurs typées pour la lib               |
| `chrono`          | 0.4+    | Timestamps (historique, métriques)       |
| `uuid`            | 1.x     | Identifiants uniques (runs, tasks)       |

---

## Commandes CLI

```
armadai run <agent> [input]        # Exécuter un agent avec un input
armadai run --pipe <a> <b> [input] # Pipeline : chaîner des agents
armadai new --template <tpl> <nom> # Créer un agent depuis un template
armadai list [--tags t] [--stack s]# Lister les agents disponibles
armadai inspect <agent>            # Afficher la config parsée d'un agent
armadai validate [agent]           # Dry-run : valider sans appel API
armadai history [--agent a]        # Historique des exécutions
armadai history --replay <id>      # Rejouer une exécution passée
armadai costs [--agent a] [--from] # Consulter les coûts
armadai config providers           # Gérer les providers
armadai config secrets init        # Initialiser SOPS + age
armadai config starters-dir list   # Lister les répertoires starters
armadai config starters-dir add <path>  # Ajouter un répertoire starters
armadai config starters-dir remove <path> # Retirer un répertoire starters
armadai init                       # Initialiser ~/.config/armadai/
armadai init --force               # Écraser les fichiers existants
armadai init --project             # Créer .armadai/config.yaml local
armadai tui                        # Lancer l'interface TUI
armadai up                         # Lancer l'infra Docker (optionnel)
armadai down                       # Arrêter l'infra Docker
armadai fleet create/link/list/show  # Gérer les flottes d'agents
armadai link --target <t>            # Générer configs natives (claude, copilot...)
armadai registry sync/search/list/add # Registre communautaire
armadai prompts list/show            # Fragments de prompts composables
armadai skills list/show             # Skills (standard SKILL.md)
armadai skills sync/search/add/info  # Registre de skills GitHub
armadai init --pack <name>           # Installer un starter pack
armadai update                       # Mise à jour automatique
armadai completion <shell>           # Générer les completions shell
armadai web [--port N]               # Lancer l'interface web
```

---

## Configuration centralisée

Toute la résolution de chemins et la configuration utilisateur passe par `core/config.rs`.

### Répertoire utilisateur

```
~/.config/armadai/
├── config.yaml          # Defaults (provider, model, storage, rate limits, costs, logging)
├── providers.yaml       # Endpoints et modèles disponibles (non sensible)
├── agents/              # Bibliothèque d'agents globale
├── prompts/             # Fragments de prompts composables
├── skills/              # Skills des agents
├── fleets/              # Définitions de flottes
├── registry/            # Cache du registre awesome-copilot
└── skills-registry/     # Cache du registre de skills GitHub
```

### Résolution XDG

1. `$ARMADAI_CONFIG_DIR` (override explicite)
2. `$XDG_CONFIG_HOME/armadai`
3. `$HOME/.config/armadai`

### Résolution des chemins (AppPaths)

Pour `agents/`, `templates/`, `config/` :
1. `.armadai/{type}/` (dossier projet préféré)
2. `{type}/` (répertoire projet-local legacy)
3. Fallback vers le répertoire global `~/.config/armadai/`

### Config projet

Priorité de recherche (walk-up depuis le cwd) :
1. `.armadai/config.yaml` (préféré)
2. `armadai.yaml` / `armadai.yml` (legacy, hint migration affiché)

### Couches de configuration

1. Defaults Rust (impl `Default`)
2. `~/.config/armadai/config.yaml` (désérialisation serde avec `#[serde(default)]`)
3. Variables d'environnement : `ARMADAI_PROVIDER`, `ARMADAI_MODEL`, `ARMADAI_TEMPERATURE`, `ARMADAI_STARTERS_DIRS`

---

## Features V1

| #  | Feature                  | Description                                                          |
| -- | ------------------------ | -------------------------------------------------------------------- |
| 1  | Scaffolding rapide       | `armadai new --template` pour créer des agents en une commande         |
| 3  | Dry-run mode             | Validation de la config agent sans appel API                         |
| 4  | Cost tracking            | Suivi des coûts par agent/exécution dans SurrealDB                   |
| 5  | Streaming TUI            | Affichage temps réel des réponses dans la TUI                        |
| 6  | Rate limiting            | Limites par provider, configurables dans settings.yaml               |
| 7  | Historique & replay      | Enregistrement et rejeu des exécutions passées                       |
| 8  | Agent versioning         | Fichiers .md versionnés avec git, diff-friendly                      |
| 9  | Context windowing        | Gestion intelligente de la taille du contexte selon le modèle        |
| 10 | Pipeline mode            | Chaînage séquentiel d'agents (output A → input B)                    |

### Hors V1 (V2+)

| #  | Feature                  | Description                                                          |
| -- | ------------------------ | -------------------------------------------------------------------- |
| 2  | Model fallback chain     | Basculement automatique sur un modèle alternatif en cas d'échec      |

---

## Stacks techniques supportées (templates initiaux)

- JavaScript / TypeScript
- Java
- sh / zsh
- Rust
- Python

---

## Gestion des secrets

```
config/
├── providers.yaml           # Non sensible (endpoints, noms de modèles)
└── providers.sops.yaml      # Chiffré par SOPS + age
```

Workflow :
1. `armadai config secrets init` → génère une clé age, configure `.sops.yaml`
2. Les clés API sont stockées dans `providers.sops.yaml` (chiffré)
3. Au runtime, `armadai` déchiffre en mémoire via la clé age locale
4. Le fichier chiffré est committé dans git (safe), la clé age ne l'est pas

---

## Stockage (SQLite)

### Mode embarqué (défaut)
- Base locale dans `data/armadai.sqlite` (projet) ou chemin configurable
- Zéro configuration, démarrage instantané via rusqlite (bundled)
- Idéal pour usage single-user

### Schéma principal

```sql
CREATE TABLE IF NOT EXISTS runs (
    id          TEXT PRIMARY KEY,
    agent       TEXT NOT NULL,
    input       TEXT NOT NULL,
    output      TEXT NOT NULL,
    provider    TEXT NOT NULL,
    model       TEXT NOT NULL,
    tokens_in   INTEGER NOT NULL DEFAULT 0,
    tokens_out  INTEGER NOT NULL DEFAULT 0,
    cost        REAL NOT NULL DEFAULT 0.0,
    duration_ms INTEGER NOT NULL DEFAULT 0,
    status      TEXT NOT NULL DEFAULT 'success',
    created_at  TEXT NOT NULL DEFAULT (datetime('now'))
);
```

---

## Docker Compose (optionnel)

```yaml
# Lancé via `armadai up`, arrêté via `armadai down`
services:
  litellm:  # Proxy multi-providers
    image: ghcr.io/berriai/litellm:main-latest
    ports:
      - "4000:4000"
    volumes:
      - ./config/litellm.yaml:/app/config.yaml
```

> **Note :** Le stockage local utilise SQLite embarqué (via rusqlite bundled) — aucun service Docker requis pour la persistance.
