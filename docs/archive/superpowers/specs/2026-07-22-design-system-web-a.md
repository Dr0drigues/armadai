# Design System → Web UI, sous-lot A (socle + shell + dashboard) — Design

## Contexte

Le design system ArmadAI (« pont de commandement ») est complet sur Claude Design (projet `416749e1-6cb2-460b-be6d-6bb2df3ddc6d`) : c'est une **spec HTML/CSS+JSX à traduire**, pas à importer. Ce chantier applique cette identité aux surfaces réelles ; l'ordre validé est **Web → TUI → CLI → Docs**. Le sous-lot Web est lui-même découpé : **Web-A** (ce spec — socle + shell + dashboard) puis **Web-B** (fidélité composants + états).

La Web UI actuelle (`src/web/index.html`, ~1117 lignes) est une SPA embarquée complète (header/nav, onglets Agents/Prompts/Skills/Starters/History/Costs/Models + vues détail, light mode, markdown, downloads — livrée en v0.12.0). La logique JS et l'API JSON (`src/web/api.rs`) restent **inchangées** ; Web-A retravaille le DOM/CSS et ajoute les icônes.

## Objectif

Appliquer l'identité « pont de commandement » au **shell** de la SPA (header/wordmark, navigation, layout, typographie, couleurs/surfaces/signaux) et à la **vue dashboard principale**, avec polices IBM Plex et icônes Lucide self-hosted, en light + dark.

## Périmètre

**Dans Web-A :**
- Couche tokens (couleurs, espacement, typographie, élévation) traduite du DS en custom properties CSS, light + dark.
- Polices IBM Plex (Sans + Mono, poids sous-setés) servies via routes axum same-origin.
- Icônes Lucide (extrait DS `assets/icons.js`) servies via route ; remplacement des glyphes du shell aux points clés.
- Re-skin du shell : header + wordmark, barre de navigation/onglets, layout général, échelle typographique, fonds/surfaces/bordures, sémantique de signal.
- Re-skin de la vue dashboard principale (liste/landing) aux tokens.

**Hors Web-A (→ Web-B) :** fidélité fine des composants (Gauge, StatusBadge, EventStream, DataTable, AgentCard, FormationDiagram) et états loading/empty/error selon `ui_kits/states`.

**Inchangé :** endpoints `api.rs`, logique JS de fetch/rendu, sémantique des onglets/downloads/markdown.

## Architecture

### 1. Assets self-hosted (routes axum)
- Nouveau module `src/web/assets.rs` : les woff2 IBM Plex (sous-set des poids réellement utilisés — a priori Sans 400/600, Mono 400/600, à ajuster) et l'extrait Lucide `icons.js` sont embarqués dans le binaire via `include_bytes!`.
- `src/web/mod.rs` : routes `GET /assets/fonts/{file}` (content-type `font/woff2`, cache-control long) et `GET /assets/icons.js` (content-type `text/javascript`). Same-origin → aucune contrainte CSP côté app réelle.
- Source DS : `assets/fonts/IBMPlex{Sans,Mono}-{400,500,600,700}.woff2`, `assets/icons.js` (Lucide extrait, ISC, `window.ArmadaiIcons`), récupérés via DesignSync à l'implémentation.

### 2. Couche tokens (dans `index.html`, bloc `<style>`)
Traduire les fichiers DS `tokens/{colors,spacing,typography,elevation,fonts}.css` en `:root` custom properties. La palette **couleurs** (source `tokens/colors.css`, oklch) :
- **Dark par défaut** (`:root, [data-theme="dark"]`, `color-scheme: dark`) — surfaces chart-blue (`--bg-abyss/base`, `--surface-1..3`), texte (`--text-primary/secondary/muted/faint`), accent laiton (`--brass*`, `--accent`), signaux distincts (`--signal-{ok,warning,critical,running,halted}` + `-bg`/`-fg`), data-viz (`--viz-track/grid`).
- **Light** (`[data-theme="light"]`, `color-scheme: light`) — « papier de carte » (cool white), mêmes noms de tokens redéfinis.
- Bascule : `@media (prefers-color-scheme)` pour le défaut OS, override par `data-theme` sur `:root` (le toggle existant de la SPA stampe `data-theme`). Les composants stylent **via les tokens**, jamais en dur dans une media query.
- `oklch()` est supporté par les navigateurs modernes visés ; fallback hex optionnel non requis (à confirmer si support legacy attendu — sinon YAGNI).

### 3. Polices & typographie
- `@font-face` pour IBM Plex Sans (UI) et Mono (télémétrie/données/nombres) pointant `/assets/fonts/…`, `font-display: swap`.
- Familles/échelle depuis `tokens/fonts.css` + `tokens/typography.css` : `--font-sans`, `--font-mono`, échelle de tailles, `font-variant-numeric: tabular-nums` sur les colonnes de chiffres (History/Costs).

### 4. Shell & wordmark
- Header : wordmark « pont de commandement » (réf `components/brand/Wordmark`) — texte + éventuel glyphe, pas d'image binaire si évitable.
- Navigation/onglets : stylés aux tokens (surface-2, accent laiton sur l'onglet actif, focus-ring visible).
- Layout : fonds `--bg-base`/`--surface-1`, bordures `--border`, espacement depuis `tokens/spacing.css`.
- Icônes Lucide aux points clés du shell (onglets, actions) via `window.ArmadaiIcons`.

## Validation visuelle (exigence Dimitri)
- **Dev** : je publie le `index.html` re-skiné (données d'exemple, polices **inline data-URI** pour respecter la CSP Artifact) comme Artifact claude.ai → Dimitri juge identité/layout, itération rapide.
- **Final avant merge** : Dimitri lance `armadai web` en local (vraies données, serving réel via les routes axum) → son go → merge.
- L'Artifact est un **preview jetable** ; l'app réelle sert les polices via axum (pas de data-URI).

## Tests & CI
- Web gated `--features web`. Gate CI : clippy 3 modes (`tui`, `tui,providers-api`, `tui,web,storage`) `-D warnings` + `cargo fmt -- --check` + `cargo test`.
- Tests automatisés : les routes assets renvoient `200` + le bon `content-type` (`font/woff2`, `text/javascript`) et un corps non vide (tests axum, style des tests d'endpoints existants dans `api.rs`/`mod.rs`).
- Le rendu visuel est validé manuellement par Dimitri (pas de snapshot pixel).

## Hors périmètre (autres sous-lots du chantier)
Web-B (composants + états) ; TUI (ratatui, `terminal-palette.json` 256/16 + `--ascii`) ; CLI (sortie humaine live + summary, `armadai view`) ; Docs/README.
