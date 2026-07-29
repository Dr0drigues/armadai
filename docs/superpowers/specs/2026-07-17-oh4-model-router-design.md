# OH4 — Routeur dynamique de modèle (`latest:auto`)

> **Statut** : design validé (brainstorm 2026-07-17)
> **Cible** : beta.3 (feature 2/2 ; OH3 headless = spec + plan séparés)
> **Origine** : enseignement OH4 de l'étude OpenHands (RouterLLM), adapté au contexte ArmadAI (pas de multimodal)

## 1. Objectif

Choisir le **tier de modèle** (`Fast`/`Pro`/`Max`) à l'exécution selon des règles **statiques et déterministes** (zéro token consommé pour router), au lieu d'un tier figé par agent. Escalade vers un tier plus fort pour les tâches complexes, downgrade pour les tâches simples ou quand le budget se réduit.

## 2. Activation & résolution

- Un agent avec **`model: latest:auto`** déclenche le routeur.
- À l'exécution : `route(...)` → `ModelTier` → `resolve_model_for_tier(provider, tier)` (existant) → modèle concret envoyé au provider.
- Tout autre `model` (concret, `latest:fast/pro/max`) est **inchangé** → aucune régression.
- `latest:auto` est détecté **explicitement** : il ne mappe pas à un `ModelTier` fixe (contrairement à `latest:fast/pro/max` via `parse_latest_placeholder`), il signale « router ».

## 3. Règles & signaux (heuristiques statiques)

Quatre signaux, composés avec une priorité déterministe :

1. **Longueur de l'input** → tier de base par seuils (défaut : `< 500` chars → Fast ; `< 4000` → Pro ; au-delà → Max).
2. **Mots-clés de complexité** → listes par tier (substring insensible à la casse) ; ex. `refactor|architecture|prove|debug` → Max, `list|format|summarize` → Fast.
3. **Tags de l'agent** (`agent.metadata.tags`) → mapping `tag → tier` ; ex. `critical`/`architecture` → Max, `format`/`lint` → Fast. **Override** : une intention explicite de l'auteur de l'agent, qui prime sur les heuristiques d'input.
4. **Budget restant** → **downgrade override** appliqué en dernier.

### Composition (règle exacte)

1. **Si** au moins un tag de l'agent est mappé dans `routing.tags` : le tier est celui du tag (si plusieurs tags mappés, prendre le **plus élevé** parmi eux). Les signaux longueur et mots-clés sont **ignorés** — le tag est une intention explicite.
2. **Sinon** : `tier = max(tier_longueur, tier_mots-clés)` (`Fast < Pro < Max`) — l'escalade suit le signal le plus exigeant.
3. **Puis, dans tous les cas** : appliquer le budget — si un `token_budget`/`cost_limit` est actif et que la fraction restante est `<= budget_downgrade_ratio`, **capper** le tier retenu vers le bas (par défaut → `Fast`).

Ainsi un tag `format→fast` force bien Fast même sur un input long (économie voulue), tandis qu'un agent sans tag mappé escalade selon longueur/mots-clés. Le budget peut toujours downgrader en dernier ressort. Règle explicable et testable branche par branche.

## 4. Configuration

Défauts embarqués (marche sans configuration). Surcharge dans `armadai.yaml` :

```yaml
routing:
  length_thresholds: { fast_max: 500, pro_max: 4000 }   # en caractères
  keywords:
    max: ["refactor", "architecture", "prove", "debug"]
    fast: ["list", "format", "summarize"]
  tags: { critical: max, architecture: max, format: fast }
  budget_downgrade_ratio: 0.2   # <=20% du budget restant → cap à Fast
```

Toute clé absente retombe sur le défaut embarqué correspondant.

## 5. Architecture

- **`core/routing.rs`** (nouveau module) :
  - `struct RoutingRules` — `Default` (défauts embarqués) + `Deserialize` (depuis `routing:`), champs `Option` retombant sur les défauts.
  - `struct BudgetState { remaining_ratio: f64 }` — état de budget minimal passé au routeur.
  - `fn route(input: &str, agent_tags: &[String], budget: Option<BudgetState>, rules: &RoutingRules) -> ModelTier` — **pure**, déterministe, sans I/O. Réutilise `ModelTier` de `crate::linker::model_resolution`.
- **Intégration** dans `src/cli/run.rs` `run_single_agent` (au calcul du `model`, ~L171-176) : si `agent.metadata.model.as_deref() == Some("latest:auto")` → `route(...)` puis `resolve_model_for_tier(&provider_name, tier)`. Une seule branche ajoutée ; sinon comportement inchangé.
- Chargement des règles : lues depuis la config projet (`ProjectConfig`/`ProjectDefaults`) quand disponible, sinon `RoutingRules::default()`.

## 6. Interaction avec l'existant

- **`model_fallback`** : inchangé — le routeur fixe le tier initial ; la chaîne de fallback reste le filet sur échec (`is_model_not_found`).
- **Orchestration** : chaque agent en `latest:auto` est routé individuellement (le routage vit dans le chemin d'exécution d'un agent, réutilisé par les moteurs).
- **Synergie OH3** : en `--json`, émettre un événement de traçabilité `route` (agent, tier choisi, signal déclencheur). Ajout d'un variant `RunEvent::Route { agent, tier, reason }` (dépend d'OH3 ; si OH3 n'est pas encore mergé, l'émission est gardée derrière la présence du sink).

## 7. Tests

- `route()` — un test par signal : seuils de longueur (Fast/Pro/Max), mot-clé Max, tag Max, budget → downgrade.
- Composition : le tier le plus élevé gagne (longueur Fast + mot-clé Max → Max) ; budget bas cappe (tags Max + budget épuisé → Fast).
- `latest:auto` invoque le routeur ; un `model` concret et `latest:pro` ne sont **pas** touchés (non-régression).
- Désérialisation de `routing:` depuis YAML ; clés absentes → défauts embarqués.

## 8. Hors scope (documenté)

- **Multimodal / bascule image** : ArmadAI ne fait transiter aucune image (`ChatMessage.content: String`) — sans objet tant qu'un support image n'existe pas.
- **Escalade réactive sur qualité** : écartée (double appel + juge = coûteux, à contre-courant de l'optim tokens ; l'escalade sur échec est déjà couverte par `model_fallback`).
- **Classifieur LLM** : écarté au profit des heuristiques statiques.
- **Interrupteur global** de routage : non retenu ; activation opt-in par agent via `latest:auto` uniquement.
