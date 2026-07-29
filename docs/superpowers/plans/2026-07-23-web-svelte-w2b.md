# Refonte Web Svelte — W2b (Costs + Models) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Porter les vues **Costs** (jauges) et **Models** (catalogue groupé par provider) dans l'appli Svelte servie à `/next`.

**Architecture:** Un composant `Gauge.svelte` réutilisable (barre + valeur, tokens DS) pour Costs ; Models = catalogue groupé par provider (tables denses, mono/tabular-nums). Mêmes patrons que les vues W1/W2a. `api.ts` reçoit les getters typés.

**Tech Stack:** Svelte 5, Vite, TypeScript.

## Global Constraints
- Node uniquement pour l'UI ; CI Rust seule (`dist/` commité). Gate : clippy 3 modes + fmt + test.
- `npm run check` **0 erreur / 0 warning** (Svelte 5 : `$state`/`$props`/`$derived`/`{@render}`/`onclick` ; jamais `on:`/`<slot>`/auto-close non-void).
- **Polices : NE PAS toucher** — IBM Plex est via `@fontsource` (main.ts). Ne jamais re-fetcher de woff2 du design system (produit des fichiers corrompus). Aucun woff2 dans `web/ui/src/**`.
- Ne PAS toucher `/`, `/api/*`, `src/web/api.rs`. Travail dans `web/ui/` + regénère `web/ui/dist/`.
- Style : tokens DS + démo `<scratchpad>/armadai-web-demo.html` (jauges `.gauge`, métriques `.metric`, tables). Scratchpad = `/private/tmp/claude-502/-Users-bl209054-work-misc-armadai/545db4a0-c939-4d03-9059-60229ed309bf/scratchpad`.
- User-facing → PR + revue indé + **validation visuelle Dimitri** (`armadai web` → `/next`) avant merge.

---

### Task 1: Composant `Gauge` + vue Costs

**Files:**
- Create: `web/ui/src/lib/Gauge.svelte`, `web/ui/src/views/Costs.svelte`
- Modify: `web/ui/src/lib/api.ts` (type + getter), `web/ui/src/App.svelte` (brancher Costs)
- Regénère + commite `web/ui/dist/`

**Interfaces:**
- Consumes : endpoint `/api/costs` → `CostSummary[]`.
- Produces : `getCosts()` ; `Gauge.svelte` (props `value: number`, `max: number`, `label?: string`, `variant?: "brass" | "warning"`).

- [ ] **Step 1 : type + getter dans `api.ts`** (aligné `src/web/api.rs`) :
```ts
export interface CostSummary { agent: string; total_runs: number; total_cost: number; total_tokens_in: number; total_tokens_out: number; }
export const getCosts = () => getJson<CostSummary[]>("/api/costs");
```

- [ ] **Step 2 : `Gauge.svelte`** — barre horizontale (piste `--viz-track`, remplissage dégradé laiton ou warning), tokens de la démo (`.gauge`/`.gauge > i`) :
```svelte
<script lang="ts">
  let { value, max, label = "", variant = "brass" }: { value: number; max: number; label?: string; variant?: "brass" | "warning" } = $props();
  const pct = $derived(max > 0 ? Math.min(100, Math.round((value / max) * 100)) : 0);
</script>
{#if label}<div class="eyebrow">{label}</div>{/if}
<div class="gauge" class:warn={variant === "warning"}><i style="width:{pct}%"></i></div>
<style>
  .eyebrow { font-size: var(--text-2xs); letter-spacing: var(--tracking-caps); text-transform: uppercase; color: var(--text-muted); }
  .gauge { height: 6px; border-radius: 3px; background: var(--viz-track); margin-top: 8px; overflow: hidden; }
  .gauge > i { display: block; height: 100%; border-radius: 3px; background: linear-gradient(90deg, var(--brass-dim), var(--brass)); }
  .gauge.warn > i { background: linear-gradient(90deg, var(--signal-warning), var(--signal-warning-fg)); }
</style>
```

- [ ] **Step 3 : `Costs.svelte`** — charge `getCosts()` (`$state` + `onMount`, état de chargement). Affiche : une rangée de métriques agrégées (total runs, coût total, tokens totaux — sommés côté client via `$derived`) en `.mono` tabular-nums, puis un tableau par agent (agent, runs, coût, tokens in/out) où chaque ligne a une `Gauge` de coût relative au coût max des agents (`variant="brass"`). Styles `.panel`/`table`/`.metric` de la démo.

- [ ] **Step 4 : brancher `Costs` dans `App.svelte`** — remplacer le placeholder `costs` par `<Costs />`.

- [ ] **Step 5 : build + gate** — `cd web/ui && npm run check` (0/0) + `npm run build` → `dist/`. `cargo test --no-default-features --features tui,web,storage web::tests` + clippy `tui,web,storage --all-targets` (0) + `cargo fmt -- --check`.

- [ ] **Step 6 : Commit**
```bash
git add web/ui/src web/ui/dist
git commit -m "feat(web): Costs view with reusable Gauge component (Svelte)"
```

---

### Task 2: Vue Models (groupée par provider)

**Files:**
- Modify: `web/ui/src/lib/api.ts` (types + getter), `web/ui/src/App.svelte` (brancher Models)
- Create: `web/ui/src/views/Models.svelte`
- Regénère + commite `web/ui/dist/`

**Interfaces:**
- Consumes : endpoint `/api/models` → `ProviderModels[]`.
- Produces : `getModels()`.

- [ ] **Step 1 : types + getter dans `api.ts`** (alignés `src/web/api.rs`) :
```ts
export interface ModelSummary { id: string; name: string | null; context: number | null; max_output: number | null; cost_input: number | null; cost_output: number | null; }
export interface ProviderModels { provider: string; models: ModelSummary[]; }
export const getModels = () => getJson<ProviderModels[]>("/api/models");
```

- [ ] **Step 2 : `Models.svelte`** — charge `getModels()` (`$state`+`onMount`, état de chargement). Rend un `.panel` par provider (en-tête = nom du provider + compteur de modèles en `.mono`), avec une table dense : id/name, contexte (formaté, ex. `128k` via un petit helper, tabular-nums mono), max output, coût in/out ($ par 1M tokens, `.mono`). Gérer les `null` (afficher `—`). Styles de la démo.

- [ ] **Step 3 : brancher `Models` dans `App.svelte`** — remplacer le placeholder `models` par `<Models />`. (Après cette tâche, tous les onglets de la nav ont une vraie vue ; il ne reste plus de placeholder « à venir ».)

- [ ] **Step 4 : build + gate** — `npm run check` (0/0) + `npm run build` → `dist/`. `cargo test … web::tests` + clippy 3 modes + fmt. `armadai web` → `/next` : Costs (jauges) + Models (groupé) affichent les données réelles. **Validation visuelle Dimitri.**

- [ ] **Step 5 : Commit**
```bash
git add web/ui/src web/ui/dist
git commit -m "feat(web): Models catalog view grouped by provider (Svelte)"
```

---

## Self-Review
- **Couverture** : Costs + jauge réutilisable (Task 1) ✓ ; Models groupé par provider (Task 2) ✓ ; tous les onglets ont désormais une vraie vue (plus de placeholder) ✓ ; polices intouchées (@fontsource) ✓ ; `/`/`api.rs` intouchés ✓.
- **Placeholders** : `Gauge.svelte`/`api.ts` complets ; Costs/Models décrits concrètement (patron Agents/Prompts). Helper de format contexte (`128k`) = `Math.round(n/1000)+"k"` (trivial, à écrire dans la vue).
- **Cohérence types** : `CostSummary`/`ModelSummary`/`ProviderModels` alignés `api.rs`↔`api.ts` ; `Gauge` props cohérentes.
