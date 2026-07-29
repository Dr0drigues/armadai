# Refonte Web Svelte — W2a (polices/icônes réelles + vues Prompts/Skills/Starters) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Compléter l'identité (vraies polices IBM Plex self-hosted + système d'icônes) et porter les vues listes Prompts/Skills/Starters dans l'appli Svelte servie à `/next`.

**Architecture:** Les woff2 IBM Plex vivent dans `web/ui/src/assets/fonts/` (importés par Vite → émis hashés dans `dist/assets/`), référencés par `@font-face` dans `tokens.css`. Un `icons.ts` (map nom→géométrie SVG, extraite de Lucide/DS) + `Icon.svelte` remplacent le SVG codé en dur du shell. Les vues Prompts/Skills/Starters suivent le patron de `Agents.svelte` (W1).

**Tech Stack:** Svelte 5, Vite, TypeScript, include_dir (embed).

## Global Constraints
- Node uniquement pour l'UI ; CI Rust seule (`dist/` commité). Gate : clippy 3 modes (`tui`, `tui,providers-api`, `tui,web,storage`) `-D warnings` + `cargo fmt -- --check` + `cargo test`.
- `npm run check` **0 erreur / 0 warning** (Svelte 5 idioms : `$state`/`$props`/`{@render}`/`onclick`, pas de `<slot>`/`on:`/auto-close non-void).
- Ne PAS toucher `/`, `/api/*`, `src/web/api.rs`. On travaille dans `web/ui/` + regénère `web/ui/dist/`.
- Source de style/valeurs : `tokens/*.css` du DS + la démo `<scratchpad>/armadai-web-demo.html`. DS projet Claude Design `416749e1-6cb2-460b-be6d-6bb2df3ddc6d`. Scratchpad session = `/private/tmp/claude-502/-Users-bl209054-work-misc-armadai/545db4a0-c939-4d03-9059-60229ed309bf/scratchpad`.
- User-facing → PR + revue indé + **validation visuelle Dimitri** (`armadai web` → `/next`) avant merge.

---

### Task 1: Vraies polices IBM Plex + système d'icônes

**Files:**
- Create: `web/ui/src/assets/fonts/IBMPlexSans-400.woff2`, `IBMPlexSans-600.woff2`, `IBMPlexMono-400.woff2`, `IBMPlexMono-600.woff2` (binaires récupérés du DS)
- Create: `web/ui/src/lib/icons.ts`, `web/ui/src/lib/Icon.svelte`
- Modify: `web/ui/src/tokens.css` (ré-ajouter `@font-face`), `web/ui/src/lib/Shell.svelte` (utiliser `<Icon>`)
- Regénère + commite `web/ui/dist/`

**Interfaces:**
- Produces : `Icon.svelte` (prop `name: string`, `size?: number`) rendant un `<svg viewBox="0 0 24 24">` depuis `icons.ts`.

- [ ] **Step 1 : Récupérer les woff2 du DS (décodage explicite)** — charge DesignSync (`ToolSearch` `select:DesignSync`). Pour chaque police (`assets/fonts/IBMPlexSans-400.woff2`, `-600`, `IBMPlexMono-400`, `-600`), `get_file` (`projectId: 416749e1-6cb2-460b-be6d-6bb2df3ddc6d`) → le résultat a `isBase64: true` + `content` (base64). **Écris le base64 dans un fichier temporaire** (`.b64`) puis décode : `base64 -d web/ui/src/assets/fonts/X.woff2.b64 > web/ui/src/assets/fonts/X.woff2 && rm X.woff2.b64`. **Vérifie `ls -la` que chaque woff2 fait > 10 000 octets** (un fichier de 0 octet = échec du décodage — recommence). Si DesignSync est indisponible ou le décodage échoue, ARRÊTE et signale BLOCKED (ne commite pas de placeholder vide).

- [ ] **Step 2 : `@font-face` dans `tokens.css`** — ré-ajouter en tête (remplaçant le commentaire « deferred to W2 ») :
```css
@font-face { font-family:"IBM Plex Sans"; font-weight:400; font-display:swap; src:url("./assets/fonts/IBMPlexSans-400.woff2") format("woff2"); }
@font-face { font-family:"IBM Plex Sans"; font-weight:600; font-display:swap; src:url("./assets/fonts/IBMPlexSans-600.woff2") format("woff2"); }
@font-face { font-family:"IBM Plex Mono"; font-weight:400; font-display:swap; src:url("./assets/fonts/IBMPlexMono-400.woff2") format("woff2"); }
@font-face { font-family:"IBM Plex Mono"; font-weight:600; font-display:swap; src:url("./assets/fonts/IBMPlexMono-600.woff2") format("woff2"); }
```
(`--font-ui`/`--font-mono` incluent déjà `"IBM Plex Sans"`/`"IBM Plex Mono"` en tête — inchangé.)

- [ ] **Step 3 : `icons.ts`** — une map `name → tableau de nœuds SVG` (géométrie Lucide, viewBox 24×24), pour les icônes utilisées par la nav + les vues. Icônes minimales : `agents`, `prompts`, `skills`, `starters`, `history`, `costs`, `models`. Ex. de forme (porter les vraies géométries Lucide correspondantes) :
```ts
type Node = { tag: "path" | "circle" | "line" | "polygon"; attrs: Record<string, string | number> };
export const ICONS: Record<string, Node[]> = {
  agents: [{ tag: "circle", attrs: { cx: 12, cy: 12, r: 3 } }, { tag: "path", attrs: { d: "M12 2v4M12 18v4M2 12h4M18 12h4M5 5l3 3M16 16l3 3M19 5l-3 3M8 16l-3 3" } }],
  // prompts, skills, starters, history, costs, models: idem (géométrie Lucide)
};
```

- [ ] **Step 4 : `Icon.svelte`**
```svelte
<script lang="ts">
  import { ICONS } from "./icons";
  let { name, size = 16 }: { name: string; size?: number } = $props();
  const nodes = $derived(ICONS[name] ?? []);
</script>
<svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
  {#each nodes as n}
    {#if n.tag === "path"}<path {...n.attrs} />{:else if n.tag === "circle"}<circle {...n.attrs} />{:else if n.tag === "line"}<line {...n.attrs} />{:else if n.tag === "polygon"}<polygon {...n.attrs} />{/if}
  {/each}
</svg>
```

- [ ] **Step 5 : brancher `Icon` dans `Shell.svelte`** — remplacer le `<svg>` codé en dur de la nav par `<Icon name={tab.icon ?? tab.id} />` ; ajouter un champ optionnel `icon?: string` au type `Tab` ; dans `App.svelte`, donner à chaque onglet son `icon` (`agents`, `prompts`, …).

- [ ] **Step 6 : build + gate** — `cd web/ui && npm run check` (0/0) + `npm run build` → regénère `dist/`. `cargo test --no-default-features --features tui,web,storage web::tests` (vert) + clippy `tui,web,storage --all-targets` (0) + `cargo fmt -- --check`. Lancer `armadai web`, ouvrir `/next` : IBM Plex charge (réseau `/next/assets/*.woff2` → 200, taille réelle), icônes nav rendues.

- [ ] **Step 7 : Commit**
```bash
git add web/ui/src web/ui/dist
git commit -m "feat(web): self-host IBM Plex fonts and add a Lucide-based Icon component"
```

---

### Task 2: Vues Prompts, Skills, Starters

**Files:**
- Modify: `web/ui/src/lib/api.ts` (types + getters)
- Create: `web/ui/src/views/Prompts.svelte`, `Skills.svelte`, `Starters.svelte`
- Modify: `web/ui/src/App.svelte` (brancher les 3 onglets)
- Regénère + commite `web/ui/dist/`

**Interfaces:**
- Consumes : `Shell`, `Icon` (Task 1), endpoints `/api/prompts` (`PromptSummary[]`), `/api/skills` (`SkillSummary[]`), `/api/starters` (`StarterSummary[]`).
- Produces : `getPrompts()`, `getSkills()`, `getStarters()` + types alignés sur `src/web/api.rs`.

- [ ] **Step 1 : types + getters dans `api.ts`** (alignés `src/web/api.rs`) :
```ts
export interface PromptSummary { name: string; description: string | null; apply_to: string[]; source: string; }
export interface SkillSummary { name: string; description: string | null; version: string | null; tools: string[]; source: string; }
export interface StarterSummary { name: string; description: string; agents_count: number; prompts_count: number; skills_count: number; }
export const getPrompts = () => getJson<PromptSummary[]>("/api/prompts");
export const getSkills = () => getJson<SkillSummary[]>("/api/skills");
export const getStarters = () => getJson<StarterSummary[]>("/api/starters");
```

- [ ] **Step 2 : vues** — 3 composants calqués sur `Agents.svelte` (chargement `$state` + `onMount`, état de chargement minimal, styles `.agent`/`.panel`/`table` de la démo) :
  - `Prompts.svelte` : liste (nom, description, `apply_to` en tags, `source` en `.mono`/`.eyebrow`).
  - `Skills.svelte` : liste (nom, description, `version` badge, `tools` en tags, `source`).
  - `Starters.svelte` : cartes (nom, description, compteurs agents/prompts/skills en `.mono` tabular-nums).

- [ ] **Step 3 : brancher dans `App.svelte`** — remplacer les placeholders « à venir en W2 » pour `prompts`/`skills`/`starters` par `<Prompts />`/`<Skills />`/`<Starters />` (garder les placeholders pour `costs`/`models` → W2b).

- [ ] **Step 4 : build + gate** — `npm run check` (0/0) + `npm run build` → `dist/`. `cargo test … web::tests` + clippy 3 modes + fmt. `armadai web` → `/next` : les 3 onglets listent les données réelles. **Validation visuelle Dimitri.**

- [ ] **Step 5 : Commit**
```bash
git add web/ui/src web/ui/dist
git commit -m "feat(web): Prompts, Skills and Starters views (Svelte)"
```

---

## Self-Review
- **Couverture** : polices IBM Plex self-hosted (Task 1, décodage explicite + vérif taille) ✓ ; système d'icônes réutilisable (`icons.ts`+`Icon.svelte`) ✓ ; vues Prompts/Skills/Starters (Task 2) ✓ ; `/`/`api.rs` intouchés ✓ ; dist commité, CI Rust seule ✓.
- **Placeholders** : `icons.ts` donne un exemple de forme + liste les icônes à porter (géométrie Lucide réelle à remplir depuis le DS `assets/icons.js` ou lucide.dev) — pas un placeholder de valeur, une source identifiée. Le reste (api.ts, Icon.svelte, @font-face) est complet.
- **Cohérence types** : `PromptSummary`/`SkillSummary`/`StarterSummary` alignés `api.rs`↔`api.ts` ; `Icon` props cohérentes Task 1→2 ; `Tab.icon` ajouté Task 1 utilisé App.svelte.
- **Risque connu** : le fetch/décodage woff2 (échec en W1) — mitigé par le décodage explicite `base64 -d` + vérif taille > 10 Ko + BLOCKED si échec (pas de placeholder vide).
