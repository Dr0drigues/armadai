<script lang="ts">
  import { onMount } from "svelte";
  import { getStarters } from "../lib/api";
  import { navigate } from "../lib/route.svelte";
  import type { StarterSummary } from "../lib/api";

  let starters = $state<StarterSummary[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);

  onMount(async () => {
    try {
      starters = await getStarters();
    } catch (e) {
      error = e instanceof Error ? e.message : "Failed to load starters";
    } finally {
      loading = false;
    }
  });
</script>

<div class="starters-container">
  {#if loading}
    <div class="panel">
      <p>…</p>
    </div>
  {:else if error}
    <div class="panel error">
      <p>Error: {error}</p>
    </div>
  {:else if starters.length === 0}
    <div class="panel">
      <p>No starters found.</p>
    </div>
  {:else}
    <div class="starters-grid">
      {#each starters as starter (starter.name)}
        <div
          class="starter-card"
          role="button"
          tabindex="0"
          onclick={() => navigate(`starters/${encodeURIComponent(starter.name)}`)}
          onkeydown={(e) => {
            if (e.key === "Enter") {
              navigate(`starters/${encodeURIComponent(starter.name)}`);
            }
          }}
        >
          <div class="header">
            <h3>{starter.name}</h3>
          </div>
          {#if starter.description}
            <p class="desc">{starter.description}</p>
          {/if}
          <div class="stats">
            <div class="stat">
              <span class="label">Agents</span>
              <span class="value mono">{starter.agents_count}</span>
            </div>
            <div class="stat">
              <span class="label">Prompts</span>
              <span class="value mono">{starter.prompts_count}</span>
            </div>
            <div class="stat">
              <span class="label">Skills</span>
              <span class="value mono">{starter.skills_count}</span>
            </div>
          </div>
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .starters-container {
    margin-bottom: var(--gutter);
  }

  .starters-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(280px, 1fr));
    gap: 12px;
  }

  .starter-card {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 14px;
    border: 1px solid var(--border-faint);
    border-radius: var(--radius);
    background: var(--surface-1);
    cursor: pointer;
  }

  .starter-card:hover {
    border-color: var(--border);
    background: var(--surface-2);
  }

  .starter-card:focus {
    outline: 2px solid var(--brass);
    outline-offset: -1px;
  }

  .starter-card .header {
    margin: 0;
  }

  .starter-card h3 {
    font-size: var(--text-md);
    font-weight: 600;
    margin: 0;
    color: var(--text-primary);
  }

  .starter-card .desc {
    font-size: var(--text-sm);
    color: var(--text-secondary);
    margin: 0;
    line-height: 1.4;
  }

  .starter-card .stats {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 8px;
    padding-top: 8px;
    border-top: 1px solid var(--border-faint);
  }

  .stat {
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
  }

  .stat .label {
    font-size: var(--text-2xs);
    color: var(--text-faint);
    text-transform: uppercase;
    letter-spacing: var(--tracking-caps);
    margin-bottom: 4px;
    font-weight: 500;
  }

  .stat .value {
    font-size: var(--text-lg);
    font-weight: 600;
    color: var(--brass);
  }

  .panel {
    background: var(--surface-1);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: var(--panel-pad);
  }

  .panel p {
    color: var(--text-secondary);
  }

  .panel.error {
    border-color: var(--signal-critical-bg);
  }

  .panel.error p {
    color: var(--signal-critical-fg);
  }
</style>
