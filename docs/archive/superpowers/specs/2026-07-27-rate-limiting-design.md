# Rate-limiting des providers API — Design

## Contexte

Le rate-limiting d'ArmadAI est **exposé mais inerte** (issue #265, constats vérifiés au code, tous CONFIRMÉS) :

- `src/providers/rate_limiter.rs` : token-bucket correct en soi, mais `parse_rate("N/hour")` fait une **division entière** (`N.max(1)/60`) → tout `N<60` → `0` → `RateLimiter::new(0)` → `refill_rate=0` → `Duration::from_secs_f64(f64::INFINITY)` → **panique dès le 1ᵉʳ `acquire()`**.
- **No-op** : `RateLimiter::new(rpm)` est reconstruit à chaque appel (`cli/run.rs:906` dans `run_single_agent`, `:1294` dans `run_single_agent_es`) → bucket toujours plein → ne bloque jamais. Et seulement sur ces deux chemins single-agent legacy.
- Les 4 moteurs event-sourcés appellent `provider.complete()` **en direct** (`es/direct.rs:231`, `es/blackboard.rs:672`, `es/ring.rs:853/914`, `es/hierarchical.rs:1314`) → **aucun** limiteur en orchestration.
- `UserConfig.rate_limits` (`core/config.rs`, défauts anthropic:50/openai:60/google:60/proxy:100) = **config morte** (jamais lue).
- `AgentMetadata.rate_limit` (frontmatter `"10/min"`) : parsé, affiché (inspect/TUI/web), mais seulement câblé aux deux chemins cassés ci-dessus.
- `ProjectConfig` (`armadai.yaml`) : **aucune** section rate-limit.
- `factory.rs::create_provider(agent: &Agent) -> Result<Box<dyn Provider>>` renvoie un provider **nu**, aucun décorateur.
- Zéro gestion 429/529 (traité au Lot 2).

Item milestone : **#265** (P1, `epic:core`). Un produit 1.0 ne doit pas exposer un rate-limiting cassé.

## Objectif (Lot 1 = A, throttling proactif)

Rendre le throttling **réel** : un limiteur partagé et durable qui s'applique à **tous** les call-sites (moteurs ES inclus), branché sur les deux knobs déjà exposés (`config.rate_limits` par provider + frontmatter `rate_limit` par agent), sans panique. Le Lot 2 (résilience 429/529) est esquissé en fin de doc mais **hors périmètre** de cette spec.

## Décisions validées (Dimitri, 2026-07-27)

1. **Périmètre** : C (les deux) livré en 2 lots, **A d'abord** (throttling proactif), B ensuite (résilience 429).
2. **Ancrage** : décorateur `Provider` posé dans `factory.rs` → couvre tous les call-sites via le trait, moteurs ES compris. Trait `Provider` **inchangé**.
3. **Deux couches** : limiteur **partagé par clé provider** (= `config.rate_limits[provider]`, cap global du quota) **+** limiteur optionnel **par agent** (frontmatter `rate_limit`). `complete()`/`stream()` awaitent les deux → le plus strict gagne.
4. **Partage** : registre **global au process** (un limiteur par clé provider partagé par tous les agents/tours d'un run). La coordination inter-process = rôle du Lot 2 (429).
5. **Surface config** : `UserConfig.rate_limits` (par provider) + frontmatter (par agent) uniquement. **Pas** de section rate-limit dans `armadai.yaml` (YAGNI Lot 1).
6. **Absence de cap** : si `config.rate_limits` ne contient pas de clé pour un provider **et** pas de frontmatter → **illimité** (aucun limiteur, pas de blocage).

## Architecture

### `RateLimiter` (remise à plat, `src/providers/rate_limiter.rs`)

- Refill stocké en **tokens/seconde `f64`** (fini la division entière). Capacité (burst) = le compte de la fenêtre (ex. `50` pour `50/min`).
- `parse_rate(&str) -> Option<Rate>` (ou `Option<f64>` tokens/sec) : `"N/sec"`→`N`, `"N/min"`→`N/60`, `"N/hour"`→`N/3600` ; unités et alias (`s/sec/second`, `m/min/minute`, `h/hr/hour`) conservés ; entrée invalide → `None`.
- **Garde anti-panique** : un refill ≤ 0 (ou une construction « illimitée ») ⇒ `acquire()` retourne immédiatement, jamais de `Duration::from_secs_f64(inf)`.
- `acquire(&self)` : logique bucket inchangée (refill proportionnel à l'`elapsed`, capé, consommation d'1 token), sur la nouvelle représentation f64.

### `RateLimitedProvider` (nouveau décorateur)

- Enveloppe `Arc<dyn Provider>` (l'inner réel) + jusqu'à 2 `Arc<RateLimiter>` : `provider_limiter: Option<Arc<RateLimiter>>` (partagé, depuis le registre) et `agent_limiter: Option<Arc<RateLimiter>>` (par agent, depuis le frontmatter).
- `complete(req)` / `stream(req)` : `if let Some(l) = &provider_limiter { l.acquire().await }` puis `if let Some(l) = &agent_limiter { l.acquire().await }`, puis délègue à l'inner. `tracing::debug!` quand un `acquire` a réellement attendu. `metadata()` : pass-through.
- (Le Lot 2 ajoutera la boucle 429/retry **dans ce même décorateur**.)

### Registre partagé (par process)

- `static PROVIDER_LIMITERS: OnceLock<Mutex<HashMap<String, Arc<RateLimiter>>>>` dans `rate_limiter.rs` (ou un module dédié). Une fonction `provider_limiter(provider_key: &str, rate: Option<Rate>) -> Option<Arc<RateLimiter>>` : renvoie/crée (memoize) le limiteur pour la clé si un `rate` est fourni, sinon `None`. Clé = nom de provider (`anthropic`/`google`/…).

### Câblage factory (`src/providers/factory.rs`)

- `create_provider(agent)` construit l'inner comme aujourd'hui, puis :
  - clé provider dérivée du provider construit / des métadonnées d'agent ;
  - `provider_rate` = `load_user_config().rate_limits.get(key)` (ranime la config morte) → `provider_limiter(key, rate)` ;
  - `agent_rate` = `agent.metadata.rate_limit` via `parse_rate` → nouveau `RateLimiter` **par agent** (non partagé) ;
  - renvoie `Box::new(RateLimitedProvider::new(inner, provider_limiter, agent_limiter))`.
- **Nettoyage** : supprimer les blocs `RateLimiter::new(...).acquire()` de `cli/run.rs:906-912` et `:1294-1299` (désormais redondants ; le throttling vit dans le décorateur pour tous les chemins).

## Impact par surface

- **Trait `Provider`** : inchangé (le décorateur l'implémente).
- **`core/config.rs`** : aucune nouvelle struct ; on **lit** enfin `rate_limits` (défauts conservés).
- **`armadai.yaml`/`ProjectConfig`** : inchangé.
- **CLI/TUI/Web** : aucune nouvelle UI ; le `rate_limit` d'agent déjà affiché (inspect/TUI/web) devient réel. Throttle silencieux + `tracing::debug` (signal Workroom « en attente de quota » = plus tard).
- **Moteurs ES** : inchangés (héritent du décorateur via la factory).

## Hors périmètre (Lot 2 = B, spec séparée)

- Détection 429/529 dans le décorateur ; lecture `Retry-After` / `anthropic-ratelimit-*` / Google `RESOURCE_EXHAUSTED` ; backoff exponentiel + retry ; option de recalibrage du bucket depuis les en-têtes serveur.
- Signal Workroom « en attente de quota » (`RunEvent` dédié).
- Section rate-limit dans `armadai.yaml`.
- Coordination inter-process (au-delà du best-effort 429 du Lot 2).

## Tests

- `parse_rate` : `"30/hour"` → taux précis non nul (plus de troncature) ; `"1/hour"` idem ; `"60/hour"`, `"10/min"`, `"1/sec"` corrects ; invalide → `None`.
- `RateLimiter` : construit « illimité » / refill 0 → `acquire()` immédiat, **jamais de panique** ; throttle réel (deux `acquire` rapprochés sur un petit taux → le second attend).
- `RateLimitedProvider` (avec un `Provider` factice comptant les appels + une horloge de test si besoin) : deux agents **partageant** la clé provider partagent le limiteur (taux soutenu throttlé) ; le limiteur par-agent resserre ; les deux doivent passer ; provider sans cap ni frontmatter → aucun blocage.
- **Factory** : `create_provider` d'un agent avec `rate_limit` frontmatter et/ou `config.rate_limits[provider]` renvoie un `RateLimitedProvider` avec les bons limiteurs ; sans aucun des deux → provider non throttlé (ou décorateur inerte).
- Non-régression : les chemins ES et single-agent compilent/passent après retrait des blocs manuels.

## Gate

`cargo fmt --all` + clippy 3 modes (`tui` / `tui,providers-api` / `tui,web,storage`) `-D warnings` + `cargo test --no-default-features --features tui` + `cargo test --no-default-features --features tui,storage`. Le rate-limiter/décorateur/factory sont sous `providers-api` (ou toujours compilés selon l'emplacement) — vérifier le gating. Une PR (Lot 1) + revue indé + validation Dimitri.

## Risques

- **Horloge dans les tests** : le token-bucket dépend d'`Instant::now()`. Pour un test déterministe du throttle, soit tolérer une petite attente réelle (`tokio::time`), soit injecter une horloge — décider à l'implémentation (préférer un test robuste non-flaky, pas d'assertion temporelle fragile).
- **Clé provider** : bien dériver une clé stable et cohérente avec les clés de `config.rate_limits` (`anthropic`/`google`/`openai`/`proxy`) depuis le provider/agent — sinon le cap partagé ne s'applique pas. À vérifier au câblage.
- **Gating features** : `RateLimitedProvider` doit compiler dans tous les modes CI (le rate_limiter est déjà non-gated ; garder le décorateur non-gated, seul l'inner API est gated `providers-api`).
