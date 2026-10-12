# Workroom Lot B — Recâblage app.rs (event-based) + follow-ups — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`).

**Goal:** Router les entrées de stream du shell vers la FSM event-based (`apply_stream_text`) au lieu du scan flou `detect_mentions`, supprimer `detect_mentions`, et appliquer les follow-ups de robustesse de la revue Lot A. Non-visuel, mergeable sur CI verte.

**Architecture:** Remplacer les 6 appels `app.workroom.detect_mentions(&text)` dans `src/shell/app.rs` par `app.workroom.apply_stream_text(&text)`, puis supprimer la méthode `detect_mentions` de `src/shell/workroom.rs`. `on_delegate` (encore utilisé à un site) reste. Follow-ups Lot A dans `workroom.rs`.

**Tech Stack:** Rust edition 2024 (gated `tui`).

## Global Constraints

- Base = `origin/release/1.0.0` (@ `6fa8477`, après Workroom Lot A). Branche `feat/workroom-lotB-rewire`, PR vers `release/1.0.0`.
- Clippy 2 modes CI `-D warnings` : `--no-default-features --features tui` ET `--features tui,providers-api`. `cargo fmt -- --check`. `cargo test --no-default-features --features tui`.
- **Non-régression comportementale** : la Workroom doit continuer à s'activer pendant l'orchestration (via les marqueurs) ; c'est un remplacement de mécanisme, pas de fonctionnalité.
- Spec : `docs/superpowers/specs/2026-07-20-workroom-event-based-design.md`. Follow-ups : cf. ledger `.superpowers/sdd/workroom-lotA-progress.md`.

---

### Task 1: Recâbler app.rs vers `apply_stream_text` + supprimer `detect_mentions`

**Files:**
- Modify: `src/shell/app.rs` (6 sites), `src/shell/workroom.rs` (suppression `detect_mentions`)

- [ ] **Step 1: Locate the call sites**

Run: `rg -n "workroom.detect_mentions" src/shell/app.rs`
Expected: 6 lines (≈ 621, 626, 937, 942, 1323, 1328), chacune `app.workroom.detect_mentions(&text);`.

- [ ] **Step 2: Replace each call**

Remplacer **chaque** `app.workroom.detect_mentions(&text);` par :

```rust
                            app.workroom.apply_stream_text(&text);
```

(même variable `text`, même indentation locale). Vérifier qu'il ne reste aucun `detect_mentions` : `rg -n "detect_mentions" src/`.

- [ ] **Step 3: Remove the `detect_mentions` method**

Dans `src/shell/workroom.rs`, supprimer entièrement la méthode `pub fn detect_mentions(&mut self, text: &str)` (le scan flou, ≈ lignes 276-323) et son test dédié s'il existe (`rg -n "detect_mentions" src/shell/workroom.rs`). Ne PAS toucher `on_delegate`/`on_complete`/`reset`/`parse_streaming_line`/`apply_stream_text`/`render`.

- [ ] **Step 4: Build + regression tests**

Run: `cargo build --no-default-features --features tui`
Expected: compiles (no remaining `detect_mentions` reference).
Run: `cargo test --no-default-features --features tui -p armadai workroom`
Expected: PASS (FSM tests + existing, minus any removed detect_mentions test).

- [ ] **Step 5: Clippy 2 modes + fmt**

Run: `cargo clippy --all-targets --no-default-features --features tui -- -D warnings && cargo clippy --all-targets --no-default-features --features tui,providers-api -- -D warnings && cargo fmt -- --check`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add src/shell/app.rs src/shell/workroom.rs
git commit -m "refactor(shell): drive workroom from marker FSM (apply_stream_text); drop detect_mentions"
```

---

### Task 2: Follow-ups de robustesse (revue Lot A)

**Files:**
- Modify: `src/shell/workroom.rs`

**Interfaces:** consomme `apply_stream_text`/`apply_marker`/`keep_tail` (Lot A).

- [ ] **Step 1: Write the failing tests**

Dans le module test de `workroom.rs` :

```rust
// The recap-guard must protect an ALREADY-DONE agent too: after an agent
// finishes, a stray END (echoed in a recap) must not re-transition anything.
#[test]
fn test_stray_end_after_agent_done_is_noop() {
    let mut wr = wr_dev_lead_core();
    wr.apply_stream_text("<!--ARMADAI_DELEGATE:core-specialist-->");
    wr.apply_stream_text("<!--ARMADAI_END-->"); // core-specialist -> Done, control back to coordinator
    let before: Vec<_> = wr.agents_for_test().iter().map(|a| (a.name.clone(), a.state.clone())).collect();
    wr.apply_stream_text("| recap echo <!--ARMADAI_END-->"); // stray, coordinator is Idle here
    let after: Vec<_> = wr.agents_for_test().iter().map(|a| (a.name.clone(), a.state.clone())).collect();
    assert_eq!(before, after, "a stray recap END must not change any agent state");
}

#[test]
fn test_empty_delegate_target_is_ignored() {
    let mut wr = wr_dev_lead_core();
    wr.apply_stream_text("<!--ARMADAI_DELEGATE:-->");
    // No agent named "" — nothing works; coordinator not forced to delegate to nobody.
    assert!(wr.agents_for_test().iter().all(|a| a.state == AgentState::Idle));
}

#[test]
fn test_unterminated_opener_buffer_is_capped() {
    let mut wr = wr_dev_lead_core();
    // A stray opener that never closes, followed by a lot of prose.
    wr.apply_stream_text("<!--ARMADAI_");
    for _ in 0..1000 {
        wr.apply_stream_text("some long prose without any closing marker ");
    }
    // Buffer must not grow unbounded.
    assert!(wr.marker_buf_len_for_test() <= 8192, "unterminated marker buffer must be capped");
}
```

Ajouter l'accesseur test-only :

```rust
#[cfg(test)]
impl Workroom {
    pub fn marker_buf_len_for_test(&self) -> usize { self.marker_buf.len() }
}
```

- [ ] **Step 2: Run to verify fail**

Run: `cargo test --no-default-features --features tui -p armadai "stray_end_after\|empty_delegate\|unterminated_opener"`
Expected: FAIL (`test_empty_delegate_target_is_ignored` — empty target currently sets current_agent to ""; `test_unterminated_opener_buffer_is_capped` — no cap; `marker_buf_len_for_test` undefined).

- [ ] **Step 3: Guard empty DELEGATE target**

Dans `apply_marker`, la branche DELEGATE : ignorer une cible vide.

```rust
        if let Some(target) = body.strip_prefix("ARMADAI_DELEGATE:") {
            let target = target.trim().to_string();
            if target.is_empty() {
                return; // malformed marker — ignore
            }
            if let Some(coord) = self.coordinator_name() {
                self.set_state(&coord, AgentState::Delegating);
            }
            self.set_state(&target, AgentState::Working);
            self.current_agent = Some(target);
            self.visible = true;
        } else if ...
```

- [ ] **Step 4: Cap the marker buffer**

Dans `apply_stream_text`, après la boucle d'extraction (avant la fin de fonction), borner le buffer résiduel (un opener non terminé ne doit pas croître sans fin) :

```rust
        // Safety cap: an unterminated `<!--ARMADAI_` opener must not let the
        // buffer grow without bound within a turn.
        const MAX_MARKER_BUF: usize = 8192;
        if self.marker_buf.len() > MAX_MARKER_BUF {
            // Keep only the tail (where a real closing `-->` would still land).
            let cut = self.marker_buf.len() - MAX_MARKER_BUF;
            // Respect char boundaries.
            let mut cut = cut;
            while cut < self.marker_buf.len() && !self.marker_buf.is_char_boundary(cut) {
                cut += 1;
            }
            self.marker_buf = self.marker_buf[cut..].to_string();
        }
```

- [ ] **Step 5: Add the test accessor + run**

Run: `cargo test --no-default-features --features tui -p armadai workroom`
Expected: PASS (new 3 tests + all existing).

- [ ] **Step 6: Clippy 2 modes + fmt**

Run: `cargo clippy --all-targets --no-default-features --features tui -- -D warnings && cargo clippy --all-targets --no-default-features --features tui,providers-api -- -D warnings && cargo fmt -- --check`
Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add src/shell/workroom.rs
git commit -m "fix(shell): guard empty delegate target + cap marker buffer; strengthen recap-guard test"
```

---

## Notes pour l'implémenteur

- Ce lot est **non-visuel** : aucun changement de rendu ni de gestion clavier. Le drill-down (sélection + overlay) et le polish couleurs sont un lot SÉPARÉ (Workroom Lot B-UI), gaté sur validation manuelle de l'utilisateur.
- `on_delegate` reste (utilisé à `app.rs:1541`, un chemin qui parse déjà un nom d'agent). Ne pas le supprimer.
- Après suppression de `detect_mentions`, vérifier `rg -n "detect_mentions" src/` = vide.
- Le comportement observable ne change quasiment pas : avant, `detect_mentions` + `parse_streaming_line` alimentaient la workroom ; maintenant `apply_stream_text` (marqueurs) le fait de façon fiable. Les projets orchestrés émettent les marqueurs (protocole injecté par le linker).
