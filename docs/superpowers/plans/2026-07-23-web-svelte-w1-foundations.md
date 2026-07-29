# Refonte Web Svelte — W1 (Fondations + shell + Agents/History) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Scaffolder l'appli Svelte (`web/ui/`), l'embarquer dans le binaire et la servir à `/next`, avec le shell « pont de commandement » (tokens DS, IBM Plex, nav, thème) et deux vues fonctionnelles (Agents, History) — sans toucher `/`, `/api/*` ni la logique existante.

**Architecture:** `web/ui/` = Svelte 5 + Vite + TypeScript, buildé en `web/ui/dist/` (commité), embarqué via `include_dir` (déjà dépendance) et servi par axum à `/next`. Le style porte la démo validée (`<scratchpad>/armadai-web-demo.html`) et les valeurs des `tokens/*.css` du DS.

**Tech Stack:** Svelte 5, Vite, TypeScript, axum 0.8, include_dir 0.7.

## Global Constraints
- **Node uniquement pour l'UI** ; CI + `cargo build`/`cargo install` restent Rust seul (le `dist/` commité est la source d'embed). Gate CI : clippy 3 modes (`tui`, `tui,providers-api`, `tui,web,storage`) `-D warnings` + `cargo fmt -- --check` + `cargo test`.
- **Ne pas modifier** `src/web/api.rs`, la route `/`, ni l'ancien `index.html`. W1 ajoute `/next` en parallèle.
- Feature `web` gate le serving. `include_dir` est non-optionnel (déjà présent).
- Tokens = valeurs oklch exactes des `tokens/colors.css` (dark défaut + light), `tokens/typography.css`, `tokens/spacing.css` du DS. Style/markup du shell + table + badges = port de `<scratchpad>/armadai-web-demo.html` (déjà validé visuellement par Dimitri).
- Source de style/valeurs : le scratchpad session `= /private/tmp/claude-502/-Users-bl209054-work-misc-armadai/545db4a0-c939-4d03-9059-60229ed309bf/scratchpad`.
- Changement user-facing → PR + revue indépendante + **validation visuelle Dimitri** (`armadai web` → `/next`) avant merge.

---

### Task 1: Scaffold Svelte/Vite/TS + build `dist/` commité

**Files:**
- Create: `web/ui/package.json`, `web/ui/vite.config.ts`, `web/ui/svelte.config.js`, `web/ui/tsconfig.json`, `web/ui/index.html`, `web/ui/src/main.ts`, `web/ui/src/App.svelte`
- Create (généré, commité): `web/ui/dist/**`
- Modify: `.gitignore` (ajouter `web/ui/node_modules/`)

**Interfaces:**
- Produces : `web/ui/dist/index.html` + assets — la cible d'embed de Task 2.

- [ ] **Step 1 : `web/ui/package.json`**
```json
{
  "name": "armadai-web-ui",
  "private": true,
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "vite build",
    "check": "svelte-check --tsconfig ./tsconfig.json"
  },
  "devDependencies": {
    "@sveltejs/vite-plugin-svelte": "^5",
    "svelte": "^5",
    "svelte-check": "^4",
    "typescript": "^5",
    "vite": "^6"
  }
}
```

- [ ] **Step 2 : configs** — `web/ui/vite.config.ts` (base `/next/` pour que les assets soient référencés sous `/next/`, proxy `/api` en dev vers `armadai web`) :
```ts
import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

export default defineConfig({
  base: "/next/",
  plugins: [svelte()],
  server: { proxy: { "/api": "http://localhost:8080" } },
  build: { outDir: "dist", emptyOutDir: true },
});
```
`web/ui/svelte.config.js` :
```js
import { vitePreprocess } from "@sveltejs/vite-plugin-svelte";
export default { preprocess: vitePreprocess() };
```
`web/ui/tsconfig.json` :
```json
{
  "compilerOptions": {
    "target": "ESNext", "module": "ESNext", "moduleResolution": "bundler",
    "strict": true, "verbatimModuleSyntax": true, "isolatedModules": true,
    "skipLibCheck": true, "types": ["svelte", "vite/client"]
  },
  "include": ["src/**/*.ts", "src/**/*.svelte"]
}
```
`web/ui/index.html` :
```html
<!doctype html>
<html lang="fr">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>ArmadAI</title>
  </head>
  <body>
    <div id="app"></div>
    <script type="module" src="/src/main.ts"></script>
  </body>
</html>
```

- [ ] **Step 3 : point d'entrée minimal** — `web/ui/src/main.ts` :
```ts
import { mount } from "svelte";
import App from "./App.svelte";

const app = mount(App, { target: document.getElementById("app")! });
export default app;
```
`web/ui/src/App.svelte` (placeholder, remplacé en Task 3) :
```svelte
<main><h1>ArmadAI — /next</h1></main>
```

- [ ] **Step 4 : `.gitignore`** — ajouter la ligne `web/ui/node_modules/` (vérifier que `web/ui/dist/` n'est PAS ignoré).

- [ ] **Step 5 : install + build** — `cd web/ui && npm install && npm run build`. Attendu : `web/ui/dist/index.html` + `web/ui/dist/assets/*` créés, exit 0.

- [ ] **Step 6 : Commit** (dist inclus)
```bash
git add web/ui/package.json web/ui/vite.config.ts web/ui/svelte.config.js web/ui/tsconfig.json web/ui/index.html web/ui/src web/ui/dist .gitignore web/ui/package-lock.json
git commit -m "build(web): scaffold Svelte/Vite/TS app under web/ui with committed dist"
```

---

### Task 2: Embarquer `dist/` et servir `/next` (Rust, TDD)

**Files:**
- Modify: `src/web/mod.rs`

**Interfaces:**
- Consumes : `web/ui/dist/` (Task 1).
- Produces : `pub async fn serve_next(axum::extract::Path<String>) -> axum::response::Response` + `async fn serve_next_root() -> Response`.

- [ ] **Step 1 : Écrire le test qui échoue** (module `#[cfg(test)]` dans `src/web/mod.rs`)
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::Path;
    use axum::http::{header, StatusCode};

    async fn parts(resp: axum::response::Response) -> (StatusCode, String, usize) {
        let status = resp.status();
        let ct = resp.headers().get(header::CONTENT_TYPE)
            .map(|v| v.to_str().unwrap().to_string()).unwrap_or_default();
        let len = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap().len();
        (status, ct, len)
    }

    #[tokio::test]
    async fn next_root_serves_html() {
        let (status, ct, len) = parts(serve_next_root().await).await;
        assert_eq!(status, StatusCode::OK);
        assert!(ct.starts_with("text/html"));
        assert!(len > 0);
    }

    #[tokio::test]
    async fn unknown_client_route_falls_back_to_index_html() {
        // A path with no file extension = client route → SPA fallback (index.html).
        let (status, ct, _) = parts(serve_next(Path("agents".to_string())).await).await;
        assert_eq!(status, StatusCode::OK);
        assert!(ct.starts_with("text/html"));
    }
}
```

- [ ] **Step 2 : Vérifier l'échec** — `cargo test --no-default-features --features tui,web,storage web::tests` → FAIL (fns absentes).

- [ ] **Step 3 : Implémenter** — dans `src/web/mod.rs`, ajouter l'embed + les handlers + les routes.

En tête du fichier :
```rust
use include_dir::{Dir, include_dir};

static WEB_DIST: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/web/ui/dist");

/// Guess a content-type from a path extension (the few types the SPA emits).
fn content_type_for(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json",
        Some("woff2") => "font/woff2",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        _ => "application/octet-stream",
    }
}

fn index_html_response() -> axum::response::Response {
    use axum::response::IntoResponse;
    let body = WEB_DIST
        .get_file("index.html")
        .map(|f| f.contents())
        .unwrap_or(b"<!doctype html><title>ArmadAI</title>");
    ([(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")], body).into_response()
}

/// `GET /next` — serve the SPA entrypoint.
async fn serve_next_root() -> axum::response::Response {
    index_html_response()
}

/// `GET /next/{*path}` — serve an embedded asset by path, or fall back to the
/// SPA `index.html` for client-side routes (paths without a file extension or
/// not found), so deep links into the SPA work.
pub async fn serve_next(axum::extract::Path(path): axum::extract::Path<String>) -> axum::response::Response {
    use axum::response::IntoResponse;
    match WEB_DIST.get_file(&path) {
        Some(f) => (
            [(axum::http::header::CONTENT_TYPE, content_type_for(&path))],
            f.contents(),
        )
            .into_response(),
        None => index_html_response(),
    }
}
```
Puis dans le `Router` de `serve()`, après la route `/` :
```rust
        .route("/next", get(serve_next_root))
        .route("/next/", get(serve_next_root))
        .route("/next/{*path}", get(serve_next))
```

- [ ] **Step 4 : Vérifier vert** — `cargo test --no-default-features --features tui,web,storage web::tests` → PASS (2 tests).

- [ ] **Step 5 : clippy 3 modes + fmt** — les 3 modes `-D warnings` + `cargo fmt -- --check` propres.

- [ ] **Step 6 : Commit**
```bash
git add src/web/mod.rs
git commit -m "feat(web): embed and serve the Svelte SPA at /next (parallel to legacy /)"
```

---

### Task 3: Tokens DS + shell (Svelte)

**Files:**
- Create: `web/ui/src/tokens.css`, `web/ui/src/lib/theme.svelte.ts`, `web/ui/src/lib/Shell.svelte`
- Create: `web/ui/src/assets/IBMPlex{Sans,Mono}-{400,600}.woff2`, `web/ui/src/assets/icons.js` (récupérés du DS)
- Modify: `web/ui/src/App.svelte`, `web/ui/src/main.ts` (importer `tokens.css`)
- Regénère + commite `web/ui/dist/`

**Interfaces:**
- Produces : composant `Shell.svelte` (topbar + nav + slot de contenu + toggle thème) consommé par `App.svelte`.

- [ ] **Step 1 : Récupérer les assets du DS** — via DesignSync (`get_file`, projet `416749e1-6cb2-460b-be6d-6bb2df3ddc6d`), écrire dans `web/ui/src/assets/` : `assets/fonts/IBMPlex{Sans,Mono}-{400,600}.woff2` (base64 → octets) et `assets/icons.js`.

- [ ] **Step 2 : `web/ui/src/tokens.css`** — porter le bloc `<style>` de `<scratchpad>/armadai-web-demo.html` : les `:root,[data-theme="dark"]`, `[data-theme="light"]`, `@media (prefers-color-scheme: light)` (valeurs oklch exactes de `tokens/colors.css`), les tokens typo/espacement, plus `@font-face` pointant les polices importées :
```css
@font-face { font-family:"IBM Plex Sans"; font-weight:400; font-display:swap; src:url("./assets/IBMPlexSans-400.woff2") format("woff2"); }
@font-face { font-family:"IBM Plex Sans"; font-weight:600; font-display:swap; src:url("./assets/IBMPlexSans-600.woff2") format("woff2"); }
@font-face { font-family:"IBM Plex Mono"; font-weight:400; font-display:swap; src:url("./assets/IBMPlexMono-400.woff2") format("woff2"); }
@font-face { font-family:"IBM Plex Mono"; font-weight:600; font-display:swap; src:url("./assets/IBMPlexMono-600.woff2") format("woff2"); }
:root { --font-ui:"IBM Plex Sans", ui-sans-serif, system-ui, sans-serif; --font-mono:"IBM Plex Mono", ui-monospace, Menlo, monospace; }
html, body { margin:0; background: var(--bg-base); color: var(--text-primary); font-family: var(--font-ui); font-size: var(--text-base); }
```
(Reprendre les valeurs `--text-*`, `--space-*`, `--panel-pad`, `--row-h`, `--sidebar-w` de la démo.)

- [ ] **Step 3 : store de thème** — `web/ui/src/lib/theme.svelte.ts` :
```ts
class Theme {
  value = $state<"dark" | "light">("dark");
  toggle() {
    this.value = this.value === "dark" ? "light" : "dark";
    document.documentElement.setAttribute("data-theme", this.value);
  }
}
export const theme = new Theme();
```

- [ ] **Step 4 : `Shell.svelte`** — porter le markup/CSS du shell de la démo (topbar + wordmark laiton « ARMAD**AI** » + tag « pont de commandement » + toggle thème ; sidebar nav Flotte/Télémétrie avec item actif en laiton + focus-ring). Props : `tabs: {id, label, count?}[]`, `active: string`, callback `onselect(id)`. Un `<slot />` pour le contenu de la vue. Styles `<style>` scopés (repris de la démo). Utiliser `theme.toggle()` sur le bouton.

- [ ] **Step 5 : `App.svelte`** — état `let active = $state("agents")` ; liste des onglets ; `<Shell {tabs} {active} onselect={...}>` avec, pour l'instant, un contenu placeholder par onglet (les vraies vues arrivent en Task 4). Importer `tokens.css` dans `main.ts` (`import "./tokens.css";`).

- [ ] **Step 6 : build + vérif** — `cd web/ui && npm run check && npm run build` (exit 0, pas d'erreur type). Lancer `cargo run --bin armadai --no-default-features --features tui,web,storage -- web`, ouvrir `/next` : le shell « pont de commandement » s'affiche, le toggle dark/light marche, IBM Plex charge.

- [ ] **Step 7 : Commit** (dist regénéré)
```bash
git add web/ui/src web/ui/dist
git commit -m "feat(web): command-bridge shell (DS tokens, IBM Plex, nav, theme) in Svelte"
```

---

### Task 4: Client API typé + vues Agents & History

**Files:**
- Create: `web/ui/src/lib/api.ts`, `web/ui/src/views/Agents.svelte`, `web/ui/src/views/History.svelte`
- Modify: `web/ui/src/App.svelte` (brancher les vues)
- Test: `web/ui/src/lib/api.test.ts` (vitest — logique de formatage)
- Regénère + commite `web/ui/dist/`

**Interfaces:**
- Consumes : `Shell.svelte` (Task 3), endpoints `/api/agents` (`AgentSummary[]`), `/api/history` (`HistoryEntry[]`).
- Produces : `getAgents(): Promise<AgentSummary[]>`, `getHistory(): Promise<HistoryEntry[]>`, `fmtTokens(n)`, `fmtCost(n)`.

- [ ] **Step 1 : Écrire le test vitest qui échoue** — d'abord ajouter `vitest` aux devDependencies de `web/ui/package.json` + script `"test": "vitest run"`. Puis `web/ui/src/lib/api.test.ts` :
```ts
import { describe, it, expect } from "vitest";
import { fmtCost, fmtTokens } from "./api";

describe("formatters", () => {
  it("formats cost with 2 decimals and $", () => { expect(fmtCost(18.7)).toBe("$18.70"); });
  it("groups token counts", () => { expect(fmtTokens(184210)).toBe("184 210"); });
});
```

- [ ] **Step 2 : Vérifier l'échec** — `cd web/ui && npm install && npm run test` → FAIL (module/fns absents).

- [ ] **Step 3 : `api.ts`** — types alignés sur `src/web/api.rs` (`AgentSummary`, `HistoryEntry`) + fetch + formatteurs :
```ts
export interface AgentSummary { name: string; provider: string; model: string; tags: string[]; stacks: string[]; scope: string[]; model_fallback: string[]; }
export interface HistoryEntry { agent: string; provider: string; model: string; tokens_in: number; tokens_out: number; cost: number; duration_ms: number; status: string; }

async function getJson<T>(path: string): Promise<T> {
  const r = await fetch(path);
  if (!r.ok) throw new Error(`${path}: ${r.status}`);
  return (await r.json()) as T;
}
export const getAgents = () => getJson<AgentSummary[]>("/api/agents");
export const getHistory = () => getJson<HistoryEntry[]>("/api/history");

export const fmtCost = (n: number) => `$${n.toFixed(2)}`;
export const fmtTokens = (n: number) => n.toLocaleString("fr-FR").replace(/ | /g, " ");
```

- [ ] **Step 4 : Vérifier vert** — `npm run test` → PASS.

- [ ] **Step 5 : Vues** — `Agents.svelte` : `let agents = $state<AgentSummary[]>([])`, charge via `getAgents()` (`$effect` ou `onMount`), rend une liste de cartes agent (avatar initiales, nom, provider·model en mono, tags) — style repris de `.agent` dans la démo. `History.svelte` : charge `getHistory()`, rend une table dense (colonnes agent / provider·model / état badge / tokens (mono, tabular-nums) / coût (mono) / durée) — style repris de `table`/`.badge` de la démo, `fmtTokens`/`fmtCost`. Gérer un état de chargement minimal (« … » — les états riches sont Web-B/plus tard). Mapper `status` → classe badge (`success`→ok, `running`→running, `halted`→halted, autre→warning).

- [ ] **Step 6 : Brancher dans `App.svelte`** — remplacer les placeholders : `{#if active === "agents"}<Agents />{:else if active === "history"}<History />{:else}<div class="panel">Vue « {active} » — à venir (W2).</div>{/if}`.

- [ ] **Step 7 : build + validation** — `npm run check && npm run test && npm run build` (exit 0). `cargo run --bin armadai --no-default-features --features tui,web,storage -- web`, ouvrir `/next` : Agents liste les agents réels, History la table des runs, avec l'identité DS. **Point de validation visuelle Dimitri.**

- [ ] **Step 8 : Commit** (dist regénéré)
```bash
git add web/ui/package.json web/ui/package-lock.json web/ui/src web/ui/dist
git commit -m "feat(web): typed API client + Agents and History views (Svelte)"
```

---

## Self-Review
- **Couverture spec** : scaffold Svelte/Vite/TS (Task 1) ✓ ; embed include_dir + serving `/next` + fallback SPA (Task 2, TDD) ✓ ; tokens DS + IBM Plex + shell + thème (Task 3) ✓ ; client API typé + Agents + History (Task 4) ✓ ; `/`, `/api/*`, ancien index.html intouchés ✓ ; dist commité, CI Rust seule (include_dir déjà dep) ✓. Écart assumé vs spec : `include_dir` (déjà présent) au lieu de `rust-embed` — une dep de moins ; dev loop = serveur Vite + proxy (Step vite.config proxy) au lieu du read-disk de rust-embed.
- **Placeholders** : configs, code Rust (embed+routes+tests), api.ts, tests vitest = complets. Les composants Svelte (Shell/Agents/History) référencent la démo validée `<scratchpad>/armadai-web-demo.html` comme source exacte de markup/CSS — pas un placeholder, une source concrète.
- **Cohérence types** : `serve_next(Path<String>)`/`serve_next_root()` → `Response` (Task 2) ; `AgentSummary`/`HistoryEntry` alignés `api.rs` ↔ `api.ts` (Task 4) ; `Shell` props cohérentes Task 3 → Task 4.
- **Validation** : `armadai web` → `/next`, validation Dimitri avant merge.
