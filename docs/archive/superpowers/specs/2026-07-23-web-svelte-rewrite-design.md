# Refonte de la Web UI en Svelte (design system « pont de commandement ») — Design

## Contexte

La Web UI actuelle est un unique `src/web/index.html` (~1117 lignes, HTML+CSS+JS mêlés) — difficile à faire évoluer, et c'est justement le point de friction qui motive ce chantier. Plutôt qu'un re-skin de ce monolithe (approche abandonnée — voir specs `2026-07-22-design-system-web-a.md`, superseded), on **reconstruit** la Web UI avec un framework léger (**Svelte**), en y intégrant nativement le design system ArmadAI (« pont de commandement », projet Claude Design `416749e1-6cb2-460b-be6d-6bb2df3ddc6d`). Ce chantier remplace, pour la surface Web, l'ancien découpage Web-A/Web-B.

L'API axum (`src/web/api.rs`, endpoints `/api/*`) **ne change pas** : le nouveau front la consomme.

## Objectif

Une SPA Svelte maintenable, fidèle au design system, embarquée dans le binaire (aucune dépendance réseau), migrée **sans régression** de l'UI web existante.

## Architecture

### Front (`web/ui/`)
- **Svelte 5 (runes) + Vite + TypeScript.** Styles en **CSS scopé par composant** + une feuille de tokens globale traduite des `tokens/*.css` du DS (custom properties CSS, oklch, dark défaut + light + `@media prefers-color-scheme` + override `data-theme`). Pas de framework CSS utilitaire — les tokens DS se mappent directement.
- **Dépendances bundlées** (fini le CDN) : `mermaid` (topologie orchestration) et `marked` (markdown) deviennent des deps npm, incluses dans le bundle → self-contained.
- **Assets** : IBM Plex woff2 (Sans+Mono, poids utilisés) et l'extrait Lucide sont importés dans le build Vite (émis dans `dist/assets/`), donc servis par le même mécanisme d'embed — plus besoin de routes axum dédiées aux polices.
- **Routeur client** léger (hash ou history-based, ex. une petite lib ou un routeur maison — tranché au plan) pour les onglets/vues + détails.
- **Client API** typé : un module `src/lib/api.ts` avec les types des réponses `/api/*` et un `fetch` wrapper (gestion erreurs + états loading).

### Build & embed
- `npm run build` (Vite) → **`web/ui/dist/`** (bundle statique : `index.html`, JS/CSS hashés, assets). **`dist/` est commité** dans git.
- Le crate embarque `web/ui/dist/` via **`rust-embed`** (dep optionnelle sous la feature `web`). En **release** le bundle est embarqué dans le binaire ; en **debug**, rust-embed lit `web/ui/dist/` depuis le disque (itération sans recompiler Rust après un `npm run build`).
- CI **100 % Rust** (aucun Node) : `cargo build`/`cargo install` fonctionnent partout car le `dist/` commité est la source d'embed.

### Serving (axum, `src/web/mod.rs`)
- Pendant la migration : route **`/next`** (+ `/next/*`) sert le SPA embarqué (assets par extension/chemin ; fallback `index.html` pour les routes client inconnues — comportement SPA). `/` continue de servir l'ancien `index.html`.
- PR finale : `/` sert le SPA Svelte, l'ancien `index.html` + son `include_str!` sont supprimés, `/next` retiré (ou redirige `/`).
- `/api/*` et le reste de `serve()` inchangés.

### Dépôt / conventions
- `web/ui/` : `package.json`, `vite.config.ts`, `svelte.config.js`, `tsconfig.json`, `src/` (composants, routes, lib), `dist/` (**commité**).
- `.gitignore` : ignorer `web/ui/node_modules/`, NE PAS ignorer `web/ui/dist/`.
- **Contributeurs** : Node requis uniquement pour modifier l'UI (`cd web/ui && npm install && npm run build`, puis commiter `dist/`). Dev rapide : `npm run dev` (serveur Vite, proxy `/api` → `armadai web` local). Discipline « rebuild+commit dist » via un hook git local documenté (pas de Node en CI). Risque assumé : un `dist/` périmé si on oublie le rebuild — atténué par le hook + la doc.

## Découpage (une PR + revue indépendante + validation visuelle Dimitri par lot)

- **W1 — Fondations** : scaffold `web/ui/` (Svelte/Vite/TS), pipeline build + embed (`rust-embed`) + route axum `/next`, feuille de tokens DS + IBM Plex + icônes, **shell** (topbar/wordmark laiton, nav, toggle thème dark/light), routeur client + client API typé, et **2 vues phares** (Agents + History) de bout en bout. Livrable : `/next` affiche le shell + Agents + History fonctionnels ; `/` inchangé.
- **W2…Wn** : porter les vues restantes par paquets — Prompts, Skills, Starters (+ download config), Costs (jauges), Models, vues détail, trace/topologie orchestration (mermaid), rendu markdown (marked). Fidélité composants DS (AgentCard, StatusBadge, Gauge, EventStream, DataTable, FormationDiagram) + états loading/empty/error au fil des vues.
- **W-final** : bascule `/` → SPA Svelte, suppression de l'ancien `index.html`, nettoyage des routes.

## Validation & tests
- **Visuel** : preview Artifact par vue (pendant le dev) + `armadai web` local (route `/next`) ; **validation Dimitri avant chaque merge** (changement user-facing).
- **Tests Rust** : la route `/next` sert le SPA (`GET /next` → 200 + `text/html` ; un asset connu → 200 + bon content-type ; route client inconnue → fallback index). Style des tests d'endpoints existants (handlers appelés directement, `axum::body::to_bytes`).
- **Tests front** : `vitest` léger sur la logique pure (client API, formatage des nombres/dates) ; le rendu visuel n'est pas testé par snapshot (validation manuelle).
- **CI** : clippy 3 modes (`tui`, `tui,providers-api`, `tui,web,storage`) `-D warnings` + `cargo fmt -- --check` + `cargo test` — Rust seul.

## Hors périmètre
- Nouvelle vue « tableau de bord métriques » de la démo (exigerait de nouveaux endpoints `/api/*` + logique) — item séparé possible après parité.
- Les autres surfaces du chantier design system : TUI (ratatui), sortie CLI, docs/README.

## Risques
- **`dist/` périmé** (source Svelte modifiée sans rebuild) → hook git local + doc ; CI ne le détecte pas (pas de Node) — assumé.
- **Node comme dépendance UI** (pas de build, mais pour éditer l'UI) — acceptable, isolé à `web/ui/`.
- **`oklch()`** : navigateurs modernes uniquement (pas de fallback hex — YAGNI sauf besoin legacy).
- **Double UI temporaire** (`/` ancien + `/next` neuf) pendant la migration — coût maîtrisé, résolu par la PR finale.
