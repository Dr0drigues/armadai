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
| Stockage            | SurrealDB embarqué (défaut)    | Zéro config, in-process, SQL+NoSQL, écrit en Rust        |
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
│  ├── Storage ──────────────────────▶  SurrealDB (embarqué)
│  └── Secrets (SOPS+age)                                    │
│                                                             │
│  ┌─ docker-compose (OPTIONNEL) ──────────────────────┐     │
│  │  surrealdb       :8000   (mode serveur)           │     │
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
- SurrealDB en mode serveur (multi-instance, persistance réseau)
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
│   ├── _coordinator.md          # Agent hub (orchestrateur)
│   └── examples/                # Exemples fournis
│       ├── code-reviewer.md
│       ├── test-writer.md
│       └── doc-generator.md
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
│   │   ├── up.rs                # armadai up (lance docker-compose)
│   │   ├── config.rs            # armadai config (gestion providers/settings)
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
│   │   ├── coordinator.rs       # Hub & spoke : décomposition et dispatch
│   │   ├── pipeline.rs          # Mode pipeline : chaînage séquentiel
│   │   ├── task.rs              # Définition d'une tâche + résultat
│   │   └── context.rs           # Gestion du contexte partagé entre agents
│   ├── parser/                  # Parsing Markdown → Agent
│   │   ├── mod.rs
│   │   ├── markdown.rs          # Parsing headings, sections, metadata
│   │   └── metadata.rs          # Parsing de la section Metadata (YAML-like)
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
│   ├── storage/                 # Couche persistance
│   │   ├── mod.rs
│   │   ├── embedded.rs          # SurrealDB mode embarqué
│   │   ├── client.rs            # SurrealDB mode serveur (Docker)
│   │   ├── schema.rs            # Définition des tables/schémas
│   │   └── queries.rs           # Requêtes : historique, coûts, métriques
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
| `ratatui`         | 0.29+   | Framework TUI                            |
| `crossterm`       | 0.28+   | Backend terminal pour ratatui            |
| `reqwest`         | 0.12+   | Client HTTP async (appels API)           |
| `serde`           | 1.x     | Sérialisation/désérialisation            |
| `serde_yaml`      | 0.9+    | Parsing YAML (configs)                   |
| `serde_json`      | 1.x     | Parsing JSON (réponses API)              |
| `surrealdb`       | 2.x     | Base de données embarquée/client         |
| `pulldown-cmark`  | 0.12+   | Parsing Markdown (fichiers agents)       |
| `age`             | 0.10+   | Chiffrement/déchiffrement (secrets)      |
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
armadai tui                        # Lancer l'interface TUI
armadai up                         # Lancer l'infra Docker (optionnel)
armadai down                       # Arrêter l'infra Docker
```

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

## Stockage (SurrealDB)

### Mode embarqué (défaut)
- Base locale dans `~/.config/armadai/data/` ou `./data/`
- Zéro configuration, démarrage instantané
- Idéal pour usage single-user

### Mode serveur (Docker)
- `armadai up` lance SurrealDB via docker-compose
- Connexion via `ws://localhost:8000`
- Pour usage multi-instance ou persistance réseau

### Schéma principal

```sql
-- Exécutions d'agents
DEFINE TABLE runs SCHEMAFULL;
DEFINE FIELD agent      ON runs TYPE string;
DEFINE FIELD input      ON runs TYPE string;
DEFINE FIELD output     ON runs TYPE string;
DEFINE FIELD provider   ON runs TYPE string;
DEFINE FIELD model      ON runs TYPE string;
DEFINE FIELD tokens_in  ON runs TYPE int;
DEFINE FIELD tokens_out ON runs TYPE int;
DEFINE FIELD cost       ON runs TYPE float;
DEFINE FIELD duration   ON runs TYPE duration;
DEFINE FIELD status     ON runs TYPE string;
DEFINE FIELD created_at ON runs TYPE datetime DEFAULT time::now();

-- Métriques agrégées par agent
DEFINE TABLE agent_stats SCHEMAFULL;
DEFINE FIELD agent       ON agent_stats TYPE string;
DEFINE FIELD total_runs  ON agent_stats TYPE int;
DEFINE FIELD total_cost  ON agent_stats TYPE float;
DEFINE FIELD avg_duration ON agent_stats TYPE duration;
DEFINE FIELD last_run    ON agent_stats TYPE datetime;
```

---

## Docker Compose (optionnel)

```yaml
# Lancé via `armadai up`, arrêté via `armadai down`
services:
  surrealdb:
    image: surrealdb/surrealdb:latest
    command: start --user root --pass root
    ports:
      - "8000:8000"
    volumes:
      - armadai-data:/data

  litellm:  # Optionnel : proxy multi-providers
    image: ghcr.io/berriai/litellm:main-latest
    ports:
      - "4000:4000"
    volumes:
      - ./config/litellm.yaml:/app/config.yaml
    profiles:
      - proxy

volumes:
  armadai-data:
```
