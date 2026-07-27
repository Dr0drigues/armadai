# Design System → Docs — Design

## Contexte

4ᵉ et dernière surface du chantier design system « pont de commandement » (Web Svelte ✅, TUI T3a-d ✅, CLI ✅ — cf. mémoire `project_design_system_rollout`). Cible : les **docs** du projet. Aujourd'hui :

- **README** (458 l) : aucun logo, badges CI/Audit **cassés** (pointent `swarm-festai` au lieu de `Dr0drigues/armadai`), pas d'identité visuelle.
- **docs/wiki/** : 11 pages Markdown nues (getting-started, agent-format, orchestration-guide, providers, skills-prompts, templates, link, registry, starter-packs, migration-v0-to-v1, orchestration).
- **Logo/wordmark** : le compas laiton + « ArmadAI / pont de commandement » n'existe **qu'inline** dans `web/ui/src/lib/Shell.svelte` (SVG 64×64 utilisant `var(--brass*)`). Pas d'asset autonome réutilisable.
- **Aucun générateur de site de doc** (pas de mdBook/mkdocs/zola). GitHub rend nativement README + wiki en Markdown.

Item milestone : **#258** (P0, `epic:docs`).

## Objectif

Donner une identité DS aux docs sur trois livrables : (1) un **asset de marque autonome** (source de vérité), (2) un **README landing** identitaire qui délègue au site, (3) un **site de doc mdBook** thémé aux tokens DS et publié sur GitHub Pages.

## Décisions validées (Dimitri, 2026-07-24)

1. **Générateur** = **mdBook** (Rust-natif, réutilise les `.md` du wiki, thème custom maîtrisé). Pas de site bespoke Svelte.
2. **README** = **landing identitaire** qui délègue au site (pas de double-maintenance README↔wiki).
3. **Publication** = **GitHub Pages, auto sur `master`** (la doc publiée reflète la dernière stable).
4. **Polices** = IBM Plex Sans/Mono **self-hostées** (woff2 latin depuis `@fontsource`, déjà présents dans `web/ui/node_modules`). **Jamais de CDN**, jamais de fetch DesignSync (corrompt).
5. Contrainte transverse : **self-contained** (aucune ressource externe au runtime du site).

## Architecture

### 1. Asset de marque (source de vérité)

- Extraire le compas de `Shell.svelte` vers **`assets/brand/armadai-mark.svg`** : mêmes primitives (cercle, aiguille triangle, réticule, point central) mais **couleurs laiton en dur** — les `var(--brass*)` ne résolvent pas hors Svelte. Ancre = **`#c79a4a`** (le laiton concret déjà utilisé transversalement dans `src/cli/style.rs`, mid-tone lisible fond clair ET sombre). Les nuances dim/strong sont dérivées de cette ancre pour matcher les tokens DS (`--brass-dim` oklch(0.66 0.086 82), `--brass-strong` oklch(0.855 0.11 86) — hex final choisi à l'implémentation). Le SVG utilise du hex (compat large : rendu GitHub, favicon).
- **`assets/brand/armadai-wordmark.svg`** : lockup horizontal = mark + « Armad**AI** » (AI en laiton) + tagline « pont de commandement » (petite capitale espacée). Pour le header du site + usage README optionnel.
- Asset canonique réutilisé par le README (embed) et mdBook (logo + favicon). `Shell.svelte` reste inline (Svelte inline le SVG) ; un commentaire dans le `.svelte` et dans `assets/brand/README.md` note que les deux doivent rester raccord (une seule vérité visuelle = le DS).

### 2. README — landing identitaire

Réécriture ciblée (on garde le fond technique correct, on retire le détail qui vit dans le wiki) :

- **En-tête** : `armadai-mark.svg` (ou wordmark) centré via une balise `<p align="center"><img …></p>`, titre, tagline, une phrase de pitch.
- **Badges corrigés** : `Dr0drigues/armadai` (CI + Security Audit) + éventuellement licence/version. Fin du `swarm-festai`.
- **Sections condensées** : Key Features (liste existante, resserrée), Quick Start **court** (install one-liner `/master/` + 3 commandes), un bloc « capture » de sortie CLI/TUI (texte, pas d'image binaire à maintenir).
- **Renvoi au site** : section « Documentation » pointant vers le site mdBook (getting-started, orchestration, providers, migration…).
- **Git Flow** : déjà migré master-only (fait en #248) — conservé.

Aucune image binaire lourde committée (le SVG de marque est léger et textuel ; pas de screenshots PNG pour éviter la dette de maintenance).

### 3. Site mdBook + thème DS

- **`book.toml`** à la racine `docs/` : `[book] title = "ArmadAI", src = "wiki"` (réutilise les 11 `.md` **tels quels** — les liens relatifs `.md` sont réécrits en `.html` par mdBook). `[output.html]` : `default-theme`/`preferred-dark-theme`, `git-repository-url`, `additional-css`, `additional-js` si besoin.
- **`docs/wiki/SUMMARY.md`** : table des matières ordonnée (Introduction → Getting Started → Agent format → Orchestration (guide + reference) → Providers → Skills & Prompts → Templates → Link → Registry → Starter packs → Migration v0→v1). Une **page d'intro** `docs/wiki/introduction.md` (landing du book : pitch + logo + liens rapides).
- **`docs/theme/`** (thème custom mdBook) :
  - `custom.css` : variables aux **tokens DS oklch** (dark + light, via les classes de thème mdBook `.navy`/`.rust`/… ou un thème custom nommé `armadai`), accents laiton, liens/titres/code blocks soignés, cohérence avec `assets/terminal-palette.json`/`tokens`.
  - **Polices** : woff2 IBM Plex Sans + Mono (latin) copiées dans `docs/theme/fonts/` + `@font-face` inline dans `custom.css`. Aucune requête externe.
  - **Logo + favicon** : `armadai-mark.svg` (favicon + logo header via `theme/index.hbs` minimalement patché ou l'option de logo mdBook).
- **Diagrammes** : le wiki n'en contient aucun (mermaid/dot) → **pas de préprocesseur**.
- **Polish contenu wiki** (léger, dans ce lot) : cohérence des titres H1, ajout de la page introduction, vérif des cross-links, front-matter si nécessaire. Pas de réécriture de fond.

### 4. Publication — GitHub Pages sur `master`

- **`.github/workflows/docs.yml`** : `on: push: branches: [master]` (+ `paths: [docs/**]` pour éviter les builds inutiles). Étapes : checkout → install `mdbook` (+ éventuels binaires de thème) via une action ou `cargo binstall`/release download → `mdbook build docs` → deploy sur GitHub Pages (`actions/deploy-pages` + `upload-pages-artifact`). Permissions `pages: write`, `id-token: write`.
- Activation de GitHub Pages (source = GitHub Actions) : étape ops manuelle, notée dans le lot D3.
- Cohérent avec le flow master-only : la doc publiée = dernière stable sur master.

## Découpage (une PR / sous-lot ; revue indé + validation visuelle Dimitri)

- **D1 — Asset de marque + README landing.** `assets/brand/*.svg` (+ `assets/brand/README.md`), réécriture README (logo, badges corrigés, features/quick-start resserrés, renvoi site). Indépendant du site. **Visuel** = preview GitHub (rendu du README + du SVG).
- **D2 — Site mdBook + thème DS.** `book.toml`, `SUMMARY.md`, `introduction.md`, `docs/theme/` (CSS tokens + polices + logo), polish contenu léger. **Visuel** = `mdbook serve docs` en local chez Dimitri (light + dark).
- **D3 — CI GitHub Pages.** `.github/workflows/docs.yml` + activation Pages. Petit lot, revue CI. **Visuel** = le site déployé (après premier run sur master — ou preview d'artefact).

## Hors périmètre

- Site bespoke Svelte (rejeté au profit de mdBook).
- Screenshots PNG/GIF (dette de maintenance ; on reste en blocs texte).
- Versionnage multi-versions de la doc (une seule version publiée = master).
- Refonte de fond du **contenu** des pages wiki (seulement mise en page/identité + polish léger).

## Tests / Gate

- **Pas de tests unitaires** (docs). Gate par lot :
  - **D1** : `cargo fmt`/clippy/test inchangés (aucun code Rust touché) ; rendu README vérifié en preview.
  - **D2** : **`mdbook build docs`** propre (zéro warning de lien cassé) ; le thème charge les polices self-hostées (vérif `@font-face` → fichiers présents) ; `mdbook serve` OK light+dark.
  - **D3** : le workflow build le site en CI (dry-run/artefact) ; déploiement Pages vérifié après merge sur master.
- **Validation visuelle Dimitri** par lot (README preview ; `mdbook serve` ; site déployé).
- CI globale (6 checks) reste verte (les lots docs ne touchent pas le code Rust, sauf éventuel commentaire dans `Shell.svelte`).

## Risques

- **Installation de mdBook en CI** : figer une version (action tierce ou download release) pour la reproductibilité ; pas de dépendance réseau au runtime du site (self-contained).
- **Thème mdBook** : l'API de theming (override `theme/`) peut casser à une montée de version mdBook → figer la version mdBook et documenter le thème dans `docs/theme/README.md`.
- **Polices** : TOUJOURS `@fontsource` (woff2 déjà dans `web/ui/node_modules`), jamais de fetch DesignSync (corrompt/tronque — piège récurrent du chantier).
- **Dérive mark inline vs asset** : `Shell.svelte` garde son SVG inline ; documenter la source unique pour éviter la divergence.
