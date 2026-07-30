# Plugin Claude Code → Workroom — P2 : suivi de session complet via SessionEnd — Design

**Date** : 2026-07-29
**Statut** : validé (design), à implémenter
**Cible** : suite de P1 (`armadai watch` + adaptateur transcript, mergé). Prérequis : P1 livré.

## Contexte & re-cadrage

Le test d'intégration live de P1 a montré que `armadai watch` suit une session Claude Code en **quasi-temps-réel** via le tail du transcript JSONL (poll ~200 ms), et que le transcript est la **source canonique** (il contient tout : tool calls, sous-agents, tokens, `stop_reason`). La prémisse d'origine de P2 (« couche hooks temps réel pour réduire la latence ») a donc un **faible ROI** : le tail est déjà fluide et les hooks n'apporteraient aucun **contenu** nouveau.

Décision (Dimitri, 2026-07-29) : **P2 minimal** — un seul hook utile, `SessionEnd`, pour un signal de fin **définitif**, qui permet à `armadai watch` de **suivre la session entière (tous les tours) jusqu'à sa vraie fin**, corrigeant la limite de P1 (finalisation au 1er `end_turn`). Puis prioriser P3 (intégration relais armadai). La couche hooks temps réel complète est **abandonnée** (hors périmètre).

## Problème résolu

En P1, `drive_session` en mode follow finalise (émet `Result`, le run apparaît « complete ») dès le **premier** tour assistant terminal (`stop_reason: end_turn`). Or une session Claude Code est **multi-tours** : l'utilisateur peut enchaîner plusieurs prompts. P1 s'arrête après le tour 1. P2 fait suivre `watch` **jusqu'au vrai `SessionEnd`**.

## Architecture (P2)

```
[Claude Code session]
  │  hook SessionStart (P1)  ──► index (claude-sessions.jsonl)   {session_id, transcript_path, cwd, started_at}
  │  hook SessionEnd  (P2)   ──► armadai __claude-register-session-end
  ▼                                └─► append session_id dans claude-session-ends.jsonl
armadai watch (follow)
  drive_session : tail transcript (tous les tours)
    finalise quand  is_ended(session_id)  OU  idle-abandon (filet)
```

Placement inchangé (cohérent OH7/P1) : l'adaptateur reste **bin-side** (`crates/armadai/src/claude_adapter/`), `armadai-core` **inchangé**, aucune variante `RunEvent` ajoutée.

## Composants

### 1. Plugin — hook `SessionEnd`
`crates/armadai/assets/claude-plugin/armadai-workroom/hooks/hooks.json` gagne une entrée `SessionEnd` (`async: true`, `type: command`) → `armadai __claude-register-session-end`. Un seul hook, symétrique du `SessionStart` de P1.

### 2. Sous-commande cachée `__claude-register-session-end`
Dans `cli/mod.rs` (comme `__claude-register-session`), cachée du `--help`. Lit le payload JSON du hook sur **stdin**, extrait `session_id`, appelle `session_index::mark_ended(session_id)`. **Toujours exit 0** (échec d'écriture → warn silencieux sur stderr), **rien sur stdout**.

### 3. `session_index` — marqueur de fin
- `pub fn ends_path() -> PathBuf` : `ARMADAI_SESSION_ENDS` sinon `<config_dir>/claude-session-ends.jsonl`.
- `pub fn mark_ended(session_id: &str) -> anyhow::Result<()>` : append `{"session_id": "..."}` (ou la ligne brute `session_id`) au fichier ends (création parent si absent).
- `pub fn is_ended(session_id: &str) -> bool` : le `session_id` figure-t-il dans le fichier ends ? (lecture défensive ; `false` si fichier absent/illisible.)

### 4. `drive_session` — suivre jusqu'à SessionEnd (mode follow)
Modifier la logique de finalisation du **mode follow uniquement** :
- Continuer à tailer le transcript **à travers tous les tours** — retirer la finalisation sur `stop_reason` terminal en mode follow (elle causait l'arrêt au tour 1).
- À chaque poll (après lecture des nouvelles lignes complètes) : si `is_ended(session_id)` → `mapper.finish()` + emit + `return Ok(())`.
- Garde-fou **idle-abandon** conservé (filet, si le hook `SessionEnd` n'a pas tiré — vieux plugin, échec — pour ne pas suivre indéfiniment un fichier mort). Seuil large (≈ celui de P1).
- **Mode replay (`follow=false`) : INCHANGÉ** — finalise à l'EOF (une session déjà finie, lue jusqu'au bout, est terminée).
- Effet de bord voulu : ouvrir `watch` (follow) sur une session déjà terminée dont le `SessionEnd` a déjà tiré → `is_ended` vrai dès le 1er poll après EOF → finalisation immédiate.

### 5. Le mapper — inchangé
`Mapper` émet `RunStart` une fois (1er assistant), accumule tokens + délégations sur **tous** les tours, et `finish()` (déclenché par le SessionEnd/le filet) émet `AgentEnd(claude)` + `Result`. Aucune modification : il n'émet jamais `Result` en cours de flux ; suivre plusieurs tours = simplement plus d'entrées poussées avant `finish()`. Entre les tours, `claude` reste actif (working/delegating) côté Workroom, puis `done` à la fin.

## Flux de données

`index (P1) → SessionRef → transcript (tail, tous tours) → RelevantEntry* → mapper → RunEvent* → sink`, avec finalisation pilotée par `is_ended(session_id)` (marqueur SessionEnd) plutôt que par `stop_reason` en mode follow.

## Gestion d'erreurs

- Hook `SessionEnd` / `__claude-register-session-end` : exit 0 toujours ; rien sur stdout ; échec d'écriture du marqueur → warn silencieux (stderr). Ne perturbe jamais Claude Code.
- `is_ended` : fichier ends absent/illisible → `false` (dégradation gracieuse : on retombe sur le filet idle-abandon).
- Payload `SessionEnd` sans `session_id` → no-op (pas d'erreur), comme le register de P1.

## Tests

- `session_index` : `mark_ended` + `is_ended` (append, membership, fichier absent → false). Via `ARMADAI_SESSION_ENDS` (comme `ARMADAI_SESSION_INDEX`, avec `ENV_MUTEX`).
- `__claude-register-session-end` : payload stdin → ligne dans le fichier ends (comme le test register de P1).
- `drive_session` (follow) : transcript avec un tour terminal `end_turn` **mais session NON marquée finie** → ne finalise PAS (pas de `Result`) dans une fenêtre courte (bounded, via `drive_session_tuned`) ; puis marquer `is_ended` → finalise (émet `Result`) au poll suivant.
- `drive_session` (replay, `follow=false`) : **inchangé** (finalise à l'EOF) — test existant conservé/vérifié.
- Gate workspace-wide : fmt + clippy 3 combos + tests 3 modes ; `cargo build -p armadai-core` inchangé.
- Runtime (optionnel, validation Dimitri) : session live multi-tours → `watch` suit jusqu'au vrai `SessionEnd`, ne finalise pas entre les tours.

## Hors périmètre (P2)

- **Couche hooks temps réel complète** (SubagentStart/Stop, PostToolUse pour push basse-latence) — abandonnée (faible ROI, le tail suffit).
- Tool calls individuels comme nœuds/lignes ; drill-down sous-agents (`agent-<id>.jsonl`).
- Purge de `claude-session-ends.jsonl` (croissance lente, non traitée).
- P3 (intégration relais armadai + retrait marqueurs) — spec/lot séparé, ensuite.

## Risques

- **Payload `SessionEnd`** : la présence de `session_id` est supposée (comme `SessionStart`, confirmé en P1) — à valider au test live ; le filet idle-abandon couvre le cas où le marqueur n'arrive pas.
- **Régression du mode follow** : retirer la finalisation `stop_reason` du mode follow ne doit PAS affecter le replay (`follow=false`), qui garde l'EOF. Test explicite des deux chemins.
- **Sessions abandonnées sans SessionEnd** : le filet idle-abandon (seuil large) évite un tail infini.
