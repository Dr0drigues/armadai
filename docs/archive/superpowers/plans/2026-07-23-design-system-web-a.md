# Design System → Web UI, sous-lot A (socle + shell) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ré-habiller le shell de la SPA web embarquée (`src/web/index.html`) à l'identité « pont de commandement » — polices IBM Plex + icônes Lucide self-hosted (routes axum), couche tokens oklch light+dark — sans toucher la logique JS ni `api.rs`.

**Architecture:** Nouveau module `src/web/assets.rs` sert les woff2 + `icons.js` embarqués (`include_bytes!`) via des routes axum same-origin. `index.html` reçoit une couche de tokens (custom properties traduites des `tokens/*.css` du DS) en **remappant les variables existantes** (`--bg`, `--surface`, `--accent`…) sur la palette DS, plus les tokens riches (`--brass`, `--signal-*`, familles/échelle typo, espacement). Le shell (header/wordmark, nav, layout) est re-stylé et les icônes Lucide branchées.

**Tech Stack:** Rust edition 2024, axum 0.8, tower-http 0.7, HTML/CSS (oklch), IBM Plex woff2, Lucide (extrait ISC).

## Global Constraints

- Web gated `--features web`. Gate CI : clippy 3 modes (`--features tui`, `tui,providers-api`, `tui,web,storage`) `-D warnings` + `cargo fmt -- --check` + `cargo test`.
- **Aucune modification** de `src/web/api.rs` ni de la logique JS (fetch/rendu/onglets/markdown/downloads) de `index.html`. On retravaille CSS + DOM du shell + on ajoute les icônes.
- Binaire self-contained : les assets sont embarqués via `include_bytes!`/`include_str!`, servis same-origin (aucune contrainte CSP côté app).
- Theme-aware : dark par défaut, light via `@media (prefers-color-scheme: light)` ET override `data-theme` (le toggle existant de la SPA stampe `data-theme` sur `<html>`).
- Tokens = source `tokens/{colors,typography,spacing}.css` du DS (projet Claude Design `416749e1-6cb2-460b-be6d-6bb2df3ddc6d`), valeurs oklch exactes.
- **Hors périmètre (→ Web-B)** : fidélité composants (Gauge/StatusBadge/EventStream/DataTable/AgentCard) + états loading/empty/error. **Hors Web-A aussi** : nouvelle vue « tableau de bord métriques » (celle de la démo) — exigerait un nouvel endpoint + JS ; Web-A re-skine les vues EXISTANTES.
- Changement user-facing (visuel) → PR + revue indépendante + **validation visuelle Dimitri avant merge** (Artifact preview déjà validé + `armadai web` local).

---

### Task 1: Servir les assets (polices + icônes) via routes axum

**Files:**
- Create: `src/web/assets.rs`
- Create: `src/web/assets/IBMPlexSans-400.woff2`, `IBMPlexSans-600.woff2`, `IBMPlexMono-400.woff2`, `IBMPlexMono-600.woff2`, `icons.js` (récupérés depuis Claude Design)
- Modify: `src/web/mod.rs` (déclarer `mod assets;` + 2 routes)

**Interfaces:**
- Produces :
  - `pub async fn serve_font(axum::extract::Path<String>) -> axum::response::Response`
  - `pub async fn serve_icons() -> axum::response::Response`

- [ ] **Step 1 : Récupérer les binaires du design system**

Via l'outil DesignSync (`get_file`, `projectId: 416749e1-6cb2-460b-be6d-6bb2df3ddc6d`), récupérer et écrire dans `src/web/assets/` :
- `assets/fonts/IBMPlexSans-400.woff2` → `src/web/assets/IBMPlexSans-400.woff2`
- `assets/fonts/IBMPlexSans-600.woff2` → `src/web/assets/IBMPlexSans-600.woff2`
- `assets/fonts/IBMPlexMono-400.woff2` → `src/web/assets/IBMPlexMono-400.woff2`
- `assets/fonts/IBMPlexMono-600.woff2` → `src/web/assets/IBMPlexMono-600.woff2`
- `assets/icons.js` → `src/web/assets/icons.js`

Les woff2 reviennent en base64 (`isBase64: true`) → décoder en octets avant d'écrire. `icons.js` est du texte. Vérifier que chaque fichier fait > 0 octet.

- [ ] **Step 2 : Écrire le test qui échoue** (`src/web/assets.rs`, module `#[cfg(test)]`)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::Path;
    use axum::http::{header, StatusCode};

    async fn body_len(resp: axum::response::Response) -> usize {
        axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap()
            .len()
    }

    #[tokio::test]
    async fn serves_known_font_with_woff2_content_type() {
        let resp = serve_font(Path("IBMPlexSans-400.woff2".to_string())).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers()[header::CONTENT_TYPE], "font/woff2");
        assert!(body_len(resp).await > 0);
    }

    #[tokio::test]
    async fn unknown_font_is_404() {
        let resp = serve_font(Path("nope.woff2".to_string())).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn serves_icons_js() {
        let resp = serve_icons().await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers()[header::CONTENT_TYPE], "text/javascript");
        assert!(body_len(resp).await > 0);
    }
}
```

- [ ] **Step 3 : Vérifier que ça échoue** — `cargo test --no-default-features --features tui,web,storage web::assets` → FAIL (module/fns absents).

- [ ] **Step 4 : Implémenter** (`src/web/assets.rs`)

```rust
//! Self-hosted static assets (fonts, icons) for the web UI, embedded in the
//! binary and served same-origin so the app carries no external dependency.

use axum::extract::Path;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};

static SANS_400: &[u8] = include_bytes!("assets/IBMPlexSans-400.woff2");
static SANS_600: &[u8] = include_bytes!("assets/IBMPlexSans-600.woff2");
static MONO_400: &[u8] = include_bytes!("assets/IBMPlexMono-400.woff2");
static MONO_600: &[u8] = include_bytes!("assets/IBMPlexMono-600.woff2");
static ICONS_JS: &str = include_str!("assets/icons.js");

const IMMUTABLE: &str = "public, max-age=31536000, immutable";

/// Serve an embedded IBM Plex woff2 by filename (`GET /assets/fonts/{file}`).
pub async fn serve_font(Path(file): Path<String>) -> Response {
    let bytes: Option<&'static [u8]> = match file.as_str() {
        "IBMPlexSans-400.woff2" => Some(SANS_400),
        "IBMPlexSans-600.woff2" => Some(SANS_600),
        "IBMPlexMono-400.woff2" => Some(MONO_400),
        "IBMPlexMono-600.woff2" => Some(MONO_600),
        _ => None,
    };
    match bytes {
        Some(b) => (
            [
                (header::CONTENT_TYPE, "font/woff2"),
                (header::CACHE_CONTROL, IMMUTABLE),
            ],
            b,
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// Serve the embedded Lucide-derived icon script (`GET /assets/icons.js`).
pub async fn serve_icons() -> Response {
    (
        [
            (header::CONTENT_TYPE, "text/javascript"),
            (header::CACHE_CONTROL, IMMUTABLE),
        ],
        ICONS_JS,
    )
        .into_response()
}
```

- [ ] **Step 5 : Brancher les routes** (`src/web/mod.rs`)

Ajouter `mod assets;` en haut, et dans le `Router` de `serve()` (après la route `/`) :
```rust
        .route("/assets/fonts/{file}", get(assets::serve_font))
        .route("/assets/icons.js", get(assets::serve_icons))
```

- [ ] **Step 6 : Vérifier vert** — `cargo test --no-default-features --features tui,web,storage web::assets` → PASS (3 tests).

- [ ] **Step 7 : Commit**
```bash
git add src/web/assets.rs src/web/assets/ src/web/mod.rs
git commit -m "feat(web): serve self-hosted IBM Plex fonts and Lucide icons via axum"
```

---

### Task 2: Couche tokens + polices dans `index.html`

**Files:**
- Modify: `src/web/index.html` (le bloc `<style>` : remplacer le `:root`/`[data-theme="light"]` existant ; ajouter `@font-face`)

**Interfaces:**
- Consumes : routes `/assets/fonts/*` (Task 1).
- Produces : variables CSS DS disponibles pour le shell (Task 3) — `--bg-base`, `--surface-1..3`, `--border`, `--text-primary/secondary/muted/faint`, `--brass`, `--accent`, `--signal-{ok,warning,critical,running,halted}(-bg/-fg)`, `--font-ui`, `--font-mono`, `--text-2xs..3xl`, `--space-*`, `--panel-pad`, `--row-h`, `--sidebar-w`, `--focus-ring`. Les variables héritées (`--bg`, `--surface`, `--border`, `--text`, `--muted`, `--accent`, `--green`, `--yellow`) sont **remappées** sur la palette DS pour que les règles existantes adoptent la nouvelle identité sans réécriture.

- [ ] **Step 1 : Remplacer le bloc de variables** — dans `<style>`, remplacer :
```css
  :root { --bg: #0d1117; --surface: #161b22; --border: #30363d; --text: #e6edf3; --muted: #8b949e; --accent: #58a6ff; --green: #3fb950; --yellow: #d29922; }
  [data-theme="light"] { --bg: #ffffff; --surface: #f6f8fa; --border: #d0d7de; --text: #1f2328; --muted: #656d76; --accent: #0969da; --green: #1a7f37; --yellow: #9a6700; }
```
par le bloc DS (dark défaut + light + `@media prefers-color-scheme`), avec les tokens complets ET le remap des variables héritées. Valeurs exactes tirées de `tokens/colors.css` :

```css
  :root, [data-theme="dark"] {
    color-scheme: dark;
    --bg-abyss: oklch(0.150 0.028 248); --bg-base: oklch(0.188 0.030 248);
    --surface-1: oklch(0.223 0.031 247); --surface-2: oklch(0.258 0.032 246); --surface-3: oklch(0.305 0.033 245);
    --border-c: oklch(0.360 0.030 244); --border-strong: oklch(0.470 0.034 243); --border-faint: oklch(0.280 0.028 246);
    --text-primary: oklch(0.955 0.008 240); --text-secondary: oklch(0.790 0.015 240);
    --text-muted-c: oklch(0.635 0.020 242); --text-faint: oklch(0.505 0.020 244); --text-on-accent: oklch(0.180 0.030 248);
    --brass: oklch(0.790 0.100 84); --brass-strong: oklch(0.855 0.110 86); --brass-dim: oklch(0.660 0.086 82);
    --brass-bg: oklch(0.300 0.045 82); --brass-border: oklch(0.500 0.070 82);
    --signal-ok: oklch(0.760 0.150 152); --signal-warning: oklch(0.820 0.155 70); --signal-critical: oklch(0.660 0.190 25);
    --signal-running: oklch(0.730 0.130 226); --signal-halted: oklch(0.620 0.018 244);
    --signal-ok-bg: oklch(0.320 0.070 152); --signal-warning-bg: oklch(0.340 0.070 70); --signal-critical-bg: oklch(0.320 0.085 25);
    --signal-running-bg: oklch(0.320 0.065 226); --signal-halted-bg: oklch(0.300 0.014 244);
    --signal-ok-fg: oklch(0.860 0.160 152); --signal-warning-fg: oklch(0.880 0.160 72); --signal-critical-fg: oklch(0.800 0.170 25);
    --signal-running-fg: oklch(0.840 0.130 226); --signal-halted-fg: oklch(0.760 0.016 244);
    --focus-ring: oklch(0.790 0.100 84 / 0.55);
    /* remap des variables héritées → palette DS */
    --bg: var(--bg-base); --surface: var(--surface-1); --border: var(--border-c);
    --text: var(--text-primary); --muted: var(--text-muted-c); --accent: var(--brass);
    --green: var(--signal-ok-fg); --yellow: var(--signal-warning-fg);
  }
  [data-theme="light"] {
    color-scheme: light;
    --bg-abyss: oklch(0.928 0.012 240); --bg-base: oklch(0.966 0.008 236);
    --surface-1: oklch(0.996 0.003 236); --surface-2: oklch(0.980 0.006 237); --surface-3: oklch(0.944 0.010 238);
    --border-c: oklch(0.868 0.013 240); --border-strong: oklch(0.770 0.022 242); --border-faint: oklch(0.910 0.010 239);
    --text-primary: oklch(0.245 0.036 248); --text-secondary: oklch(0.400 0.030 246);
    --text-muted-c: oklch(0.520 0.026 245); --text-faint: oklch(0.630 0.022 244); --text-on-accent: oklch(0.985 0.004 236);
    --brass: oklch(0.560 0.110 78); --brass-strong: oklch(0.475 0.108 76); --brass-dim: oklch(0.660 0.090 82);
    --brass-bg: oklch(0.930 0.045 84); --brass-border: oklch(0.780 0.075 82);
    --signal-ok: oklch(0.520 0.150 152); --signal-warning: oklch(0.560 0.140 66); --signal-critical: oklch(0.535 0.195 27);
    --signal-running: oklch(0.520 0.130 232); --signal-halted: oklch(0.560 0.018 244);
    --signal-ok-bg: oklch(0.945 0.050 152); --signal-warning-bg: oklch(0.950 0.055 70); --signal-critical-bg: oklch(0.948 0.050 27);
    --signal-running-bg: oklch(0.945 0.045 232); --signal-halted-bg: oklch(0.935 0.010 244);
    --signal-ok-fg: oklch(0.440 0.140 152); --signal-warning-fg: oklch(0.470 0.125 62); --signal-critical-fg: oklch(0.470 0.185 27);
    --signal-running-fg: oklch(0.445 0.125 232); --signal-halted-fg: oklch(0.480 0.016 244);
    --focus-ring: oklch(0.560 0.110 78 / 0.45);
    --bg: var(--bg-base); --surface: var(--surface-1); --border: var(--border-c);
    --text: var(--text-primary); --muted: var(--text-muted-c); --accent: var(--brass);
    --green: var(--signal-ok-fg); --yellow: var(--signal-warning-fg);
  }
  @media (prefers-color-scheme: light) {
    :root:not([data-theme="dark"]) {
      color-scheme: light;
      --bg-base: oklch(0.966 0.008 236); --surface-1: oklch(0.996 0.003 236); --surface-2: oklch(0.980 0.006 237); --surface-3: oklch(0.944 0.010 238);
      --border-c: oklch(0.868 0.013 240); --border-strong: oklch(0.770 0.022 242); --border-faint: oklch(0.910 0.010 239);
      --text-primary: oklch(0.245 0.036 248); --text-secondary: oklch(0.400 0.030 246); --text-muted-c: oklch(0.520 0.026 245); --text-faint: oklch(0.630 0.022 244);
      --brass: oklch(0.560 0.110 78); --brass-strong: oklch(0.475 0.108 76); --brass-bg: oklch(0.930 0.045 84); --brass-border: oklch(0.780 0.075 82);
      --signal-ok-fg: oklch(0.440 0.140 152); --signal-warning-fg: oklch(0.470 0.125 62); --focus-ring: oklch(0.560 0.110 78 / 0.45);
      --bg: var(--bg-base); --surface: var(--surface-1); --border: var(--border-c); --text: var(--text-primary); --muted: var(--text-muted-c); --accent: var(--brass); --green: var(--signal-ok-fg); --yellow: var(--signal-warning-fg);
    }
  }
```

Puis ajouter, dans le même `<style>`, les tokens typo/espacement + `@font-face` (au début du bloc) :
```css
  @font-face { font-family:"IBM Plex Sans"; font-weight:400; font-display:swap; src:url("/assets/fonts/IBMPlexSans-400.woff2") format("woff2"); }
  @font-face { font-family:"IBM Plex Sans"; font-weight:600; font-display:swap; src:url("/assets/fonts/IBMPlexSans-600.woff2") format("woff2"); }
  @font-face { font-family:"IBM Plex Mono"; font-weight:400; font-display:swap; src:url("/assets/fonts/IBMPlexMono-400.woff2") format("woff2"); }
  @font-face { font-family:"IBM Plex Mono"; font-weight:600; font-display:swap; src:url("/assets/fonts/IBMPlexMono-600.woff2") format("woff2"); }
  :root {
    --font-ui:"IBM Plex Sans", ui-sans-serif, system-ui, -apple-system, "Segoe UI", sans-serif;
    --font-mono:"IBM Plex Mono", ui-monospace, "SF Mono", Menlo, monospace;
    --text-2xs:10px; --text-xs:11px; --text-sm:12px; --text-base:13px; --text-md:14px; --text-lg:16px; --text-xl:19px; --text-2xl:23px; --text-3xl:28px;
    --tracking-caps:0.14em; --tracking-wide:0.04em;
    --space-4:8px; --space-5:12px; --space-6:16px; --space-8:24px; --panel-pad:16px; --row-h:36px; --sidebar-w:240px;
  }
```

- [ ] **Step 2 : Appliquer la famille UI au body** — dans la règle `body { … }`, remplacer `font-family: -apple-system, …` par `font-family: var(--font-ui);` et `font-size: var(--text-base);`.

- [ ] **Step 3 : Vérifier que la page se sert toujours** — le contenu HTML est statique ; vérifier qu'aucun test existant ne casse et que le fichier reste du HTML valide (pas de test unitaire ici — validé visuellement + par le build). Run : `cargo build --no-default-features --features tui,web,storage` → OK.

- [ ] **Step 4 : Commit**
```bash
git add src/web/index.html
git commit -m "feat(web): translate the design-system tokens (oklch, IBM Plex) into the web UI"
```

---

### Task 3: Re-skin du shell (header/wordmark, nav, layout, icônes, chiffres)

**Files:**
- Modify: `src/web/index.html` (règles CSS du shell + markup header/nav + chargement `icons.js` + points d'icônes)

**Interfaces:**
- Consumes : tokens (Task 2), route `/assets/icons.js` + `window.ArmadaiIcons` (Task 1).

- [ ] **Step 1 : Header + wordmark** — re-styler `header` (fond `var(--surface-2)`, bordure `var(--border-c)`), le titre en wordmark « ARMAD**AI** » avec une petite marque laiton (carré dégradé `--brass-strong`→`--brass-dim`, `--text-on-accent`), et un tag « pont de commandement » en `.eyebrow` (caps, `--text-faint`, `--tracking-caps`). Réutiliser le markup existant de `<header>` (ne pas casser les hooks JS de nav).

- [ ] **Step 2 : Navigation** — `nav button.active` : fond `var(--brass-bg)`, couleur `var(--brass-strong)`, poids 600, + un rail laiton à gauche (`::before`). `nav button:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: 1px; }`.

- [ ] **Step 3 : Surfaces & chiffres** — `.card`/`.card-header` sur `--surface-1`/`--border-c` ; ajouter `font-variant-numeric: tabular-nums; font-family: var(--font-mono);` sur les cellules numériques (colonnes coût/tokens/durée — cibler les `td` de nombres ou ajouter une classe `.num` sur les colonnes chiffrées d'History/Costs sans toucher le JS de remplissage : styler via `th`/`td` nth-child si le JS ne pose pas de classe — sinon garder simple, styler `table td` en `--font-mono` pour les vues History/Costs uniquement). Les eyebrows de `th` : `letter-spacing: var(--tracking-caps)`.

- [ ] **Step 4 : Icônes Lucide** — charger le script juste avant la fin du `<body>` : `<script src="/assets/icons.js"></script>`. Aux points clés du shell (boutons de nav, en-têtes de section), injecter les icônes via `window.ArmadaiIcons` (suivre l'API exposée par `icons.js` — inspecter le fichier récupéré en Task 1 ; typiquement une fonction qui remplace des `<i data-icon="agents">` par du SVG). Ne pas bloquer si l'API diffère : dégrader proprement (les libellés texte restent). Ne pas toucher mermaid/marked (déjà chargés en CDN — hors périmètre).

- [ ] **Step 5 : Vérifier build + servir** — `cargo build --no-default-features --features tui,web,storage` → OK. Lancer `cargo run --bin armadai --no-default-features --features tui,web,storage -- web` et vérifier à la main que la page charge, que le toggle light/dark marche, que les polices IBM Plex chargent (onglet réseau : `/assets/fonts/*` en 200), que l'onglet actif est en laiton.

- [ ] **Step 6 : Commit**
```bash
git add src/web/index.html
git commit -m "feat(web): reskin the shell (wordmark, nav, surfaces, icons) to the command-bridge identity"
```

---

## Self-Review
- **Couverture spec** : assets self-hosted routes (Task 1) ✓ ; couche tokens oklch light+dark (Task 2) ✓ ; @font-face IBM Plex (Task 2) ✓ ; re-skin shell header/wordmark/nav/focus/tabular-nums/icônes (Task 3) ✓ ; JS/api inchangés (contrainte respectée — seules CSS + markup shell + une balise script) ✓. Hors périmètre (composants/états/nouvelle vue métriques) explicitement exclu.
- **Placeholders** : aucun ; valeurs oklch exactes copiées de `tokens/colors.css`. L'API précise de `icons.js` (Task 3 Step 4) est à lire dans le fichier récupéré — dégradation propre spécifiée si elle diffère.
- **Cohérence types** : `serve_font(Path<String>)`/`serve_icons()` → `Response` cohérents entre Task 1 (def) et mod.rs (routes) ; noms de tokens cohérents Task 2 → Task 3.
- **Validation** : Artifact preview déjà validé par Dimitri ; validation finale = `armadai web` local avant merge.
