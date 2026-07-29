# C9 Lot 3 — Exposition (C6 web : hierarchical + arbre des sous-runs) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`).

**Goal:** Afficher dans le trace UI web (C6) les runs hierarchical persistés (Lot 2) : arbre de délégation (mermaid) + sous-runs blackboard/ring imbriqués dépliables, chacun réutilisant le rendu séquence/timeline existant.

**Architecture:** Backend axum : enrichir `get_orchestration_trace_detail` avec `delegation_events` + `children` (sous-runs avec leurs entries), filtrer la liste sur les racines. Frontend : `index.html` détecte un run hierarchical et rend l'arbre de délégation + les sous-runs.

**Tech Stack:** Rust edition 2024, axum, serde_json ; SPA HTML/JS embarquée + Mermaid (déjà présent).

## Global Constraints

- Base = `origin/release/1.0.0` (@ `f0cd9e2`, après C9 Lot 2). Branche `feat/c9-lot3-exposition`, PR vers `release/1.0.0`.
- Le module `web` est gated `web`, storage gated `storage`. Vérifier : clippy CI standard `--no-default-features --features tui -- -D warnings` ET `--features tui,providers-api` (pas de régression), PLUS **`cargo clippy --no-default-features --features tui,web,storage -- -D warnings`** et **`cargo build --release`** (features par défaut incl. web+storage). `cargo fmt -- --check`.
- Tests backend : `cargo test --no-default-features --features tui,web,storage -p armadai`.
- Réutiliser les queries du Lot 2 (`get_delegation_events`, `get_child_orchestration_runs`) et les queries entries existantes (`get_board_entries`, `get_ring_contributions`, `get_ring_votes`). Ne PAS créer de nouvelle query.
- Réutiliser le rendu frontend existant (`generateTraceSequenceDiagram`, `generateTraceTimeline`) pour chaque sous-run.
- **JSONL headless : hors périmètre code** — les événements `NestedStart`/`NestedEnd` traversent déjà le sink partagé (Lot 1) et sont couverts par `test_nested_blackboard_runs_and_folds_metrics` (assert `nested_start` reçu). Aucun code à ajouter ; une note de vérification suffit (voir Notes).

---

### Task 1: Backend — détail hierarchical (delegation_events + children) + liste racines

**Files:**
- Modify: `src/web/api.rs`

**Interfaces:**
- Consumes (Lot 2) : `queries::get_delegation_events(db, run_id) -> Vec<DelegationEventRecord{run_id,seq,from_agent,to_agent,message,depth}>`, `queries::get_child_orchestration_runs(db, parent_run_id) -> Vec<OrchestrationRunRecord>`, `OrchestrationRunRecord.parent_run_id`.
- Consumes (existant) : `get_board_entries`, `get_ring_contributions`, `get_ring_votes`.
- Produces : `get_orchestration_trace_detail` renvoie en plus `"parent_run_id"`, `"delegation_events": [...]`, `"children": [ { "run": {...}, "board_entries": [...], "ring_contributions": [...], "ring_votes": [...] } ]`. La liste `get_orchestration_trace` ne renvoie que les runs racines (`parent_run_id` nul).

- [ ] **Step 1: Write the failing test**

Dans le module test de `api.rs` (qui a déjà `TempStorageGuard`, `insert_run_with_id`, `insert_orchestration_run`, etc.), ajouter :

```rust
    #[tokio::test]
    async fn test_trace_detail_hierarchical_has_delegation_events_and_children() {
        let _guard = TempStorageGuard::new();
        let db = crate::storage::init_db().unwrap();

        // Parent hierarchical run.
        insert_run_with_id(&db, "h-1", RunRecord {
            agent: "coordinator".to_string(), input: "go".to_string(), output: "done".to_string(),
            provider: "orchestration".to_string(), model: String::new(),
            tokens_in: 10, tokens_out: 20, cost: 0.0, duration_ms: 0, status: "success".to_string(),
        }).unwrap();
        insert_orchestration_run(&db, OrchestrationRunRecord {
            run_id: "h-1".to_string(), pattern: "hierarchical".to_string(), config_json: "{}".to_string(),
            outcome_json: None, rounds: 2, halt_reason: None, parent_run_id: None,
        }).unwrap();
        insert_delegation_event(&db, DelegationEventRecord {
            run_id: "h-1".to_string(), seq: 0, from_agent: "coordinator".to_string(),
            to_agent: "research-lead".to_string(), message: "analyze".to_string(), depth: 1,
        }).unwrap();

        // Nested child blackboard run linked to the parent.
        insert_run_with_id(&db, "c-1", RunRecord {
            agent: "orchestration:blackboard".to_string(), input: "analyze".to_string(),
            output: "x".to_string(), provider: "orchestration".to_string(), model: String::new(),
            tokens_in: 5, tokens_out: 5, cost: 0.0, duration_ms: 0, status: "success".to_string(),
        }).unwrap();
        insert_orchestration_run(&db, OrchestrationRunRecord {
            run_id: "c-1".to_string(), pattern: "blackboard".to_string(), config_json: "{}".to_string(),
            outcome_json: None, rounds: 1, halt_reason: None, parent_run_id: Some("h-1".to_string()),
        }).unwrap();
        insert_board_entry(&db, BoardEntryRecord {
            run_id: "c-1".to_string(), agent: "searcher".to_string(), round: 1, kind: "finding".to_string(),
            content: "a finding".to_string(), refs_json: "[]".to_string(), confidence: 0.9,
            tokens_in: 5, tokens_out: 5,
        }).unwrap();

        drop(db);

        // Detail of the hierarchical run.
        let response = get_orchestration_trace_detail(Path("h-1".to_string())).await;
        let v = response.0;
        assert_eq!(v["run"]["pattern"], "hierarchical");
        let events = v["delegation_events"].as_array().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["to"], "research-lead");
        let children = v["children"].as_array().unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0]["run"]["pattern"], "blackboard");
        assert_eq!(children[0]["board_entries"].as_array().unwrap().len(), 1);

        // The list shows only the root (the nested child is hidden).
        let list = get_orchestration_trace().await.0;
        let traces = list["traces"].as_array().unwrap();
        assert!(traces.iter().any(|t| t["id"] == "h-1"));
        assert!(!traces.iter().any(|t| t["id"] == "c-1"), "nested child must not appear in the list");
    }
```

- [ ] **Step 2: Run to verify fail**

Run: `cargo test --no-default-features --features tui,web,storage -p armadai test_trace_detail_hierarchical`
Expected: FAIL (`delegation_events`/`children` absent ; child `c-1` currently appears in the list).

- [ ] **Step 3: Extract a per-run entries helper**

Dans `api.rs`, ajouter (gated `#[cfg(feature = "storage")]`) un helper qui renvoie les trois collections d'entries pour un run, pour le réutiliser au niveau du run principal ET de chaque enfant :

```rust
#[cfg(feature = "storage")]
fn fetch_run_entries(
    db: &crate::storage::Database,
    run_id: &str,
) -> (Vec<serde_json::Value>, Vec<serde_json::Value>, Vec<serde_json::Value>) {
    use crate::storage::queries;
    let board_entries = queries::get_board_entries(db, run_id)
        .unwrap_or_default()
        .into_iter()
        .map(|e| serde_json::json!({
            "agent": e.agent, "round": e.round, "kind": e.kind, "content": e.content,
            "refs": e.refs_json, "confidence": e.confidence,
            "tokens_in": e.tokens_in, "tokens_out": e.tokens_out,
        }))
        .collect();
    let ring_contributions = queries::get_ring_contributions(db, run_id)
        .unwrap_or_default()
        .into_iter()
        .map(|c| serde_json::json!({
            "agent": c.agent, "lap": c.lap, "position_in_lap": c.position_in_lap,
            "action": c.action, "content": c.content, "reactions": c.reactions_json,
            "tokens_in": c.tokens_in, "tokens_out": c.tokens_out,
        }))
        .collect();
    let ring_votes = queries::get_ring_votes(db, run_id)
        .unwrap_or_default()
        .into_iter()
        .map(|v| serde_json::json!({
            "agent": v.agent, "position": v.position, "confidence": v.confidence,
            "supports": v.supports, "concerns": v.concerns,
        }))
        .collect();
    (board_entries, ring_contributions, ring_votes)
}
```

- [ ] **Step 4: Rewrite the storage `get_orchestration_trace_detail` to use the helper + add delegation_events + children**

Remplacer le corps (version `#[cfg(feature = "storage")]`) après l'ouverture de `db` :

```rust
    let run = queries::get_orchestration_run(&db, &run_id)
        .ok()
        .flatten()
        .map(|r| {
            serde_json::json!({
                "id": r.run_id,
                "pattern": r.pattern,
                "config": r.config_json,
                "outcome": r.outcome_json,
                "rounds": r.rounds,
                "halt_reason": r.halt_reason,
                "parent_run_id": r.parent_run_id,
            })
        });

    let (board_entries, ring_contributions, ring_votes) = fetch_run_entries(&db, &run_id);

    let delegation_events: Vec<serde_json::Value> = queries::get_delegation_events(&db, &run_id)
        .unwrap_or_default()
        .into_iter()
        .map(|e| serde_json::json!({
            "seq": e.seq, "from": e.from_agent, "to": e.to_agent,
            "message": e.message, "depth": e.depth,
        }))
        .collect();

    let children: Vec<serde_json::Value> = queries::get_child_orchestration_runs(&db, &run_id)
        .unwrap_or_default()
        .into_iter()
        .map(|c| {
            let (cb, cc, cv) = fetch_run_entries(&db, &c.run_id);
            serde_json::json!({
                "run": {
                    "id": c.run_id, "pattern": c.pattern, "config": c.config_json,
                    "outcome": c.outcome_json, "rounds": c.rounds, "halt_reason": c.halt_reason,
                    "parent_run_id": c.parent_run_id,
                },
                "board_entries": cb,
                "ring_contributions": cc,
                "ring_votes": cv,
            })
        })
        .collect();

    Json(serde_json::json!({
        "run": run,
        "board_entries": board_entries,
        "ring_contributions": ring_contributions,
        "ring_votes": ring_votes,
        "delegation_events": delegation_events,
        "children": children,
    }))
```

Mettre à jour le `empty` closure (début de la fonction) pour inclure les nouveaux champs :

```rust
    let empty = || {
        serde_json::json!({
            "run": null,
            "board_entries": [],
            "ring_contributions": [],
            "ring_votes": [],
            "delegation_events": [],
            "children": [],
        })
    };
```

- [ ] **Step 5: Update the non-storage stub**

Dans la version `#[cfg(not(feature = "storage"))]` de `get_orchestration_trace_detail`, ajouter `"delegation_events": [], "children": []` au JSON renvoyé.

- [ ] **Step 6: Filter the list to roots only**

Dans `get_orchestration_trace` (version storage), filtrer les enfants avant le `.map` :

```rust
            let traces: Vec<serde_json::Value> = runs
                .iter()
                .filter(|r| r.parent_run_id.is_none())
                .map(|r| {
                    serde_json::json!({
                        "id": r.run_id,
                        "pattern": r.pattern,
                        "config": r.config_json,
                        "outcome": r.outcome_json,
                        "rounds": r.rounds,
                        "halt_reason": r.halt_reason,
                    })
                })
                .collect();
```

- [ ] **Step 7: Run the test + full web suite**

Run: `cargo test --no-default-features --features tui,web,storage -p armadai test_trace_detail_hierarchical`
Expected: PASS.
Run: `cargo test --no-default-features --features tui,web,storage -p armadai api::`
Expected: PASS (l'ancien `test_get_orchestration_trace_detail_returns_run_and_entries` inchangé — les nouveaux champs s'ajoutent).

- [ ] **Step 8: Clippy + fmt + build**

Run: `cargo clippy --no-default-features --features tui,web,storage -- -D warnings && cargo clippy --no-default-features --features tui -- -D warnings && cargo fmt -- --check && cargo build --release`
Expected: clean.

- [ ] **Step 9: Commit**

```bash
git add src/web/api.rs
git commit -m "feat(web): hierarchical trace detail — delegation events + nested children; list roots only"
```

---

### Task 2: Frontend — rendu hierarchical (arbre de délégation + sous-runs)

**Files:**
- Modify: `src/web/index.html`

**Interfaces:**
- Consumes : `/api/orchestration/trace/{id}` renvoie désormais `delegation_events` + `children` (Task 1). Réutilise `generateTraceSequenceDiagram`, `generateTraceTimeline`, `mermaidSanitizeId`, `mermaidEscapeLabel`, `escapeHtml`, `escapeHtmlContent`, `truncateText`.

- [ ] **Step 1: Add a delegation-tree diagram generator**

Dans `index.html`, avant `viewTraceRun`, ajouter :

```javascript
// Build a mermaid flowchart of the hierarchical delegation tree from events.
function generateDelegationDiagram(delegationEvents) {
  const lines = ['flowchart TD'];
  const seenNode = new Set();
  const node = (name) => {
    const id = mermaidSanitizeId(name);
    if (!seenNode.has(id)) {
      seenNode.add(id);
      const label = mermaidEscapeLabel(String(name == null ? 'unknown' : name), 30) || id;
      lines.push(`${id}["${label}"]`);
    }
    return id;
  };
  (delegationEvents || []).forEach(ev => {
    const from = node(ev.from);
    const to = node(ev.to);
    const label = mermaidEscapeLabel(ev.message || '', 40);
    lines.push(label ? `${from}-->|${label}| ${to}` : `${from}--> ${to}`);
  });
  return lines.join('\n');
}
```

- [ ] **Step 2: Render hierarchical runs in `viewTraceRun`**

Dans `viewTraceRun`, après `const outcomeObj = parseJsonSafe(run.outcome);` et avant le calcul de `bodyHtml`, insérer la branche hierarchical. Remplacer le bloc `let bodyHtml; if (!hasEntries) {...} else {...}` par :

```javascript
  const delegationEvents = d.delegation_events || [];
  const children = d.children || [];
  const isHierarchical = (run.pattern === 'hierarchical') || delegationEvents.length > 0 || children.length > 0;

  let bodyHtml;
  if (isHierarchical) {
    if (!delegationEvents.length && !children.length) {
      bodyHtml = '<div class="empty">This hierarchical run recorded no delegations or nested sub-runs.</div>';
    } else {
      const tree = generateDelegationDiagram(delegationEvents);
      const childSections = children.map((c, i) => {
        const cRun = c.run || {};
        const cBoard = c.board_entries || [];
        const cContribs = c.ring_contributions || [];
        const cVotes = c.ring_votes || [];
        const cHasEntries = cBoard.length > 0 || cContribs.length > 0;
        const inner = cHasEntries
          ? `<div class="list-section"><h4>Sequence</h4><div class="mermaid">${escapeHtml(generateTraceSequenceDiagram(cBoard, cContribs, cVotes))}</div></div>
             <div class="list-section"><h4>Timeline</h4>${generateTraceTimeline(cBoard, cContribs, cVotes)}</div>`
          : '<div class="empty">No entries for this sub-run.</div>';
        const lead = cRun.pattern || 'sub-run';
        return `<details${i === 0 ? ' open' : ''}>
          <summary>Nested ${escapeHtmlContent(lead)} — ${escapeHtmlContent(String(cRun.id || ''))}</summary>
          ${inner}
        </details>`;
      }).join('');
      bodyHtml = `
        <div class="list-section"><h3>Delegation Tree</h3><div class="mermaid">${escapeHtml(tree)}</div></div>
        <div class="list-section"><h3>Nested Sub-runs (${children.length})</h3>${childSections || '<div class="empty">None.</div>'}</div>`;
    }
  } else if (!hasEntries) {
    bodyHtml = '<div class="empty">This run has no recorded board/ring entries (direct pattern, or an empty trace).</div>';
  } else {
    const diagram = generateTraceSequenceDiagram(boardEntries, ringContributions, ringVotes);
    const timeline = generateTraceTimeline(boardEntries, ringContributions, ringVotes);
    bodyHtml = `
      <div class="list-section"><h3>Sequence Diagram</h3><div class="mermaid">${escapeHtml(diagram)}</div></div>
      <div class="list-section"><h3>Timeline</h3>${timeline}</div>`;
  }
```

- [ ] **Step 3: Ensure mermaid renders for hierarchical too**

Remplacer la dernière ligne `if (hasEntries) mermaid.run();` par :

```javascript
  if (hasEntries || isHierarchical) mermaid.run();
```

- [ ] **Step 4: Build (compiles the embedded SPA)**

Run: `cargo build --release`
Expected: success (la SPA est embarquée à la compilation).

- [ ] **Step 5: Clippy + fmt**

Run: `cargo clippy --no-default-features --features tui,web,storage -- -D warnings && cargo fmt -- --check`
Expected: clean (Task 2 ne touche que du HTML/JS embarqué — pas de code Rust, mais lancer par sûreté).

- [ ] **Step 6: Commit**

```bash
git add src/web/index.html
git commit -m "feat(web): render hierarchical traces — delegation tree + expandable nested sub-runs"
```

---

## Notes pour l'implémenteur

- **Validation manuelle (Task 2)** : `cargo run --release -- web`, générer un run hierarchical avec sous-pattern (config `orchestration.teams[].pattern: blackboard`) via `armadai run --orchestrate hierarchical ...`, ouvrir l'onglet Traces → cliquer le run hierarchical → vérifier l'arbre de délégation + les sous-runs dépliables. Le rendu Mermaid en navigateur ne peut pas être testé automatiquement ; le signaler dans la PR comme point à valider par l'utilisateur (cohérent avec la dette C6 déjà notée).
- **JSONL headless** : aucun code à écrire. Vérifier (et le mentionner dans le rapport) que `armadai run --orchestrate hierarchical --headless --json ...` sur une config avec sous-pattern émet bien des lignes `{"t":"nested_start",...}` / `{"t":"nested_end",...}` — c'est déjà le cas (sink partagé, Lot 1). Si tu veux un garde-fou, ajouter un test asserting qu'un run hierarchical imbriqué via `CaptureSink` contient `nested_start` (mais `test_nested_blackboard_runs_and_folds_metrics` le couvre déjà — ne pas dupliquer).
- Ne PAS créer de nouvelle query storage ni toucher au moteur.
- `<h4>` dans les sous-sections : réutiliser le style existant (les `.list-section h3` existent ; `h4` héritera raisonnablement — ne pas ajouter de CSS sauf si le rendu casse).
