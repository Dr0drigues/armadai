# Design System → Docs Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give the ArmadAI docs a "pont de commandement" design-system identity: a standalone brand asset, an identity-forward README, and a themed mdBook site published to GitHub Pages.

**Architecture:** Three independent sub-lots, one PR each. D1 = brand SVGs + README landing (no build tooling). D2 = mdBook site reusing `docs/wiki/*.md` as-is with a DS-themed `docs/theme/` (oklch tokens + self-hosted IBM Plex). D3 = GitHub Pages CI on master. Docs are not Rust code, so "tests" are `mdbook build` cleanliness, font-file presence, and visual validation — not unit tests.

**Tech Stack:** mdBook (Rust), SVG, CSS custom properties (oklch), IBM Plex woff2 (`@fontsource`), GitHub Actions Pages.

## Global Constraints

- Spec: `docs/superpowers/specs/2026-07-24-design-system-docs-design.md`. Milestone item **#258** (P0, `epic:docs`).
- Branch: `feat/ds-docs-1` (already exists, contains the spec), base `release/1.0.0`. One PR per sub-lot.
- **Fonts: ALWAYS self-host IBM Plex woff2 copied from `web/ui/node_modules/@fontsource/…`. NEVER fetch from a CDN or DesignSync (corrupts/truncates).**
- **Self-contained site**: no external resource at runtime (fonts, CSS, JS all local).
- Brand brass anchor = `#c79a4a` (matches `src/cli/style.rs`); dim `#a8823d`, strong `#e0b45c`.
- **No binary screenshots** (PNG/GIF) — use text code blocks.
- No mermaid/dot in the wiki → **no mdBook preprocessor**.
- Conventional Commits, **single type only**. Commit trailer: `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`.
- rust-analyzer is unreliable here — verify at the compiler/build tool, not RA diagnostics.
- Per-lot gate: independent review + Dimitri visual validation before merge (README preview / `mdbook serve` / deployed site).

---

## File Structure

- `assets/brand/armadai-mark.svg` — the compass mark (self-contained, brass hex). **D1**
- `assets/brand/armadai-wordmark.svg` — horizontal lockup (mark + "ArmadAI" + tagline). **D1**
- `assets/brand/README.md` — notes the asset is the brand source of truth; keep `Shell.svelte` inline SVG in sync. **D1**
- `README.md` — rewritten as identity landing. **D1**
- `book.toml` — mdBook config at repo root (`src = "docs/wiki"`). **D2**
- `docs/wiki/SUMMARY.md` — mdBook table of contents. **D2**
- `docs/wiki/introduction.md` — book landing page. **D2**
- `docs/theme/custom.css` — DS tokens (light+dark) + `@font-face`. **D2**
- `docs/theme/fonts/*.woff2` — IBM Plex Sans 400/600/700 + Mono 400/500 (latin). **D2**
- `docs/theme/favicon.svg` — copy of the mark. **D2**
- `.github/workflows/docs.yml` — build + deploy to Pages on master. **D3**

---

## Task 1 (D1): Brand asset + README landing

**Files:**
- Create: `assets/brand/armadai-mark.svg`
- Create: `assets/brand/armadai-wordmark.svg`
- Create: `assets/brand/README.md`
- Modify: `README.md` (full rewrite of header + badges + intro; keep accurate technical content, trim what the wiki owns)

**Interfaces:**
- Produces: `assets/brand/armadai-mark.svg` (64×64 viewBox, self-contained brass hex) — reused by D2 as logo + favicon. `assets/brand/armadai-wordmark.svg` — header lockup.

- [ ] **Step 1: Create the mark SVG**

`assets/brand/armadai-mark.svg` (extracted from `web/ui/src/lib/Shell.svelte` lines 31-40, `var(--brass*)` → hex):

```svg
<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64" viewBox="0 0 64 64" role="img" aria-label="ArmadAI">
  <circle cx="32" cy="32" r="22" fill="none" stroke="#c79a4a" stroke-width="2.2" opacity="0.5"/>
  <polygon points="32,10 43,31 21,31" fill="#c79a4a"/>
  <g stroke="#a8823d" stroke-width="3" stroke-linecap="round">
    <line x1="32" y1="37" x2="32" y2="52"/>
    <line x1="17" y1="32" x2="24" y2="32"/>
    <line x1="47" y1="32" x2="40" y2="32"/>
  </g>
  <circle cx="32" cy="32" r="3.2" fill="#e0b45c"/>
</svg>
```

- [ ] **Step 2: Create the wordmark lockup SVG**

`assets/brand/armadai-wordmark.svg` — mark + text; "Armad" uses `currentColor` (adapts to light/dark GitHub theme), "AI" + tagline in brass:

```svg
<svg xmlns="http://www.w3.org/2000/svg" width="300" height="64" viewBox="0 0 300 64" role="img" aria-label="ArmadAI — pont de commandement">
  <g transform="translate(8,0)">
    <circle cx="32" cy="32" r="22" fill="none" stroke="#c79a4a" stroke-width="2.2" opacity="0.5"/>
    <polygon points="32,10 43,31 21,31" fill="#c79a4a"/>
    <g stroke="#a8823d" stroke-width="3" stroke-linecap="round">
      <line x1="32" y1="37" x2="32" y2="52"/>
      <line x1="17" y1="32" x2="24" y2="32"/>
      <line x1="47" y1="32" x2="40" y2="32"/>
    </g>
    <circle cx="32" cy="32" r="3.2" fill="#e0b45c"/>
  </g>
  <text x="78" y="34" font-family="'IBM Plex Sans','Segoe UI',system-ui,sans-serif" font-size="27" font-weight="700" fill="currentColor">Armad<tspan fill="#c79a4a">AI</tspan></text>
  <text x="79" y="50" font-family="'IBM Plex Sans','Segoe UI',system-ui,sans-serif" font-size="10" letter-spacing="2.5" fill="#a8823d">PONT DE COMMANDEMENT</text>
</svg>
```

- [ ] **Step 3: Create the brand README**

`assets/brand/README.md`:

```markdown
# ArmadAI brand assets

Source of truth for the "pont de commandement" identity.

- `armadai-mark.svg` — the compass mark (64×64). Brass `#c79a4a` / dim `#a8823d` / strong `#e0b45c`.
- `armadai-wordmark.svg` — horizontal lockup (mark + wordmark + tagline).

The web dashboard renders the same mark inline in `web/ui/src/lib/Shell.svelte`
(using `var(--brass*)` tokens). Keep the two visually in sync — this directory
is the canonical reference.
```

- [ ] **Step 4: Rewrite the README header + badges**

In `README.md`, replace the current top block (title + two broken `swarm-festai` badges, lines ~1-6) with a centered identity header and **corrected** badges (`Dr0drigues/armadai`):

```markdown
<p align="center">
  <img src="assets/brand/armadai-wordmark.svg" alt="ArmadAI" width="300">
</p>

<p align="center">
  AI agent orchestrator — define, manage and run specialized agents from Markdown files.
</p>

<p align="center">
  <a href="https://github.com/Dr0drigues/armadai/actions/workflows/ci.yml"><img src="https://github.com/Dr0drigues/armadai/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/Dr0drigues/armadai/actions/workflows/audit.yml"><img src="https://github.com/Dr0drigues/armadai/actions/workflows/audit.yml/badge.svg" alt="Security Audit"></a>
</p>
```

- [ ] **Step 5: Trim body toward a landing + add a Documentation section**

In `README.md`: keep **Overview**, a **condensed Key Features** list, a **short Quick Start** (the `install.sh` one-liner already points to `/master/`, plus the 3 example commands already present), and the existing example command block. Remove long-form how-to prose that duplicates the wiki (deep provider/orchestration/config walk-throughs) and replace it with a **Documentation** section linking the wiki pages:

```markdown
## Documentation

Full documentation lives in [`docs/wiki/`](docs/wiki/) (and the published site once available):

- [Getting Started](docs/wiki/getting-started.md)
- [Agent format](docs/wiki/agent-format.md)
- [Orchestration guide](docs/wiki/orchestration-guide.md)
- [Providers](docs/wiki/providers.md)
- [Skills & Prompts](docs/wiki/skills-prompts.md)
- [Migration v0 → v1](docs/wiki/migration-v0-to-v1.md)
```

Keep the existing **Git Flow** section (already master-only after #248). Do not touch install URLs (already `/master/`). Read the whole current `README.md` first and preserve any still-accurate section rather than deleting wholesale — the goal is a tighter, identity-forward landing, not information loss.

- [ ] **Step 6: Verify (no build tooling in D1)**

Run:
```bash
cargo fmt --all -- --check && echo "fmt OK"
xmllint --noout assets/brand/armadai-mark.svg assets/brand/armadai-wordmark.svg 2>&1 || echo "(xmllint absent — verify SVGs open in a browser)"
grep -c "swarm-festai" README.md   # expected: 0
grep -c "Dr0drigues/armadai" README.md  # expected: >= 2
```
Expected: fmt OK; SVGs well-formed; zero `swarm-festai`; corrected badges present. No Rust code touched, so clippy/test are unaffected (state it in the report).

- [ ] **Step 7: Commit**

```bash
git add assets/brand/ README.md
git commit -m "docs: brand assets + identity-forward README landing (DS-Docs D1)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

Visual validation (Dimitri): GitHub preview of the README (wordmark renders, badges green, layout reads as a landing).

---

## Task 2 (D2): mdBook site + DS theme

**Files:**
- Create: `book.toml` (repo root)
- Create: `docs/wiki/SUMMARY.md`
- Create: `docs/wiki/introduction.md`
- Create: `docs/theme/custom.css`
- Create: `docs/theme/fonts/` (5 woff2 copied from `@fontsource`)
- Create: `docs/theme/favicon.svg` (copy of `assets/brand/armadai-mark.svg`)
- Modify (light polish only): heading consistency / cross-links in `docs/wiki/*.md` if a broken link surfaces during `mdbook build`

**Interfaces:**
- Consumes: `assets/brand/armadai-mark.svg` (from D1) → copied to `docs/theme/favicon.svg`.
- Produces: a buildable mdBook at repo root (`mdbook build docs`? — see book.toml note) consumed by D3's CI.

- [ ] **Step 1: Install mdBook locally (pinned)**

```bash
cargo install mdbook --version 0.4.40 --locked
mdbook --version   # expected: mdbook v0.4.40
```
(If already installed at a different version, note it; D3 pins the same version in CI.)

- [ ] **Step 2: Create `book.toml`**

At repo root. `src` points at the existing wiki; theme dir is `docs/theme`:

```toml
[book]
title = "ArmadAI"
description = "AI agent orchestrator — pont de commandement"
authors = ["Dimitri Rodrigues-Oliveira"]
language = "en"
src = "docs/wiki"

[output.html]
default-theme = "light"
preferred-dark-theme = "navy"
additional-css = ["docs/theme/custom.css"]
git-repository-url = "https://github.com/Dr0drigues/armadai"
site-url = "/armadai/"
no-section-label = false

[output.html.fold]
enable = true
level = 1
```

Note: `build-dir` defaults to `book/` next to `book.toml`. Add `book/` to `.gitignore` in this task.

- [ ] **Step 3: Create `docs/wiki/SUMMARY.md`**

mdBook requires `SUMMARY.md` in `src`. Order the existing pages:

```markdown
# Summary

[Introduction](introduction.md)

- [Getting Started](getting-started.md)
- [Agent format](agent-format.md)
- [Orchestration](orchestration-guide.md)
  - [Reference](orchestration.md)
- [Providers](providers.md)
- [Skills & Prompts](skills-prompts.md)
- [Templates](templates.md)
- [Link](link.md)
- [Registry](registry.md)
- [Starter packs](starter-packs.md)
- [Migration v0 → v1](migration-v0-to-v1.md)
```

- [ ] **Step 4: Create `docs/wiki/introduction.md`**

Book landing page (the mark is served from the theme; reference it relatively):

```markdown
# ArmadAI

> AI agent orchestrator — define, manage and run specialized agents from Markdown files.

ArmadAI lets you build a fleet of specialized AI agents, each configured with a
simple Markdown file, and run them against any LLM provider (Claude, GPT, Gemini)
or CLI tool through a unified interface — with multi-pattern orchestration, a TUI
dashboard, a web UI, and cost tracking.

## Where to next

- New here? Start with [Getting Started](getting-started.md).
- Writing agents? See the [Agent format](agent-format.md).
- Coordinating a fleet? Read the [Orchestration guide](orchestration-guide.md).
- Upgrading? Follow [Migration v0 → v1](migration-v0-to-v1.md).
```

- [ ] **Step 5: Copy the self-hosted fonts + favicon**

```bash
mkdir -p docs/theme/fonts
FS=web/ui/node_modules/@fontsource
cp "$FS/ibm-plex-sans/files/ibm-plex-sans-latin-400-normal.woff2" docs/theme/fonts/
cp "$FS/ibm-plex-sans/files/ibm-plex-sans-latin-600-normal.woff2" docs/theme/fonts/
cp "$FS/ibm-plex-sans/files/ibm-plex-sans-latin-700-normal.woff2" docs/theme/fonts/
cp "$FS/ibm-plex-mono/files/ibm-plex-mono-latin-400-normal.woff2" docs/theme/fonts/
cp "$FS/ibm-plex-mono/files/ibm-plex-mono-latin-500-normal.woff2" docs/theme/fonts/
cp assets/brand/armadai-mark.svg docs/theme/favicon.svg
ls docs/theme/fonts/   # expected: 5 woff2 files
```
**Never** download these from a CDN/DesignSync — copy the `@fontsource` files only.

- [ ] **Step 6: Create `docs/theme/custom.css`**

`@font-face` (relative to the CSS, mdBook serves theme files at site root, so `fonts/…`) + DS token overrides on mdBook's `.light` and `.navy` theme classes. Values copied from `web/ui/src/tokens.css` (light `:root[data-theme=light]` block; dark `:root` block):

```css
/* Self-hosted IBM Plex (copied from @fontsource; never a CDN) */
@font-face { font-family:"IBM Plex Sans"; font-weight:400; font-style:normal; font-display:swap; src:url("fonts/ibm-plex-sans-latin-400-normal.woff2") format("woff2"); }
@font-face { font-family:"IBM Plex Sans"; font-weight:600; font-style:normal; font-display:swap; src:url("fonts/ibm-plex-sans-latin-600-normal.woff2") format("woff2"); }
@font-face { font-family:"IBM Plex Sans"; font-weight:700; font-style:normal; font-display:swap; src:url("fonts/ibm-plex-sans-latin-700-normal.woff2") format("woff2"); }
@font-face { font-family:"IBM Plex Mono"; font-weight:400; font-style:normal; font-display:swap; src:url("fonts/ibm-plex-mono-latin-400-normal.woff2") format("woff2"); }
@font-face { font-family:"IBM Plex Mono"; font-weight:500; font-style:normal; font-display:swap; src:url("fonts/ibm-plex-mono-latin-500-normal.woff2") format("woff2"); }

:root {
  --ds-brass: #c79a4a;
  --ds-brass-strong: #e0b45c;
}

html { font-family:"IBM Plex Sans","Segoe UI",system-ui,sans-serif; }
code, pre, .hljs { font-family:"IBM Plex Mono",ui-monospace,monospace; }

/* Light theme — DS light tokens (oklch from tokens.css) */
.light {
  --bg: oklch(0.985 0.004 236);
  --fg: oklch(0.245 0.036 248);
  --sidebar-bg: oklch(0.944 0.010 238);
  --sidebar-fg: oklch(0.400 0.030 246);
  --sidebar-non-existant: oklch(0.630 0.022 244);
  --sidebar-active: var(--ds-brass);
  --sidebar-spacer: oklch(0.868 0.013 240);
  --scrollbar: oklch(0.770 0.022 242);
  --icons: oklch(0.520 0.026 245);
  --icons-hover: var(--ds-brass);
  --links: oklch(0.560 0.110 78);
  --inline-code-color: oklch(0.475 0.108 76);
  --theme-popup-bg: oklch(0.980 0.006 237);
  --theme-popup-border: oklch(0.868 0.013 240);
  --theme-hover: oklch(0.944 0.010 238);
  --quote-bg: oklch(0.944 0.010 238);
  --quote-border: var(--ds-brass);
  --table-border-color: oklch(0.868 0.013 240);
  --table-header-bg: oklch(0.930 0.045 84);
  --table-alternate-bg: oklch(0.980 0.006 237);
  --searchbar-border-color: oklch(0.868 0.013 240);
  --searchbar-bg: oklch(0.980 0.006 237);
  --searchbar-fg: oklch(0.245 0.036 248);
  --search-mark-bg: oklch(0.930 0.045 84);
}

/* Dark theme (navy slot) — DS dark tokens */
.navy {
  --bg: oklch(0.188 0.030 248);
  --fg: oklch(0.955 0.008 240);
  --sidebar-bg: oklch(0.223 0.031 247);
  --sidebar-fg: oklch(0.790 0.015 240);
  --sidebar-non-existant: oklch(0.505 0.020 244);
  --sidebar-active: var(--ds-brass);
  --sidebar-spacer: oklch(0.360 0.030 244);
  --scrollbar: oklch(0.470 0.034 243);
  --icons: oklch(0.635 0.020 242);
  --icons-hover: var(--ds-brass-strong);
  --links: var(--ds-brass);
  --inline-code-color: var(--ds-brass-strong);
  --theme-popup-bg: oklch(0.223 0.031 247);
  --theme-popup-border: oklch(0.360 0.030 244);
  --theme-hover: oklch(0.258 0.032 246);
  --quote-bg: oklch(0.223 0.031 247);
  --quote-border: var(--ds-brass);
  --table-border-color: oklch(0.360 0.030 244);
  --table-header-bg: oklch(0.300 0.045 82);
  --table-alternate-bg: oklch(0.223 0.031 247);
  --searchbar-border-color: oklch(0.360 0.030 244);
  --searchbar-bg: oklch(0.223 0.031 247);
  --searchbar-fg: oklch(0.955 0.008 240);
  --search-mark-bg: oklch(0.300 0.045 82);
}

/* Brass menu title */
.menu-title { font-weight:700; color:var(--ds-brass); }
```
Note: mdBook variable names above are its documented theme variables; if `mdbook build`/serve reveals an off variable name for the pinned version, adjust to match — the values are the binding requirement (DS oklch), the variable names follow mdBook 0.4.40.

- [ ] **Step 7: Wire the favicon/logo**

mdBook uses `theme/favicon.svg` automatically if present in the theme dir. Confirm `docs/theme/favicon.svg` exists (Step 5). No `index.hbs` override needed unless the header logo is desired later (out of scope for a clean first pass).

- [ ] **Step 8: Build and verify**

```bash
mdbook build docs 2>&1 | tee /tmp/mdbook-build.log
grep -iE "warn|error|not found|broken" /tmp/mdbook-build.log || echo "clean build"
ls docs/theme/fonts/*.woff2 | wc -l   # expected: 5
```
Expected: build succeeds, **no broken-link/missing-file warnings**, 5 fonts present. If a wiki cross-link is flagged, fix that link in the offending `docs/wiki/*.md` (light polish) and rebuild.

- [ ] **Step 9: Visual check locally**

```bash
mdbook serve docs   # open http://localhost:3000, toggle light/dark
```
Confirm: IBM Plex renders (not a system fallback), brass accents on links/active nav/headers, legible in **both** light and navy. (This is where Dimitri validates visually before merge.)

- [ ] **Step 10: Ignore build output + commit**

```bash
echo "book/" >> .gitignore
git add book.toml docs/wiki/SUMMARY.md docs/wiki/introduction.md docs/theme/ .gitignore
git add docs/wiki/*.md   # only if a link was polished
git commit -m "docs: mdBook site with design-system theme (DS-Docs D2)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 3 (D3): GitHub Pages CI

**Files:**
- Create: `.github/workflows/docs.yml`

**Interfaces:**
- Consumes: `book.toml` + `docs/` from D2 (buildable with pinned mdBook 0.4.40).

- [ ] **Step 1: Create the workflow**

`.github/workflows/docs.yml`:

```yaml
name: Docs

on:
  push:
    branches: [master]
    paths:
      - "docs/**"
      - "book.toml"
      - ".github/workflows/docs.yml"
  workflow_dispatch:

permissions:
  contents: read
  pages: write
  id-token: write

concurrency:
  group: pages
  cancel-in-progress: false

jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Install mdBook
        run: |
          MDBOOK_VERSION=0.4.40
          curl -fsSL "https://github.com/rust-lang/mdBook/releases/download/v${MDBOOK_VERSION}/mdbook-v${MDBOOK_VERSION}-x86_64-unknown-linux-gnu.tar.gz" | tar -xz -C /usr/local/bin
      - name: Build
        run: mdbook build docs
      - uses: actions/upload-pages-artifact@v3
        with:
          path: book

  deploy:
    needs: build
    runs-on: ubuntu-latest
    environment:
      name: github-pages
      url: ${{ steps.deployment.outputs.page_url }}
    steps:
      - id: deployment
        uses: actions/deploy-pages@v4
```

Note: `mdbook build docs` writes to `book/` (per book.toml default relative to root). Confirm the `upload-pages-artifact` `path: book` matches the actual build-dir; if `build-dir` is set otherwise in book.toml, align them.

- [ ] **Step 2: Validate the workflow build locally (act-free)**

The deploy job only runs on GitHub. Validate the build half by reproducing its command:
```bash
mdbook build docs && test -f book/index.html && echo "artifact OK"
```
Expected: `artifact OK`.

- [ ] **Step 3: Note the ops step (Pages activation)**

Add to the PR description: **GitHub Pages must be enabled with source = "GitHub Actions"** in repo Settings → Pages (one-time manual step by the maintainer). The workflow only deploys after master receives the docs (which happens at the 1.0.0 master publish, or on the first `docs/**` push to master).

- [ ] **Step 4: Commit**

```bash
git add .github/workflows/docs.yml
git commit -m "ci: build and deploy mdBook docs to GitHub Pages on master (DS-Docs D3)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

Visual validation (Dimitri): after merge to master (or via `workflow_dispatch`), the deployed site renders with the DS theme.

---

## Self-Review

**Spec coverage:**
- Brand asset (mark + wordmark + brand README, brass anchor `#c79a4a`) → Task 1 Steps 1-3. ✅
- README landing (logo, fixed badges, condensed features/quick-start, docs links, git-flow kept) → Task 1 Steps 4-5. ✅
- mdBook (`book.toml` src=wiki, SUMMARY, introduction, theme = oklch tokens light+dark + self-hosted IBM Plex + favicon, light content polish, no preprocessor) → Task 2. ✅
- Pages CI on master → Task 3. ✅
- Constraints (fonts @fontsource only, self-contained, no screenshots, no preprocessor, conventional commits) → Global Constraints + per-step. ✅
- Sub-lot decomposition D1/D2/D3, one PR each, per-lot gate + visual validation → task boundaries. ✅

**Placeholder scan:** No TBD/TODO. SVG/CSS/YAML/TOML contents given in full; the two "adjust if mdBook version differs" notes are bounded fallbacks (values are fixed, only variable names may shift for the pinned version), not open-ended placeholders.

**Consistency:** Brass hex (`#c79a4a`/`#a8823d`/`#e0b45c`) identical across mark, wordmark, brand README, custom.css. mdBook version `0.4.40` identical in Task 2 Step 1 and Task 3 Step 1. `docs/theme/favicon.svg` produced in Task 2 Step 5, referenced Step 7. Font filenames identical between the copy step and the `@font-face` `src`.
