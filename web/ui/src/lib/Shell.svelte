<script lang="ts">
  import { onMount, type Snippet } from "svelte";
  import { navigate } from "./route.svelte";
  import { theme } from "./theme.svelte";
  import Icon from "./Icon.svelte";

  interface Tab {
    id: string;
    label: string;
    count?: number;
    icon?: string;
  }

  let {
    tabs = [] as Tab[],
    active = "agents",
    children,
  }: {
    tabs?: Tab[];
    active?: string;
    children?: Snippet;
  } = $props();

  onMount(() => {
    theme.init();
  });
</script>

<div class="topbar">
  <div class="wordmark">
    <svg class="mark" width="21" height="21" viewBox="0 0 64 64" aria-hidden="true">
      <circle cx="32" cy="32" r="22" fill="none" stroke="var(--brass)" stroke-width="2.2" opacity="0.5" />
      <polygon points="32,10 43,31 21,31" fill="var(--brass)" />
      <g stroke="var(--brass-dim)" stroke-width="3" stroke-linecap="round">
        <line x1="32" y1="37" x2="32" y2="52" />
        <line x1="17" y1="32" x2="24" y2="32" />
        <line x1="47" y1="32" x2="40" y2="32" />
      </g>
      <circle cx="32" cy="32" r="3.2" fill="var(--brass-strong)" />
    </svg>
    <span class="name">Armad<b>AI</b></span>
    <span class="tag">pont de commandement</span>
  </div>
  <div class="topbar-spacer"></div>
  <button class="btn-ghost" onclick={() => theme.toggle()} aria-label="Basculer le thème">
    {theme.value === "dark" ? "☀" : "◐"} Thème
  </button>
</div>

<div class="shell">
  <nav class="sidebar" aria-label="Navigation">
    <div class="nav-section eyebrow">Flotte</div>
    {#each tabs as tab}
      <div
        class="nav-item"
        class:active={active === tab.id}
        role="button"
        tabindex="0"
        onclick={() => navigate(tab.id)}
        onkeydown={(e) => e.key === "Enter" && navigate(tab.id)}
      >
        <Icon name={tab.icon ?? tab.id} size={16} />
        {tab.label}
        {#if tab.count !== undefined}
          <span class="count mono">{tab.count}</span>
        {/if}
      </div>
    {/each}
  </nav>

  <main class="main">
    {@render children?.()}
  </main>
</div>

<style>
  .topbar {
    height: 52px;
    display: flex;
    align-items: center;
    gap: var(--gutter);
    padding: 0 20px;
    background: var(--surface-2);
    border-bottom: 1px solid var(--border);
    position: sticky;
    top: 0;
    z-index: 10;
  }

  .wordmark {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .wordmark .mark {
    flex: none;
  }

  .wordmark .name {
    font-weight: 700;
    letter-spacing: -0.02em;
    font-size: var(--text-lg);
    white-space: nowrap;
  }

  .wordmark .name :global(b) {
    color: var(--brass);
    font-weight: 700;
  }

  .wordmark .tag {
    font-size: var(--text-2xs);
    color: var(--text-faint);
    letter-spacing: var(--tracking-caps);
    text-transform: uppercase;
  }

  .topbar-spacer {
    flex: 1;
  }

  .btn-ghost {
    background: transparent;
    border: 1px solid var(--border);
    color: var(--text-secondary);
    height: 30px;
    padding: 0 12px;
    border-radius: var(--radius);
    cursor: pointer;
    font-family: var(--font-ui);
    font-size: var(--text-sm);
  }

  .btn-ghost:hover {
    border-color: var(--border-strong);
    color: var(--text-primary);
    background: var(--surface-3);
  }

  .btn-ghost:focus-visible {
    outline: 2px solid var(--focus-ring);
    outline-offset: 1px;
  }

  .shell {
    display: grid;
    grid-template-columns: var(--sidebar-w) 1fr;
    min-height: calc(100vh - 52px);
  }

  .sidebar {
    background: var(--surface-1);
    border-right: 1px solid var(--border);
    padding: 14px 10px;
  }

  .nav-section {
    margin-bottom: 6px;
    padding: 8px 10px 4px;
  }

  .nav-item {
    display: flex;
    align-items: center;
    gap: 10px;
    height: 34px;
    padding: 0 10px;
    border-radius: var(--radius);
    color: var(--text-secondary);
    cursor: pointer;
    font-size: var(--text-md);
    position: relative;
    transition: background-color 0.15s, color 0.15s;
  }

  .nav-item :global(svg) {
    width: 16px;
    height: 16px;
    stroke: currentColor;
    flex: none;
  }

  .nav-item:hover {
    background: var(--surface-3);
    color: var(--text-primary);
  }

  .nav-item.active {
    background: var(--brass-bg);
    color: var(--brass-strong);
    font-weight: 600;
  }

  .nav-item.active::before {
    content: "";
    position: absolute;
    left: -10px;
    top: 6px;
    bottom: 6px;
    width: 3px;
    background: var(--brass);
    border-radius: 0 2px 2px 0;
  }

  .nav-item .count {
    margin-left: auto;
    font-size: var(--text-2xs);
    color: var(--text-faint);
  }

  .main {
    padding: var(--gutter);
    max-width: 1440px;
  }

  @media (max-width: 960px) {
    .shell {
      grid-template-columns: 1fr;
    }

    .sidebar {
      display: none;
    }
  }
</style>
