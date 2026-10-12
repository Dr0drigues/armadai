# C6 — Web trace UI — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`).

**Goal:** Un onglet « Traces » dans le dashboard web qui liste les runs d'orchestration et affiche le déroulé d'un run (diagramme Mermaid `sequenceDiagram` + timeline) via un nouvel endpoint détail.

**Architecture:** Backend axum : endpoint `GET /api/orchestration/trace/{run_id}` réutilisant les queries storage existantes. Frontend : onglet dans la SPA `src/web/index.html` (liste + vue détail).

**Tech Stack:** Rust edition 2024, axum, serde_json ; SPA HTML/JS embarquée + Mermaid (déjà présent pour topology).

## Global Constraints
- Base = `origin/release/1.0.0` (@ `21a64a5`). Branche `feat/c6-web-trace-ui`, PR vers `release/1.0.0`.
- **Le module `web` est gated `web` (hors des modes CI standards).** Vérifier : clippy CI standard `--no-default-features --features tui -- -D warnings` ET `--features tui,providers-api` (ne doivent pas régresser), PLUS **`cargo clippy --no-default-features --features tui,web,storage -- -D warnings`** et **`cargo build --release`** (features par défaut incl. web) pour couvrir le code web. `cargo fmt -- --check`. `cargo test --no-default-features --features tui,web,storage` pour les tests backend.
- Réutiliser les queries existantes (`src/storage/queries.rs`) : `get_orchestration_run`, `get_board_entries`, `get_ring_contributions`, `get_ring_votes`. Ne PAS créer de nouvelle query storage.
- Suivre les patterns existants de `src/web/api.rs` et `index.html` (thème light/dark, fetch JSON, onglets).
- Périmètre : blackboard/ring (+ direct). Hierarchical hors scope (pas dans `orchestration_runs`).

---

### Task 1: Backend — endpoint détail
**Files:** Modify `src/web/api.rs` (nouveau handler), `src/web/mod.rs` (route). Test dans `api.rs` `#[cfg(test)]` ou `tests/`.

**Interfaces produces:** `GET /api/orchestration/trace/{run_id}` → `Json` de `{ "run": {...}, "board_entries": [...], "ring_contributions": [...], "ring_votes": [...] }`. `run_id` absent → objet avec champs vides (ou 404 propre — au choix, documenter).

- [ ] Écrire le handler `get_orchestration_trace_detail(Path(run_id): Path<String>) -> Json<serde_json::Value>` (gated `#[cfg(feature = "storage")]` pour l'accès DB ; sans storage, renvoyer `{ "run": null, ... }`). Il appelle `init_db()` puis `get_orchestration_run(&db, &run_id)`, `get_board_entries`, `get_ring_contributions`, `get_ring_votes`, sérialise chaque record en JSON (mêmes champs que les Record structs). Suivre le style de `get_orchestration_trace` (api.rs:621).
- [ ] Ajouter la route `.route("/api/orchestration/trace/{run_id}", get(api::get_orchestration_trace_detail))` dans `mod.rs` (après la route trace existante).
- [ ] Test : insérer un `orchestration_run` + quelques `board_entries`/`ring_votes` en DB de test, appeler le handler, asserter la structure JSON (run présent, entrées/votes présents avec les bons champs). Utiliser le pattern de test DB existant (init_db temporaire).
- [ ] Vérifs (voir Global Constraints — inclure `--features tui,web,storage`) + commit `feat(web): add orchestration trace detail endpoint`.

---

### Task 2: Frontend — onglet Traces (liste + détail)
**Files:** Modify `src/web/index.html`.

**Interfaces consumes:** `/api/orchestration/trace` (liste, existant), `/api/orchestration/trace/{run_id}` (détail, Task 1).

- [ ] Ajouter un onglet « Traces » à la barre d'onglets (même mécanisme que les onglets existants : Agents/Prompts/Skills/Starters/History/Costs/Models). 
- [ ] **Liste** : au chargement de l'onglet, `fetch('/api/orchestration/trace')` → tableau des runs (colonnes : id/pattern/rounds/halt_reason/outcome résumé/coût si présent). Barre de recherche/tri cohérente avec les autres onglets. Chaque ligne cliquable.
- [ ] **Détail** : au clic d'un run, `fetch('/api/orchestration/trace/{id}')` → afficher :
  - Un **diagramme Mermaid `sequenceDiagram`** généré en JS à partir des données : participants = agents distincts (des board_entries/ring_contributions) + `Board`/`Ring` ; une ligne `participant`/`->>` par entrée/contribution (ordre round/lap), votes en `Note`. Rendre via l'API Mermaid déjà utilisée pour topology (regarde comment topology rend son diagramme, index.html ~L640).
  - Une **timeline** : liste ordonnée (round/lap) — agent · type · extrait de contenu (tronqué) · confiance des votes. Rendu markdown des contenus si l'existant le fait.
  - Gérer le cas vide (run sans entrées, ou pattern direct) proprement.
- [ ] Thème light/dark : réutiliser les classes/variables CSS existantes.
- [ ] Vérifs : `cargo build --release` (compile la SPA embarquée), et **validation manuelle** (voir Notes). Commit `feat(web): add orchestration traces tab (list + detail with mermaid + timeline)`.

---

## Notes pour l'implémenteur
- Test manuel Task 2 : `cargo run --release -- web` (ou la commande qui lance le dashboard, cf. `armadai web`/`Up`), ouvrir le navigateur, onglet Traces, vérifier liste + clic → diagramme + timeline. Nécessite des runs d'orchestration en DB (en lancer un via `armadai run --orchestrate blackboard ...` au préalable, ou tester avec une DB peuplée).
- Générer le `sequenceDiagram` Mermaid : échapper les noms d'agents/contenus (pas de caractères cassant la syntaxe Mermaid). Tronquer les contenus longs dans les messages du diagramme (le détail complet va dans la timeline).
- Si `index.html` est très gros et que l'ajout devient difficile à situer, garde les ajouts groupés (bloc onglet + bloc JS de rendu) et suis la structure existante ; ne restructure pas la SPA.
