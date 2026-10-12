# Refonte Web Svelte — W2c (routeur + pages détail + onglet Orchestration) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Ajouter à l'appli Svelte (`/next`) un routeur client, des **pages détail** dédiées (agent/prompt/skill/starter) avec rendu **markdown**, et un onglet **Orchestration** (topologie **mermaid** + traces).

**Architecture:** Un petit routeur hash maison (`route.svelte.ts`, pas de lib) : `#/agents`, `#/agents/{name}`, `#/orchestration`, `#/orchestration/{run_id}`, etc. `App.svelte` rend la vue selon la route ; le shell/nav navigue en changeant le hash. `marked` (markdown) et `mermaid` (diagrammes) deviennent des deps npm bundlées par Vite. Détails via `/api/{kind}/{name}` (JSON libre), orchestration via `/api/orchestration/{trace,trace/{id},topology}`.

**Tech Stack:** Svelte 5, Vite, TypeScript, marked, mermaid.

## Global Constraints
- Node uniquement pour l'UI ; CI Rust seule (`dist/` commité). Gate : clippy 3 modes + fmt + test.
- `npm run check` **0 erreur / 0 warning** (Svelte 5 : `$state`/`$props`/`$derived`/`{@render}`/`onclick` ; jamais `on:`/`<slot>`/auto-close non-void).
- **Polices : NE PAS toucher** — IBM Plex via `@fontsource` (main.ts). Aucun woff2 dans `web/ui/src/**`, aucun `@font-face` manuel, ne jamais fetch de woff2 du design system.
- Ne PAS toucher `/`, `/api/*`, `src/web/api.rs`. Travail dans `web/ui/` + regénère `web/ui/dist/`.
- Le SPA est servi sous base `/next/` (fallback SPA déjà en place côté axum) — le **routing client se fait via le hash** (`location.hash`), donc aucune route serveur supplémentaire n'est nécessaire.
- `marked`/`mermaid` = bundlés (pas de CDN). Rendu markdown : **assainir** (au minimum, ne pas injecter de HTML arbitraire non maîtrisé — `marked` seul ne sanitize pas ; utiliser `marked.parse` sur du contenu de config local, considéré de confiance ici — documenter).
- Style : tokens DS + démo `<scratchpad>/armadai-web-demo.html`. Scratchpad = `/private/tmp/claude-502/-Users-bl209054-work-misc-armadai/545db4a0-c939-4d03-9059-60229ed309bf/scratchpad`.
- User-facing → PR + revue indé + **validation visuelle Dimitri** (`armadai web` → `/next`) avant merge.

---

### Task 1: Routeur hash + composant Markdown

**Files:**
- Create: `web/ui/src/lib/route.svelte.ts`, `web/ui/src/lib/Markdown.svelte`
- Modify: `web/ui/src/App.svelte` (rendu par route), `web/ui/src/lib/Shell.svelte` (nav via hash), `web/ui/package.json` (dep `marked`)

**Interfaces:**
- Produces : store `route` (`{ view: string; param: string | null }`, dérivé de `location.hash`) + `navigate(path: string)` ; `Markdown.svelte` (prop `source: string`).

- [ ] **Step 1 : dep marked** — `cd web/ui && npm install marked` (dep, pas devDep — bundlée dans le runtime).

- [ ] **Step 2 : `route.svelte.ts`** — routeur hash minimal (Svelte 5 runes) :
```ts
function parse(hash: string): { view: string; param: string | null } {
  const path = hash.replace(/^#\/?/, "");         // "agents/foo" | "agents" | ""
  const [view = "agents", param = null] = path.split("/");
  return { view, param: param || null };
}
class Router {
  current = $state(parse(location.hash));
  constructor() {
    addEventListener("hashchange", () => { this.current = parse(location.hash); });
  }
  navigate(path: string) { location.hash = "#/" + path.replace(/^#?\/?/, ""); }
}
export const router = new Router();
export const navigate = (p: string) => router.navigate(p);
```

- [ ] **Step 3 : `Markdown.svelte`** — rend le markdown via `marked` :
```svelte
<script lang="ts">
  import { marked } from "marked";
  let { source = "" }: { source?: string } = $props();
  const html = $derived(source ? (marked.parse(source, { async: false }) as string) : "");
</script>
<div class="md">{@html html}</div>
<style>
  .md :global(h1), .md :global(h2), .md :global(h3) { font-weight: 600; margin: 0.6em 0 0.3em; }
  .md :global(p) { margin: 0.4em 0; color: var(--text-secondary); }
  .md :global(code) { font-family: var(--font-mono); font-size: var(--text-sm); background: var(--surface-3); padding: 1px 4px; border-radius: 3px; }
  .md :global(pre) { background: var(--surface-3); padding: var(--panel-pad); border-radius: 6px; overflow-x: auto; }
  .md :global(a) { color: var(--brass); }
</style>
```
Note sécurité : `marked.parse` n'assainit pas le HTML ; la source est du contenu de config local (agents/prompts/skills de l'utilisateur), considéré de confiance. Documenter dans un commentaire.

- [ ] **Step 4 : `App.svelte` + `Shell.svelte` par route** — `App.svelte` : `const r = $derived(router.current)` ; rendre la vue selon `r.view` (`agents`→liste ou détail selon `r.param`, etc., `orchestration`→onglet Orchestration en Task 3). `Shell.svelte` : la nav appelle `navigate(tab.id)` (au lieu du callback `onselect`) et `active` = `router.current.view`. L'onglet actif dérive de la route.

- [ ] **Step 5 : build + gate** — `npm run check` (0/0) + `npm run build` → `dist/`. `cargo test … web::tests` + clippy 3 modes + fmt. `armadai web` → `/next` : navigation par onglet marche via hash (`#/agents`, `#/history`…), un titre markdown de test rend correctement.

- [ ] **Step 6 : Commit**
```bash
git add web/ui/src web/ui/dist web/ui/package.json web/ui/package-lock.json
git commit -m "feat(web): hash router + Markdown component (marked)"
```

---

### Task 2: Pages détail (agent / prompt / skill / starter)

**Files:**
- Modify: `web/ui/src/lib/api.ts` (getters détail), `web/ui/src/App.svelte` (routes détail), les vues listes (lignes cliquables → `navigate`)
- Create: `web/ui/src/views/Detail.svelte` (générique) ou une par type
- Regénère + commite `web/ui/dist/`

**Interfaces:**
- Consumes : router (Task 1), Markdown (Task 1), endpoints `/api/{agents,prompts,skills,starters}/{name}` → JSON libre (`unknown`/`Record<string, unknown>`).
- Produces : `getDetail(kind, name): Promise<Record<string, unknown>>`.

- [ ] **Step 1 : getter détail dans `api.ts`**
```ts
export const getDetail = (kind: string, name: string) =>
  getJson<Record<string, unknown>>(`/api/${kind}/${encodeURIComponent(name)}`);
```

- [ ] **Step 2 : `Detail.svelte`** — props `kind: string`, `name: string`. Charge `getDetail(kind, name)` (`$state`+`onMount`/`$effect` sur `name`). Rend : un fil d'Ariane / bouton retour (`navigate(kind)`), le nom en titre, les champs du JSON (itérer les paires clé/valeur avec un rendu adapté : tableaux → tags, `description`/contenu long → `<Markdown source={...}>`, scalaires → texte). Style `.panel`/`.detail` de la démo.

- [ ] **Step 3 : lignes cliquables** — dans Agents/Prompts/Skills/Starters, rendre chaque ligne cliquable (`onclick={() => navigate(\`${kind}/${encodeURIComponent(name)}\`)}`, `role="button"`, `tabindex`, `onkeydown` Enter) → change le hash vers la route détail.

- [ ] **Step 4 : routes détail dans `App.svelte`** — quand `router.current.param` est défini pour une vue à détail, rendre `<Detail kind={r.view} name={r.param} />` au lieu de la liste.

- [ ] **Step 5 : build + gate** — `npm run check` (0/0) + build + `cargo test … web::tests` + clippy 3 modes + fmt. `armadai web` → `/next` : clic sur un agent → page détail (`#/agents/{name}`), description en markdown, retour vers la liste. **Validation visuelle Dimitri.**

- [ ] **Step 6 : Commit**
```bash
git add web/ui/src web/ui/dist web/ui/package.json
git commit -m "feat(web): dedicated detail pages (agent/prompt/skill/starter) with markdown"
```

---

### Task 3: Onglet Orchestration (topologie mermaid + traces)

**Files:**
- Modify: `web/ui/src/lib/api.ts` (getters orchestration), `web/ui/src/App.svelte` (onglet + routes), `web/ui/src/lib/Shell.svelte` (onglet nav + icône), `web/ui/package.json` (dep `mermaid`)
- Create: `web/ui/src/lib/Mermaid.svelte`, `web/ui/src/views/Orchestration.svelte`
- Regénère + commite `web/ui/dist/`

**Interfaces:**
- Consumes : router, endpoints `/api/orchestration/topology` (`OrchestrationTopology{enabled, pattern, coordinator, teams:[{lead,agents}], agents}`), `/api/orchestration/trace` (JSON libre : liste de runs), `/api/orchestration/trace/{run_id}` (JSON libre : run + entrées).
- Produces : `getTopology()`, `getTraces()`, `getTraceDetail(runId)` ; `Mermaid.svelte` (prop `code: string`).

- [ ] **Step 1 : dep mermaid** — `npm install mermaid` (dep runtime, bundlée). NOTE : mermaid est lourd — vérifier que le build reste raisonnable ; import dynamique (`await import("mermaid")`) dans `Mermaid.svelte` pour ne pas gonfler le bundle initial.

- [ ] **Step 2 : getters + types `api.ts`**
```ts
export interface TopologyTeam { lead: string | null; agents: string[]; }
export interface OrchestrationTopology { enabled: boolean; pattern: string | null; coordinator: string | null; teams: TopologyTeam[]; agents: string[]; }
export const getTopology = () => getJson<OrchestrationTopology>("/api/orchestration/topology");
export const getTraces = () => getJson<Record<string, unknown>[]>("/api/orchestration/trace");
export const getTraceDetail = (id: string) => getJson<Record<string, unknown>>(`/api/orchestration/trace/${encodeURIComponent(id)}`);
```

- [ ] **Step 3 : `Mermaid.svelte`** — prop `code: string` ; import dynamique de mermaid, `mermaid.initialize({ startOnLoad: false, theme: "dark" })` (aligner sur le thème courant), `mermaid.render(id, code)` → `{@html svg}` dans un conteneur `overflow-x:auto`. Gérer l'erreur de parse (afficher le code brut en fallback).

- [ ] **Step 4 : `Orchestration.svelte`** — charge `getTopology()` + `getTraces()`. Si `topology.enabled`, construire un graphe mermaid (`flowchart TD`) depuis coordinator→teams(lead)→agents et le passer à `<Mermaid>`. Sinon message « aucune orchestration configurée ». Sous la topologie, lister les traces récentes (lignes cliquables → `navigate(\`orchestration/${runId}\`)`). Quand `router.current.param` est défini, rendre le détail d'une trace (`getTraceDetail`) : entrées de délégation, métriques.

- [ ] **Step 5 : onglet nav** — ajouter `{ id: "orchestration", label: "Orchestration", icon: "..." }` aux tabs (App.svelte) + une icône dans `icons.ts` (ex. réutiliser une géométrie Lucide « share-2 »/« workflow »). `App.svelte` : `r.view === "orchestration"` → `<Orchestration />`.

- [ ] **Step 6 : build + gate** — `npm run check` (0/0) + build (vérifier la taille du bundle mermaid raisonnable) + `cargo test … web::tests` + clippy 3 modes + fmt. `armadai web` → `/next` → onglet Orchestration : topologie rendue (mermaid) + traces → détail. **Validation visuelle Dimitri.**

- [ ] **Step 7 : Commit**
```bash
git add web/ui/src web/ui/dist web/ui/package.json web/ui/package-lock.json
git commit -m "feat(web): Orchestration tab — topology (mermaid) and delegation traces"
```

---

## Self-Review
- **Couverture** : routeur hash + markdown (Task 1) ✓ ; pages détail routées (Task 2, décision Dimitri « page dédiée ») ✓ ; onglet Orchestration topologie mermaid + traces (Task 3, décision Dimitri « nouvel onglet ») ✓ ; polices intouchées, `/`/`api.rs` intouchés ✓.
- **Placeholders** : code du routeur / Markdown / getters / Mermaid complet ; les vues Detail/Orchestration décrites concrètement (JSON libre → rendu par paires, mermaid depuis OrchestrationTopology). marked/mermaid = deps npm ajoutées aux steps.
- **Cohérence types** : `router.current {view,param}` cohérent Task 1→2→3 ; `OrchestrationTopology` aligné `api.rs` ; `getDetail`/`getTraceDetail` renvoient `Record<string, unknown>` (JSON libre, cohérent avec les endpoints `Json<Value>`).
- **Risques** : mermaid lourd → import dynamique (Task 3 Step 1/3) ; `{@html}` du markdown/mermaid = contenu de config local de confiance (documenté) — pas d'entrée utilisateur distante.
