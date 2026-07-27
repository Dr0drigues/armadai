# OH1 parallélisme — Design (#250)

## Contexte

Le moteur d'orchestration event-sourcé exécute les délégations **une par une**.
Le cœur générique `run_event_sourced` (`src/core/orchestration/es/engine.rs`)
tourne ainsi : `decider.decide(&state) -> Vec<Action>`, puis pour **chaque**
`Action` dans l'ordre, `Action::Invoke { agent, input }` déclenche
`append_and_apply(AgentInvoked)` → `effects.run_invoke(...).await` (**séquentiel,
bloquant**) → `append_and_apply(AgentObserved)`. Le hiérarchique
(`es/hierarchical.rs::dispatch_actions`) produit un fan-out sous forme de batch
`[Emit(Delegated A), Invoke A, Emit(Delegated B), Invoke B, …]` via
`plan_from_response`.flat_map(`invoke_actions`) — mais le run_loop les exécute en
série.

**Motivation concrète (constatée cette session)** : chaque tour `claude -p` d'un
spécialiste est une **session agentique complète (~500 s)**. En hiérarchique, un
coordinateur qui délègue à N spécialistes indépendants les exécute en série →
un run réel prend **N × ~500 s**. Le parallélisme rend les délégations
indépendantes concurrentes → temps mur ≈ le plus lent d'un lot, pas la somme.

**Contrainte cardinale** : le log event-sourcé (`execution_events`, `seq`
déterministe) doit rester **rejouable/reprenable** (`--resume`/`--replay`,
livrés OH1 Lot 6). `replay` est un fold pur sur l'ordre **enregistré** ;
`resume_event_sourced` réamorce depuis ce fold. Donc l'ordre **enregistré** des
events doit être **stable et indépendant de l'ordre de complétion**, sinon deux
replays du même run divergent.

Item milestone : **#250** (P0, `epic:core`, `epic-a` maturité du cœur).

## Objectif

Rendre l'exécution de délégations **indépendantes** concurrente **au niveau du
socle** (moteur générique), avec :
- un **ordre enregistré déterministe** (= ordre du `Vec` fourni par le décideur,
  **jamais** l'ordre de complétion) ;
- une **concurrence bornée** (cap par défaut, override config) ;
- une **résilience** : l'échec d'une délégation n'avorte pas le run (collect-and-record).

Chaque pattern **opte** pour le parallélisme en émettant la nouvelle action ; les
patterns séquentiels par nature (ring) et mono-agent (direct) restent inchangés.

## Décisions validées (Dimitri, 2026-07-27)

1. **Généralisation au socle** : nouvelle action `Action::InvokeParallel` dans le
   moteur générique. Tous les patterns y **optent** ; aucun n'est parallélisé de
   force. Lot 1 livre le socle + tests sur mock ; Lot 2 fait opter le hiérarchique.
2. **Concurrence bornée + override** : cap par défaut **4**, surchargé par
   `orchestration.max_concurrency` (`armadai.yaml`). `buffer_unordered(cap)`.
3. **Échec = collect-and-record** : un `run_invoke` qui échoue devient un
   **nouvel event `ExecutionEvent::AgentFailed { agent, error }`** (pas une
   propagation d'`Err` qui avorte le run). Le run continue ; le coordinateur
   synthétise les résultats partiels.

## Architecture

### 1. Nouvelle action `Action::InvokeParallel` (`es/engine.rs`)

```rust
pub enum Action {
    Invoke { agent: String, input: String },
    /// Invoke several agents concurrently. The loop records one
    /// `AgentInvoked` per entry in Vec order, runs the effects concurrently
    /// (bounded by `max_concurrency`), then records each outcome in Vec order
    /// — independent of completion order, so replay/resume stay deterministic.
    InvokeParallel {
        batch: Vec<InvokeSpec>,
        max_concurrency: usize,
    },
    Emit(ExecutionEvent),
    Halt { reason: String },
    Complete { content: String },
}

/// One unit of work inside an `InvokeParallel` batch. Named distinctly from
/// the `Action::Invoke` variant to avoid confusion.
pub struct InvokeSpec { pub agent: String, pub input: String }
```

`Action::Invoke` **reste inchangée** (chemin séquentiel : ring, et tout décideur
qui n'opte pas). C'est le seul ajout à l'enum — pas de changement du trait
`Decider` ni du trait `EffectRunner` (signature `run_invoke` conservée).

### 2. Handler `InvokeParallel` dans `run_loop`

Ordre déterministe garanti par une **séparation stricte record ↔ exécution** :

1. **Append séquentiel des `AgentInvoked`** dans l'ordre du `Vec` (déterministe) :
   pour chaque `InvokeSpec { agent, input }`, `append_and_apply(AgentInvoked { agent, input })`.
   *(Émettre tous les `AgentInvoked` d'abord = tous les agents apparaissent
   « working » en même temps dans la Workroom — voir §6.)*
2. **Exécution concurrente bornée** : snapshot de l'état courant (`&state`
   immuable, cohérent avec la signature `run_invoke(&self, agent, input, &state)`),
   puis
   ```rust
   let outcomes: Vec<anyhow::Result<ExecutionEvent>> =
       futures::stream::iter(batch.iter().enumerate())
           .map(|(i, inv)| async move {
               (i, effects.run_invoke(&inv.agent, &inv.input, &state).await)
           })
           .buffer_unordered(max_concurrency)
           .collect::<Vec<_>>().await;
   ```
   On collecte en conservant l'**indice** pour rétablir l'ordre du `Vec`
   (`buffer_unordered` rend les résultats dans l'ordre de complétion).
3. **Append des outcomes dans l'ordre du `Vec`** (tri par indice) : pour chaque
   entrée, `Ok(event)` → `append_and_apply(event)` (typiquement `AgentObserved`) ;
   `Err(e)` → `append_and_apply(AgentFailed { agent, error: e.to_string() })`.
   **Le run continue** (collect-and-record).

**Invariant de déterminisme** : les events appendés pour un batch sont
exactement `[AgentInvoked×N (ordre Vec)]` puis `[outcome×N (ordre Vec)]`,
totalement indépendants de l'ordre de complétion → `replay` (fold pur) et
`resume` restent déterministes.

**Le cap `max_concurrency` est porté par l'action `InvokeParallel` elle-même**
(pas par la signature du moteur). Choix : le moteur générique
(`run_event_sourced`/`resume_event_sourced`) garde sa signature **inchangée** →
aucun des 8 points d'entrée (direct/blackboard/ring/hierarchical × run+resume)
n'est touché en Lot 1, et un pattern qui n'opte pas ignore totalement le cap.
Le décideur qui émet `InvokeParallel` fixe le cap (le hiérarchique lira
`config.max_concurrency()` en Lot 2, défaut 4). `Action::Invoke` (séquentiel)
n'a pas de cap.

### 3. Nouvel event `ExecutionEvent::AgentFailed { agent, error }`

```rust
/// A delegated invocation failed. Recorded instead of aborting the run
/// (collect-and-record): the coordinator synthesizes partial results.
AgentFailed { agent: String, error: String },
```

**Réducteur (`apply`, `es/state.rs`) — décision structurelle critique** :
`apply(AgentFailed)` **doit pousser un message `assistant`** dans
`state.conversations[agent]`, avec un contenu marqueur, p.ex.
`"[Delegation failed: <error>]"`. Raison : le décideur hiérarchique détecte
qu'un enfant est « en vol / non réglé » via `latest_response(to).is_none()`
(`awaiting_in_flight`, `es/hierarchical.rs:358`) et `is_settled` via
`latest_response(...).is_some_and(is_final_answer)`. Si `AgentFailed` ne
poussait rien, `latest_response(enfant échoué)` resterait `None` → le
coordinateur **attendrait indéfiniment** un enfant qui ne répondra jamais
(run bloqué jusqu'au turn-cap). En poussant le marqueur, l'enfant devient
« réglé » et la synthèse démarre sur les résultats partiels.

- Le marqueur `[Delegation failed: …]` **ne contient aucun marqueur `@agent:`**
  → `is_final_answer` (`es/hierarchical.rs:198`) le lit comme une réponse finale
  ordinaire (pas une nouvelle délégation). À vérifier au test.
- `AgentFailed` **n'incrémente pas** le budget (tokens/coût) — contrairement à
  `AgentObserved`.

**Pont `RunEvent` (`es/bridge.rs`)** : `AgentFailed { agent, error }` →
`RunEvent::AgentEnd { agent, tin: 0, tout: 0, cost: 0.0, content: "[Delegation failed: <error>]" }`
(clôt la tuile « working » de l'agent dans la Workroom). *(Le rendu enrichi des
échecs — badge, couleur — est laissé au rapport de run #279 ; ici on ferme
proprement la tuile.)* La fonction de contenu final du bridge (fallback sur le
dernier `AgentObserved`, `bridge.rs:288`) reste inchangée ; un `AgentFailed`
n'est pas un `Completed`.

### 4. Projections / `--replay`

`replay` folde le log via `apply`. Comme `AgentFailed` est un event enregistré à
part entière avec une règle `apply` déterministe, aucun traitement spécial :
`--replay` reconstruit `conversations` (avec le marqueur d'échec) et le bridge
régénère les `RunEvent` à l'identique. Le contenu final synthétisé par le
coordinateur inclut donc déjà les résultats partiels.

### 5. Opt-in hiérarchique (Lot 2, `es/hierarchical.rs::dispatch_actions`)

Le fan-out actuel `[Emit(Delegated A), Invoke A, Emit(Delegated B), Invoke B, …]`
devient `[Emit(Delegated A), Emit(Delegated B), …, InvokeParallel([A, B, …])]` :
les `Emit(Delegated)` (purs, bookkeeping du `hier.trace`) restent séquentiels et
**précèdent** l'`InvokeParallel` unique portant les N invocations. Aucun autre
changement du décideur : `awaiting_in_flight`/`is_settled`/`synthesis_count`
raisonnent sur `conversations`, alimentées identiquement par le run_loop.
La délégation en profondeur (un enfant qui délègue à son tour) reste un batch
séparé au tour suivant — la concurrence est **intra-lot**, la profondeur reste
sérialisée par les tours du décideur (correct : un enfant ne peut déléguer
qu'après avoir été invoqué).

### 6. Workroom

Déjà compatible : le run_loop émet tous les `AgentInvoked` (→ `RunEvent::AgentStart`)
**avant** les observations → plusieurs agents « working » simultanément.
Les `AgentObserved`/`AgentFailed` arrivent dans l'ordre du `Vec` (→ `AgentEnd`
successifs). Aucun changement Workroom dans ce périmètre.

## Impact par surface

- **`es/engine.rs`** : `Action::InvokeParallel { batch, max_concurrency }` +
  struct `InvokeSpec` ; handler dans `run_loop`. Signatures
  `run_event_sourced`/`resume_event_sourced`/`run_loop` **inchangées** (cap
  porté par l'action). Dép. `futures-util` (`buffer_unordered`) — déjà dans le
  graphe (transitif via reqwest/tokio), à promouvoir en dépendance directe.
- **`es/event.rs`** : variante `AgentFailed { agent, error }`.
- **`es/state.rs`** : bras `apply(AgentFailed)` (push `assistant` + marqueur).
- **`es/bridge.rs`** : bras `AgentFailed` → `RunEvent::AgentEnd`.
- **`es/hierarchical.rs`** (Lot 2) : `dispatch_actions` émet `InvokeParallel`.
- **`core/orchestration/mod.rs`** : champ `max_concurrency: Option<u32>` sur
  `OrchestrationConfig` (défaut 4), suivant le motif des autres limites partagées.
- **Décideur hiérarchique** (Lot 2) : lit `config.max_concurrency()` et le place
  dans l'action `InvokeParallel`. Aucun changement des points d'appel `cli/run.rs`.
- **Ring / direct / blackboard** : **inchangés** (n'émettent pas `InvokeParallel`).
- **`--resume`/`--replay`** : inchangés fonctionnellement (fold pur sur ordre
  enregistré stable).

## Découpage en sous-lots (une PR chacun + revue indé + validation Dimitri)

- **Lot 1 — Socle** : `Action::InvokeParallel { batch, max_concurrency }` +
  struct `InvokeSpec` ; `ExecutionEvent::AgentFailed` (event + `apply` + bridge) ;
  handler `run_loop` (`buffer_unordered`, ordre Vec déterministe,
  collect-and-record) ; champ `OrchestrationConfig::max_concurrency` +
  accesseur `max_concurrency()` (défaut 4). Tests **sur mock `EffectRunner`**
  (aucun pattern ne l'émet encore) : déterminisme d'ordre, cap respecté, échec
  partiel enregistré + run continue.
- **Lot 2 — Opt-in hiérarchique** : `dispatch_actions` émet `InvokeParallel` (cap
  = `config.max_concurrency()`) ; test d'intégration hiérarchique (fan-out
  concurrent, ordre enregistré déterministe, un enfant en échec → synthèse sur
  partiel).

## Hors périmètre

- **Parallélisme blackboard** (agents concurrents intra-round) : opt-in
  ultérieur possible (même socle), non traité ici.
- **Rapport de run consolidé (#279)** : rendu enrichi des échecs (badge/couleur
  Workroom + vue TUI/Web). Ici, `AgentFailed` ferme juste la tuile.
- **Timeout par délégation (#270)** : orthogonal (reset de budget-temps
  par tour). Non traité.
- **Coordination inter-process** du cap : le cap est intra-run/process.

## Tests

**Lot 1 (mock `EffectRunner`)** :
- **Déterminisme d'ordre** : un décideur mock émet `InvokeParallel([A, B, C])`
  avec un runner dont les complétions arrivent dans le désordre (p.ex. C, A, B) ;
  le log enregistré est exactement `AgentInvoked A/B/C` puis `AgentObserved A/B/C`
  (ordre Vec), **indépendant** de l'ordre de complétion. Deux exécutions →
  logs identiques ; `replay` du log → état identique.
- **Cap respecté** : runner instrumenté comptant la concurrence courante (max
  observé) ; `InvokeParallel` de 6 avec `max_concurrency=2` → jamais >2 en vol.
- **Échec partiel (collect-and-record)** : runner qui `Err` sur B ; le log
  contient `AgentObserved A`, `AgentFailed B`, `AgentObserved C` (ordre Vec) ;
  le run **continue** (pas d'`Err` propagée, statut non avorté) ; `apply` a
  poussé un message `assistant` marqueur pour B.
- **Bridge** : `AgentFailed` → un `RunEvent::AgentEnd` avec le contenu marqueur.
- **`is_final_answer`** : le marqueur `[Delegation failed: …]` est traité comme
  réponse finale (pas comme délégation).
- **Non-régression** : `Action::Invoke` séquentielle inchangée ; direct/ring
  compilent et passent.

**Lot 2 (intégration hiérarchique, style e2e déterministe existant)** :
- Fan-out coordinateur→3 spécialistes via `InvokeParallel` ; ordre enregistré
  déterministe ; un spécialiste en échec → le coordinateur synthétise sur les 2
  réussis ; run complété (non bloqué).

## Gate (par lot)

`cargo fmt --all` + clippy **3 modes** (`tui` / `tui,providers-api` /
`tui,web,storage`) `-D warnings` + `cargo test --no-default-features --features tui`
+ `cargo test --no-default-features --features tui,storage` (couvre la cible e2e).
`rust-analyzer` non fiable (diagnostics ABI/stale) → **vérifier au compilateur**.
Conventional Commits, trailer `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`.

## Risques

- **`apply(AgentFailed)` = point de rupture silencieux** : si le marqueur n'est
  pas poussé dans `conversations`, le hiérarchique se bloque (attente d'un enfant
  jamais réglé). C'est **le** invariant à tester explicitement (test « échec
  partiel » ci-dessus). Documenté dans le code au bras `apply`.
- **Ordre enregistré vs complétion** : la tentation d'appender au fil des
  complétions (`buffer_unordered` les livre déséquencées) casserait le
  déterminisme replay. Le handler **doit** ré-ordonner par indice avant
  d'appender. Test de déterminisme = garde-fou.
- **Cap & rate-limit (Lot 1 rate-limit, #269)** : le décorateur rate-limit
  plafonne déjà par provider ; `max_concurrency` est un second plafond
  orthogonal (nombre d'invocations en vol). Les deux se composent (le plus
  strict gagne de facto) — pas de couplage à coder.
- **Dépendance `futures`** : confirmer `buffer_unordered` disponible en
  dépendance directe (sinon l'ajouter à `Cargo.toml`), et le gating features
  (le moteur ES est compilé dans tous les modes CI).
- **`&state` immuable partagé dans `buffer_unordered`** : `run_invoke` prend
  `&ExecutionState` ; le batch lit un snapshot **cohérent** pris après les
  `AgentInvoked` (donc chaque invocation voit son propre `user`-turn, mais **pas**
  les `AgentObserved` de ses pairs du même lot — correct : les délégations d'un
  lot sont indépendantes par construction).
