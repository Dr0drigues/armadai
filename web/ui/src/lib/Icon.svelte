<script lang="ts">
  import { ICONS } from "./icons";

  let { name, size = 16 }: { name: string; size?: number } = $props();
  const nodes = $derived(ICONS[name] ?? []);
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width="2"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
>
  {#each nodes as n}
    {#if n.tag === "path"}
      <path {...n.attrs} />
    {:else if n.tag === "circle"}
      <circle {...n.attrs} />
    {:else if n.tag === "line"}
      <line {...n.attrs} />
    {:else if n.tag === "polyline"}
      <polyline {...n.attrs} />
    {:else if n.tag === "polygon"}
      <polygon {...n.attrs} />
    {:else if n.tag === "rect"}
      <rect {...n.attrs} />
    {/if}
  {/each}
</svg>

<style>
  svg {
    display: inline-block;
  }
</style>
