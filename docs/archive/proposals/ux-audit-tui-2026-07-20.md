# UX/UI Consistency Audit — ArmadAI Terminal UIs

**Date:** 2026-07-20
**Scope:** Shell TUI (`src/shell/`) + Dashboard TUI (`src/tui/`)
**Mode:** Findings-first. No code was modified.

---

## Summary

**Dashboard TUI (`src/tui/`)** — The more mature and internally consistent of the two. Every list
view (Agents, Prompts, Skills, Starters, History, Models, Orchestration) shares the same skeleton:
a `>` selection marker, cyan-bold selected row, a title showing `— N loaded, M shown` + sort
indicator, an identical yellow `/ query█` search bar, and a per-tab shortcuts legend. Selection,
search and sort are wired uniformly through `filter.rs`. The main weaknesses are (a) the Costs tab
is a dead-end that breaks the list conventions every other tab follows, (b) detail views can't
scroll so long content is silently truncated, and (c) massive copy-paste of the search-bar renderer.

**Shell TUI (`src/shell/`)** — Functionally rich (streaming, tandem, pipeline, PTY, workroom) but
visually rougher and far less discoverable. There is **no persistent legend/shortcut bar** — once
the welcome placeholder disappears, the only affordance hints are gone. Colors are hardcoded to a
GitHub-dark palette (`md_render.rs`, statusbar bg, workroom) that degrades badly on light terminals,
and the workroom's newly-enriched per-agent data (`last_action`, `transitions`, durations) is
collected on every turn but **never rendered** — the single biggest untapped opportunity. The two
TUIs also do not share a palette, a spinner constant, or quit/back conventions.

**Finding counts:** P0 = 2 · P1 = 8 · P2 = 8

---

## P0 — Blocking / ugly

### P0-1 · Workroom "idle" and "done" agents are effectively invisible
- **File:** `src/shell/workroom.rs:429` (`Done` → `Color::DarkGray`), `:434` (`Idle` → `Color::Rgb(60,60,60)`)
- **What's wrong:** `Rgb(60,60,60)` is near-black; on a typical dark terminal the `○ idle` rows and
  the agent name are barely legible, and on a light terminal `DarkGray`/near-black-on-white also
  washes out. In an orchestration with many idle agents, the panel reads as empty. The distinction
  between "not started" and "finished" is lost because both are grey.
- **Recommendation:** Use theme-safe named colors: `Idle` → `Color::DarkGray`, `Done` →
  `Color::Green`-dim or a `✓` in `Color::Green`. Never use `Rgb` below ~90 for foreground text.
- **Effort:** S · **Risk:** Low

### P0-2 · Hardcoded dark-theme colors break on light terminals (cross-cutting)
- **Files:** `src/shell/md_render.rs:15-27` (explicit "GitHub dark theme" RGB block),
  `src/shell/tui.rs:843` (statusbar `bg(Rgb(22,27,34))` under `fg(DarkGray)`),
  `src/tui/views/agent_detail.rs:235` / `prompt_detail.rs:107` / `skill_detail.rs:142`
  (`fg(Color::White)` body text).
- **What's wrong:** The shell assumes a dark background. On a light terminal the statusbar renders
  dark-grey text on a near-black bar (unreadable strip), and `fg(White)` prompt/body bodies in the
  dashboard detail views become white-on-white. `md_render` fixes absolute RGB regardless of theme.
- **Recommendation:** Prefer ratatui named colors (which map to the terminal palette) for text;
  reserve `Rgb` for accents only, and drop the fixed statusbar background (or pair it with an
  explicit light fg). A shared `theme.rs` with semantic constants (see P1-8) is the durable fix.
- **Effort:** M · **Risk:** Medium (visual regression on the maintainer's own dark terminal if colors shift)

---

## P1 — Real inconsistencies

### P1-1 · Shell has no persistent shortcut/legend bar
- **File:** `src/shell/tui.rs:624-677` (render composes header/messages/statusbar/input, no legend);
  hints exist only in the empty-state placeholder `:717-724`.
- **What's wrong:** The dashboard shows a per-view shortcuts bar (`tui/views/shortcuts.rs`); the
  shell shows nothing. Slash commands (`/switch`, `/tandem`, `/pipeline`, `/workroom`, `/pty`,
  `/resume`, `/save`, `/clear`) and keys (Ctrl+L, Ctrl+C, PageUp/Down, ↑↓ history) are undiscoverable
  once the user types their first message.
- **Recommendation:** Add a one-line hint bar (above or fused with the statusbar) e.g.
  `Enter send · /help commands · Ctrl+L clear · Esc quit`, and a `/help` popup listing all slash
  commands. This is also the natural home for the workroom-focus hint (see Drill-down section).
- **Effort:** M · **Risk:** Low

### P1-2 · Shell popup is dismissed by *any* unhandled key
- **File:** `src/shell/tui.rs:456` (`_ => self.dismiss_popup()`)
- **What's wrong:** In the popup key handler, only Esc/q/Enter/↑↓/PgUp/PgDn are meaningful; every
  other key (Left, Right, Home, End, letters) closes the overlay. A user pressing an arrow to scroll
  horizontally or fat-fingering a key loses the popup. The dashboard's palette (`tui/mod.rs:81`) and
  search (`:100`) instead treat unknown keys as no-ops — opposite behavior.
- **Recommendation:** Change the fallthrough to `_ => {}` (no-op) so only the documented close keys
  dismiss. Matches the popup's own footer hint ("Esc to close").
- **Effort:** S · **Risk:** Low

### P1-3 · Esc semantics are inconsistent and dangerous in the shell
- **File:** `src/shell/tui.rs:463-466` (Esc → `should_quit = true`) vs `src/tui/mod.rs:109-138`
  (Esc = back-to-parent from detail views, quit only from a top-level tab).
- **What's wrong:** In the shell, a bare Esc at the REPL quits the entire session immediately with no
  confirmation — even mid-conversation. Users conditioned by the dashboard (Esc = "go back / cancel")
  will lose their session. There is no "cancel current input" affordance either.
- **Recommendation:** Make shell Esc context-aware: if input is non-empty, clear it; else no-op or
  require a second Esc / `Ctrl+C` to quit (or a `/quit` confirmation). At minimum, document it in the
  legend (P1-1).
- **Effort:** M · **Risk:** Medium (changes muscle-memory quit path)

### P1-4 · Quit and navigation keys diverge between the two TUIs
- **Files:** shell `src/shell/tui.rs:463-475` (quit = Esc / Ctrl+C; `q` is text input);
  dashboard `src/tui/mod.rs:141` (quit = `q`) & `:136` (Esc at top level).
- **What's wrong:** `q` quits the dashboard but is a literal character in the shell; Ctrl+C quits the
  shell but is unhandled in the dashboard. Nav is `j/k`+arrows in the dashboard, arrows-only (and ↑↓
  are history, not scroll) in the shell. No shared mental model.
- **Recommendation:** Document both explicitly in each legend; where a key can be shared without
  clashing (Ctrl+C to quit in both), align it. Full unification is subjective — flag for arbitration.
- **Effort:** S (doc) / L (unify) · **Risk:** Low / Medium

### P1-5 · Costs tab breaks every list convention
- **Files:** `src/tui/views/costs.rs` (no `>` marker, no selection style, no search bar, no sort);
  `src/tui/views/shortcuts.rs:67-72` (Costs legend omits `j/k`, `/`, `s`).
- **What's wrong:** Every other content tab is navigable/searchable/sortable and shows a selection
  marker; Costs is a static table. Pressing `/` or `s` on Costs silently does nothing (see also the
  `is_searchable`/`is_sortable` guards in `tui/mod.rs:150-167,175-193` that exclude Costs). This reads
  as a half-finished tab.
- **Recommendation:** Either bring Costs up to parity (selection + `/` filter + `s` sort by cost/name
  + a total row that's visually distinct) or intentionally style it as a read-only summary and note
  that in the title. Sorting costs by cost descending is the obvious default.
- **Effort:** M · **Risk:** Low

### P1-6 · Cost / numeric formatting is inconsistent across views
- **Files:** `costs.rs:39` (`${:.6}` per row) vs `costs.rs:59` title (`${:.4}`);
  `history.rs:66` (`{:.4}` — no `$`); `models_list.rs:57,63` (`${:.2}`);
  context window `models_list.rs:52` (`{}K`) vs `model_detail.rs:56` (raw integer).
- **What's wrong:** Cost appears as `$0.000001`, `$0.0000`, `0.0001`, `$1.50` depending on where you
  look; the same context window shows `200K` in the list and `200000` in the detail. Users can't
  compare figures at a glance.
- **Recommendation:** Centralize `format_cost(f64)` and `format_context(u64)` helpers and use them
  everywhere. Pick one cost precision (`$0.0000` reads well for per-run) and always show `K`/`M`.
- **Effort:** S · **Risk:** Low

### P1-7 · Dashboard detail views cannot scroll — long content is truncated
- **Files:** `agent_detail.rs:30-50` (fixed `Constraint::Length`/`Min(6)` panels, no scroll),
  `prompt_detail.rs`, `skill_detail.rs`, `orchestration.rs:181` (`.scroll((0,0))` hardcoded).
- **What's wrong:** A long system prompt, SKILL.md body, or orchestration config JSON is clipped to
  the panel height with no way to see the rest. The shell popup, by contrast, supports ↑↓/PgUp/PgDn
  scrolling (`tui.rs:444-455`) — so the app already knows how to do this, just not here.
- **Recommendation:** Add a scroll offset to detail views (reuse the shell popup's scroll model) and
  surface `↑↓ scroll` in the detail shortcuts. Minimum viable: make the body panel `Min(0)` and add
  a scroll offset driven by `j/k`.
- **Effort:** M · **Risk:** Low

### P1-8 · No shared color/semantic palette between the two TUIs
- **Files:** shell role colors `workroom.rs:439-442` (Coordinator `Rgb(231,76,60)`, Lead
  `Rgb(243,156,18)`, Agent `Rgb(88,166,255)`); dashboard accents `agent_detail.rs:143` (pattern =
  `Magenta`), `:178` (ring = `Blue`), tags = `Yellow`, stacks = `Green`. Selection = `Cyan` in the
  dashboard, absent in the workroom.
- **What's wrong:** The same concepts are colored differently across (and within) the two UIs. There
  is no single source of truth, so "selected", "active", "muted", "error", "role" drift. Cyan means
  "selected/brand" in the dashboard but "border/title" in the shell.
- **Recommendation:** Introduce `src/<shared>/theme.rs` with semantic constants (SELECTION, ACTIVE,
  MUTED, ERROR, ROLE_COORDINATOR/LEAD/AGENT, HEADING) and route both TUIs through it. Resolves P0-1,
  P0-2, and this finding together. Larger, needs sign-off on the actual palette.
- **Effort:** L · **Risk:** Medium

---

## P2 — Polish

### P2-1 · `render_search_bar` is copy-pasted in 7 files
- **Files:** identical fn in `dashboard.rs:156`, `history.rs:104`, `orchestration.rs:108`,
  `models_list.rs:121`, `prompts_list.rs:91`, `skills_list.rs:101`, `starters_list.rs:97`.
- **Recommendation:** Extract one `widgets::search_bar(frame, query, area)`. Pure dedup.
- **Effort:** S · **Risk:** Low

### P2-2 · Spinner frames duplicated
- **Files:** `src/shell/workroom.rs:52` (`SPINNER`) and `src/shell/tui.rs:19` (`SPINNER_FRAMES`) —
  byte-identical arrays.
- **Recommendation:** Define once (e.g. in `shell/mod.rs`) and share.
- **Effort:** S · **Risk:** Low

### P2-3 · Three different cursor glyphs
- **Files:** search bar `█` (`dashboard.rs:164` et al.), palette input `_` (`palette.rs:31`), shell
  input reverse-video block (`tui.rs:976-1000`).
- **Recommendation:** Pick one insertion-cursor representation (reverse-video block is the most
  standard) and use it across search + palette + shell input.
- **Effort:** S · **Risk:** Low

### P2-4 · Command palette has no footer hint and no match highlighting
- **File:** `src/tui/views/palette.rs:71-76` (list has no "↑↓ select · Enter run · Esc close" line;
  matched substring not emphasized).
- **Recommendation:** Add a footer hint (the shell popup already sets this precedent, `tui.rs:697`)
  and bold the matched portion of each command name.
- **Effort:** S · **Risk:** Low

### P2-5 · Numeric tab shortcuts (1–8) are undiscoverable
- **File:** `src/tui/mod.rs:296-304` handles `1`–`8`; `shortcuts.rs` never mentions them.
- **Recommendation:** Add `1-8 Jump` to the dashboard legend, or show the digit next to each tab title.
- **Effort:** S · **Risk:** Low

### P2-6 · Dead code in the shell wizard
- **File:** `src/shell/wizard.rs:420` (`detect_model_name` — a second, unused copy; the live one is
  `shell/detect.rs:131`), `wizard.rs:19` + `:414` (`WizardResult.provider_args` — set but never read;
  `app.rs:130` uses `detect::args_for_provider` instead), and `WizardResult.project_name` (built at
  `:416`, never consumed).
- **Recommendation:** Delete the unused fn and fields. (Hygiene — will also silence future dead-code
  lints if `#[allow]` is ever tightened.)
- **Effort:** S · **Risk:** Low

### P2-7 · Workroom width is a fixed 35 cols with no narrow-terminal guard
- **File:** `src/shell/tui.rs:659` (`Constraint::Length(35)` for the workroom column).
- **What's wrong:** On a ~60-col terminal the message area is squeezed to ~23 cols. No minimum-width
  fallback that hides the panel when the terminal is too narrow.
- **Recommendation:** Only split when `frame.area().width` exceeds a threshold (e.g. ≥ 80); otherwise
  keep the workroom hidden or overlay it.
- **Effort:** S · **Risk:** Low

### P2-8 · History "sort" sorts by agent name, not by recency
- **Files:** `filter.rs:193` (history sorted by `agent`); History has no timestamp column
  (`history.rs:31-33`).
- **What's wrong:** The most useful ordering for a history log is chronological, but `s` only cycles
  name A→Z/Z→A on the agent field, and there's no time column to anchor the user.
- **Recommendation:** Add a timestamp/relative-time column and a "recent first" default sort.
- **Effort:** M · **Risk:** Low

---

## Workroom drill-down — recommended interaction design

**Opportunity.** `TrackedAgent` now carries `last_action: Option<String>`, `transitions:
Vec<(AgentState, Instant)>`, plus `started_at`/`finished_at` (`workroom.rs:38-41,34-35`). All of it
is populated during streaming (`apply_marker`, `set_state` at `workroom.rs:293-309,357-393`) but the
renderer (`workroom.rs:401-474`) only draws icon + name + state. None of the enriched data reaches
the screen. The goal: let the user select an agent and open a detail popup showing its timeline.

**Hard constraint from the shell interaction model.** `ShellApp::handle_key` (`tui.rs:437-572`) is an
input REPL: **every `KeyCode::Char(c)` is inserted into the input buffer** (`tui.rs:477-482`). So
bare letters (`j`/`k`/`q`) and Enter/Esc are already claimed:
- `Enter` → submit (consumed both in `handle_key` `:566` *and* re-checked in the event loop at
  `app.rs:261-266`),
- `Esc` → quit (`tui.rs:463`),
- `↑`/`↓` → input-history navigation (`tui.rs:518-557`),
- `PageUp`/`PageDown` → message scroll (`tui.rs:558-565`).

A drill-down therefore **cannot** use bare keys at the REPL — it needs an explicit **focus mode** that
gates key handling, exactly like the existing popup gate (`tui.rs:441-459`) which short-circuits all
keys while a popup is open.

**Recommended design — a focus toggle + gated navigation:**

1. **Focus toggle key: `Ctrl+W`** (mnemonic "Workroom"; currently free — `tui.rs` only binds Ctrl+C
   and Ctrl+L). If the workroom is hidden, `Ctrl+W` shows+pins it and enters focus; if already
   focused, `Ctrl+W` exits focus back to the REPL. Do **not** use `Tab` (reserve it for future input
   completion) and never a bare letter.

2. **New `ShellApp` state:** `workroom_focus: bool` and delegate a selection index to the workroom
   (`Workroom` gains `selected: usize` + `focused: bool` with setters, since `agents` is private).
   Add `Workroom::select_next()/select_prev()` (wrapping, mirroring `tui/app.rs:566-749`) and
   `Workroom::selected_detail_markdown() -> Option<String>`.

3. **Gate at the very top of `handle_key`, before the text-input branch** (insert a block after the
   popup gate, ~`tui.rs:459`):
   - `↑`/`↓` **or** `j`/`k` → `select_prev()` / `select_next()` (consumed; NOT inserted into input),
   - `Enter` → build the detail markdown and call `self.show_popup(md)` — reusing the existing
     markdown overlay (`show_popup` `tui.rs:320`, rendered by `render_popup` `:680-715` via
     `md_render::render_markdown`),
   - `Esc` **or** `Ctrl+W` → exit focus (set `workroom_focus = false`) — **must return before the
     Esc-quit branch** so focus-exit never quits the app,
   - all other keys → no-op while focused (don't leak into the input buffer).
   Because Enter is intercepted and consumed here, also guard the event-loop submit check
   (`app.rs:261`): `if app.workroom_focus { continue; }` right after `handle_key`, so a focused Enter
   opens the popup instead of submitting the pending input.

4. **Visual selection indicator** (`workroom.rs:450-455`): when `focused`, render the selected row
   reversed (`Style::add_modifier(Modifier::REVERSED)`) or prefix it with `▸`; when not focused,
   render as today. Draw a footer line inside the panel: `Ctrl+W exit · j/k select · Enter detail`.

5. **Detail popup content** (markdown, so `md_render` styles it consistently with the rest of the shell):
   ```
   # {name}  ({role})
   **State:** working · **Elapsed:** 8.4s
   **Last action:** complete

   ## Timeline
   - delegating      +0.0s
   - working         +1.2s
   - done            +8.4s
   ```
   Compute rows from `transitions` as offsets from the first transition's `Instant`; show live
   elapsed from `started_at.elapsed()` while `Working`/`Delegating`, and `finished_at - started_at`
   once `Done`. The popup already scrolls (`tui.rs:444-455`) so long timelines are fine.

**Key-binding clash summary (call-outs):**
- Bare `j`/`k`/`q`/letters — CLASH with input REPL; only usable *inside* focus mode behind the gate.
- `Enter` — CLASH (submit at REPL + re-checked in event loop `app.rs:261`); must be consumed in focus
  mode and the event-loop submit guarded on `workroom_focus`.
- `Esc` — CLASH (quit); focus-exit must be handled *before* the quit branch (`tui.rs:463`).
- `Ctrl+W` — FREE; recommended focus toggle.
- `↑`/`↓` — currently input-history; safe to repurpose *only while focused*.

---

## Shortlist A — safe consistency fixes (low-risk, objective — applyable directly)

1. **P1-2** — Change the shell popup key fallthrough `_ => self.dismiss_popup()` to `_ => {}`
   (`src/shell/tui.rs:456`) so only Esc/q/Enter close it (matches its own footer + the dashboard).
2. **P0-1** — Replace `Idle` `Rgb(60,60,60)` → `Color::DarkGray` and `Done` `DarkGray` →
   `Color::Green`-toned in `src/shell/workroom.rs:429,434` for legible, theme-safe contrast.
3. **P1-6 / P2-1 / P2-2** — Dedup + unify: extract `format_cost`/`format_context` helpers and one
   `render_search_bar` widget, and share the single SPINNER constant. Purely mechanical, no behavior
   change. (Plus the P2-6 dead-code deletion in `wizard.rs` if desired — zero runtime impact.)

## Shortlist B — needs user arbitration (subjective / larger)

- **P1-3 / P1-4** — Redefining Esc (quit vs cancel/back) and unifying quit keys changes established
  muscle memory; needs a decision on the intended contract.
- **P1-8 / P0-2** — A shared `theme.rs` and light-terminal support is a larger refactor and requires
  sign-off on the actual palette (the maintainer runs a dark terminal; shifting colors is visible).
- **P1-5** — Whether Costs becomes a full navigable list or an intentional read-only summary is a
  product call.
- **P1-7** — Adding scroll to detail views is worthwhile but touches every detail view's layout.
- **Workroom drill-down** — the focus-mode design above is a net-new interaction; confirm the
  `Ctrl+W` binding and the popup content before implementing.
