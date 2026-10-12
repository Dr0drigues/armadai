# Étude d'OpenHands — enseignements pour ArmadAI v1

> **Statut** : étude exploratoire (non engageante)
> **Date** : 2026-07-16
> **Sujet** : [OpenHands](https://github.com/OpenHands/OpenHands) (ex-OpenDevin)
> **Objectif** : extraire des enseignements actionnables pour la v1 d'ArmadAI, orchestrateur de flotte d'agents IA en Rust (agents définis en Markdown, exécutés contre n'importe quel provider LLM API ou CLI).
> **Méthode** : recherche multi-sources avec vérification adversariale (24 affirmations confirmées sur 25, sources primaires majoritaires — docs officielles, 2 papiers de l'équipe, Makefile/README du repo).

---

## 1. Synthèse

OpenHands (ex-OpenDevin, org renommée `All-Hands-AI` → `OpenHands` le 20 octobre 2025) est un framework d'agents de code **open-source et model-agnostic**. Son intérêt pour ArmadAI n'est pas ce qu'il fait (agent codeur autonome à la Devin) mais **comment il est architecturé**.

Distinction importante dans les sources :
- **V0** — repo historique `All-Hands-AI/OpenHands`, papier ICLR 2025 ([arXiv 2407.16741](https://arxiv.org/html/2407.16741v3)).
- **V1** — nouveau *Software Agent SDK*, papier [arXiv 2511.03690](https://arxiv.org/html/2511.03690v1) (nov. 2025).

L'event stream et le sandbox Docker sont **communs** aux deux. L'event-sourcing immuable strict, le local/remote transparent, le `RouterLLM` et le CLI 5-modes sont des apports **V1/SDK**.

Trois piliers architecturaux sont directement transposables, et deux traits confirment que le positionnement d'ArmadAI est le bon.

---

## 2. Les 3 piliers architecturaux à retenir

### 2.1. Architecture event-sourced
**Confiance : haute** — 4 affirmations, toutes votées 3-0.

Toutes les interactions sont des **événements immuables** ajoutés à un log append-only (collection chronologique d'actions et d'observations). L'agent est une **fonction pure / stateless** `history → next action`, implémentant un `step`. La **seule composante mutable est `ConversationState`**. La boucle agentique itère `plan → action → observation` jusqu'à complétion ; l'agent par défaut `CodeAct` exécute réellement le code et vérifie les résultats (il ne se contente pas de suggérer des éditions).

Citations primaires (papier SDK) :
- « At V1's core lies an event-sourcing pattern treating all interactions as immutable events appended to a log »
- « Agents are defined as stateless, immutable specifications... that can be serialized and transmitted across process boundaries »
- « components like Agent, Tool, and LLM are immutable... all changing variables live in ConversationState, making it the only stateful component »
- Papier ICLR : « step function which takes the current state as input and generates an appropriate action »

**Pour ArmadAI** : modéliser l'orchestration de flotte comme un event stream sérialisable (actions/observations). Nos agents Markdown = specs immuables ; un state central versionnable. Bénéfices directs : **reprise après crash, audit/replay, transmission inter-process**. C'est la brique la plus intéressante à évaluer pour notre `storage/` (aujourd'hui table `runs` plate) et notre `Coordinator`.

Sources : [SDK](https://arxiv.org/html/2511.03690v1), [ICLR](https://arxiv.org/html/2407.16741v3), [docs events](https://docs.openhands.dev/sdk/arch/events).

### 2.2. Portabilité local/distant transparente
**Confiance : haute** — 3-0.

Le **même code** tourne en `LocalConversation` (boucle complète in-process, sans conteneur ni réseau) ou en `RemoteConversation` (HTTP/WebSocket) de façon transparente selon le type de workspace fourni, via un pattern factory à **API identique**. Migration prototype local → déploiement conteneurisé multi-utilisateurs sans changer le code.

Citation primaire (papier SDK) :
> « When instantiated with a string path or LocalWorkspace, it returns a LocalConversation... When provided a RemoteWorkspace, the same call transparently constructs a RemoteConversation, which serializes the agent configuration and delegates execution to an agent server over HTTP and WebSocket. Both implementations share an identical API allowing seamless migration from local prototyping to containerized multi-user deployments without code changes. »

**Pour ArmadAI** : abstraire l'exécution derrière une interface unique local/distant. Atout majeur pour une flotte qui doit tourner en dev local **et** en CI/cloud.

Source : [SDK](https://arxiv.org/html/2511.03690v1).

### 2.3. Runtime sandbox isolé
**Confiance : haute** — 5 affirmations, toutes 3-0.

Un conteneur Docker **par session/tâche** (image `agent-server` dédiée sur GHCR, ex. `ghcr.io/openhands/agent-server:*-python`), exposant :
- un shell **bash**,
- un noyau **Jupyter/IPython** pour Python interactif,
- un navigateur **Chromium** via BrowserGym/Playwright,
- un accès SSH.

Piloté par le backend via une API REST.

Citation primaire (papier ICLR) :
> « For each task session, OpenHands spins up a securely isolated docker container sandbox, where all the actions from the event stream are executed. »

**Caveat sécurité confirmé** : le montage du socket Docker (`/var/run/docker.sock`) constitue une frontière de confiance qui **affaiblit l'isolation hôte en pratique**. Des alternatives microVM QEMU sont à l'étude chez eux (issue #13203). Les pins de version d'image (`1.28`/`1.29`/`1.30-python`) sont mouvants.

**Pour ArmadAI** : envisager un runtime sandbox **optionnel** (feature-flag, comme nos autres deps lourdes) pour l'exécution d'agents CLI, en documentant clairement le compromis socket Docker. À arbitrer selon les priorités v1.

Sources : [docs runtime](https://docs.openhands.dev/openhands/usage/architecture/runtime), [ICLR](https://arxiv.org/html/2407.16741v3), [SDK](https://arxiv.org/html/2511.03690v1), [deepwiki](https://deepwiki.com/All-Hands-AI/OpenHands/2.2-running-openhands-locally).

---

## 3. Ce qui valide le positionnement ArmadAI

| Trait OpenHands | Confiance | Statut chez ArmadAI |
|---|---|---|
| **Model-agnostic** via LiteLLM (100+ providers : Anthropic, OpenAI, Gemini, DeepSeek, Ollama local, vLLM, Bedrock...), positionné comme différenciateur explicite vs Devin/Claude Code | haute (3 affirmations 3-0) | ✅ **Déjà notre cœur** (agents Markdown × n'importe quel provider API/CLI). Axe à assumer franchement. |
| **RouterLLM** (sous-classe de `LLM` avec `select_llm()`) : modèle différent par requête au sein d'un agent ; `MultimodalRouter` escalade vers un modèle multimodal si image | haute | 🔶 Piste : équivalent « modèle par tâche/étape » — proche de notre `model_resolution` par cible de linker. |
| **Délégation multi-agents** : `AgentDelegateAction` comme action de 1re classe dans l'event stream, `AgentHub` généralistes (`CodeActAgent`) / spécialistes (`BrowsingAgent`...), `DelegateTool` (spawn parallèle) vs `TaskToolSet` (cycle de vie séquentiel) | haute (2 affirmations 3-0) | ✅ C'est **notre valeur centrale** (Dev Lead → spécialistes). Le pattern « délégation = événement » renforce notre délégation hiérarchique. |
| **Cœur réutilisable** : le CLI est bâti sur l'*OpenHands Software Agent SDK* (repo distinct), moteur commun du CLI **et** de OpenHands Cloud — logique d'agent séparée des interfaces | haute (2 affirmations 3-0) | 🔶 Extraire un cœur d'orchestration (lib) distinct des interfaces TUI/Web/CLI. |
| **Binaire natif** | — | ✅ Rust nous donne déjà l'avantage : pas de runtime Python (OpenHands V1 exige Python 3.12+ via `uv`, ou passe par PyInstaller). |

Citation positionnement (papier SDK) : « RouterLLM, a subclass of LLM that enables the agent to use different models for different LLM requests ». Swap via `LLM_MODEL` / `LLM_API_KEY` / `LLM_BASE_URL` (ex. `ollama/qwen2.5-coder:7b`).

Citation délégation (papier ICLR) : « a special action type AgentDelegateAction, which enables an agent to delegate a specific subtask to another agent » ; exemple : le généraliste `CodeActAgent` délègue le web-browsing au spécialiste `BrowsingAgent`.

---

## 4. Modes d'exécution : le modèle multi-mode
**Confiance : haute** — 5 affirmations, toutes 3-0.

OpenHands V1 se distribue comme un **binaire unique** exposant **5 modes** :

| Mode | Invocation | Usage |
|---|---|---|
| TUI terminal interactif | `openhands` | usage local interactif |
| Intégration IDE via **protocole ACP** | `openhands acp` | Zed / VSCode / JetBrains / Toad |
| **Headless CI/automation** | `openhands --headless -t "task"` (ou `-f task.txt`) | CI/CD, scripting, batch. Toujours en *always-approve*. `--json` → streaming **JSONL** événement par événement |
| Web TUI navigateur | `openhands web` (port 12000) | interface web légère |
| Serveur GUI complet | `openhands serve` (Docker, port 3000) | dashboard complet |

(+ **OpenHands Cloud**.)

Distribution : script curl (`curl -fsSL https://install.openhands.dev/install.sh | sh`) ou `uv tool install openhands --python 3.12` (Python 3.12+ requis pour cette voie ; le binaire standalone s'en affranchit — build PyInstaller).

**Pour ArmadAI** : nous avons déjà TUI + Web. Les deux patterns à considérer pour la v1 :
1. un **mode headless CI-first** (sortie JSONL parsable, auto-approve — idéal pour pipelines),
2. plus tard, une **intégration IDE via ACP**.

> **Note** : l'affirmation « 3 modes seulement (terminal/headless/cloud) » d'un blog a été **réfutée** (vote 0-3) au profit des 5 modes documentés.

Sources : [OpenHands-CLI](https://github.com/OpenHands/OpenHands-CLI), [README](https://github.com/OpenHands/OpenHands-CLI/blob/main/README.md), [docs headless](https://docs.openhands.dev/openhands/usage/cli/headless), [docs command-reference](https://docs.openhands.dev/openhands/usage/cli/command-reference).

---

## 5. Exécution locale : natif vs conteneurisé
**Confiance : haute** — 3-0.

En local, OpenHands offre deux modes principaux :
- **`make run`** — backend + frontend nativement sur l'hôte avec hot-reload (nécessite Python 3.12+ / Node 22+ / Poetry / npm).
- **`make docker-run`** — via docker-compose, environnement isolé proche de la production (mount `WORKSPACE_BASE`, `SANDBOX_USER_ID`).
- (+ un 3e target `docker-dev` pour dev-container.)

**Pour ArmadAI** : proposer une distinction claire entre exécution native (dev rapide) et exécution conteneurisée (isolation/prod), documentée dans le workflow.

Sources : [Makefile](https://github.com/All-Hands-AI/OpenHands/blob/main/Makefile), [deepwiki](https://deepwiki.com/All-Hands-AI/OpenHands/2.2-running-openhands-locally).

---

## 6. Questions ouvertes

À creuser si l'on veut s'inspirer sérieusement de l'event-sourcing :

1. **Persistance/replay de l'event log** : format de sérialisation, stockage, reprise après crash — quel schéma réutiliser pour notre state SQLite/event stream ?
2. **Overhead réel de RemoteConversation** (protocole HTTP/WebSocket, sécurité, latence) — transposable à une flotte Rust distribuée ?
3. **Partage de contexte** et **budget de tokens** entre agent parent et enfants (parallèle vs séquentiel).
4. **Maturité/adoption du protocole ACP** (Agent Client Protocol) — vaut-il l'investissement pour exposer nos agents dans Zed/VSCode/JetBrains ?

---

## 7. Réserves méthodologiques

- **Time-sensitivity** : OpenHands évolue vite (renommage org oct. 2025 ; `ghcr.io/all-hands-ai/` → `ghcr.io/openhands/`). Les pins de version d'image agent-server sont mouvants — ne pas s'appuyer sur un tag précis.
- **V0 vs V1** : les deux coexistent dans les sources. L'event stream et le sandbox Docker sont communs ; l'event-sourcing immuable strict, `LocalConversation`/`RemoteConversation`, `RouterLLM` et le CLI 5-modes sont V1/SDK.
- **Qualité des sources** : la majorité des findings s'appuie sur des sources primaires (docs officielles, deux papiers de l'équipe, Makefile/README). Les blogs cités (toolhalla, glukhov) sont de faible qualité mais leurs affirmations ont été re-corroborées par les sources primaires.
- **Positionnement marketing** : « model-agnostic vs Devin/Claude Code » provient d'un tableau comparatif du vendeur — vrai comme énoncé de positionnement, pas comme jugement objectif.
- **Sécurité** : l'isolation Docker est réelle mais le montage du socket `/var/run/docker.sock` affaiblit la garantie d'isolation hôte en pratique.

---

## 8. Recommandation

Le gisement le plus fort est l'**architecture event-sourced** (§2.1) : elle donnerait à ArmadAI reprise/audit/replay « gratuits » et cadrerait proprement la délégation hiérarchique qu'on veut pousser.

- **v1 (candidats concrets)** : event stream dans `core`/`storage` ; portabilité local/distant (§2.2) ; mode headless CI-first à sortie JSONL (§4).
- **Post-v1** : runtime sandbox Docker optionnel (§2.3) ; intégration IDE via ACP (§4).

---

## Annexe — Sources

| # | URL | Qualité |
|---|---|---|
| 1 | https://docs.openhands.dev/openhands/usage/architecture/runtime | primary |
| 2 | https://arxiv.org/html/2511.03690v1 (papier SDK V1) | primary |
| 3 | https://arxiv.org/html/2407.16741v3 (papier ICLR 2025, V0) | primary |
| 4 | https://docs.openhands.dev/openhands/usage/cli/headless | primary |
| 5 | https://docs.openhands.dev/openhands/usage/cli/command-reference | primary |
| 6 | https://docs.openhands.dev/sdk/arch/events | primary |
| 7 | https://docs.openhands.dev/sdk/arch/llm | primary |
| 8 | https://docs.openhands.dev/sdk/guides/llm-routing | primary |
| 9 | https://docs.openhands.dev/sdk/guides/agent-delegation | primary |
| 10 | https://github.com/OpenHands/OpenHands-CLI | primary |
| 11 | https://github.com/OpenHands/OpenHands-CLI/blob/main/README.md | primary |
| 12 | https://github.com/All-Hands-AI/OpenHands/blob/main/Makefile | primary |
| 13 | https://www.openhands.dev/product/cli | primary (marketing) |
| 14 | https://deepwiki.com/All-Hands-AI/OpenHands/2.2-running-openhands-locally | secondary |
| 15 | https://toolhalla.ai/blog/devin-vs-openhands-vs-swe-agent-2026 | blog |
| 16 | https://www.glukhov.org/ai-devtools/openhands/ | blog |
| 17 | https://www.codesota.com/agentic/openhands-vs-swe-agent | blog |
| 18 | https://dev.to/truongpx396/openhands-deep-dive-build-your-own-guide-1al0 | blog |

**Stats de la recherche** : 3 angles · 12 sources récupérées · 59 affirmations extraites · 25 vérifiées · 24 confirmées · 1 réfutée · 92 appels d'agents.
