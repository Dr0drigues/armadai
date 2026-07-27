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
