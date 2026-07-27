<script lang="ts">
  import { onMount } from "svelte";
  import { getAgents } from "../lib/api";
  import { navigate } from "../lib/route.svelte";
  import type { AgentSummary } from "../lib/api";

  let agents = $state<AgentSummary[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);

  onMount(async () => {
    try {
      agents = await getAgents();
    } catch (e) {
      error = e instanceof Error ? e.message : "Failed to load agents";
    } finally {
      loading = false;
    }
  });

  function getInitials(name: string): string {
    return name
      .split(/\s+/)
      .slice(0, 2)
      .map((w) => w[0].toUpperCase())
      .join("");
  }
</script>

<div class="agents-container">
  {#if loading}
    <div class="panel">
      <p>…</p>
    </div>
  {:else if error}
    <div class="panel error">
      <p>Error: {error}</p>
    </div>
  {:else if agents.length === 0}
    <div class="panel">
      <p>No agents found.</p>
    </div>
  {:else}
    <div class="agents-list">
      {#each agents as agent (agent.name)}
        <div
          class="agent"
          role="button"
          tabindex="0"
          onclick={() => navigate(`agents/${encodeURIComponent(agent.name)}`)}
          onkeydown={(e) => {
            if (e.key === "Enter") {
              navigate(`agents/${encodeURIComponent(agent.name)}`);
            }
          }}
        >
          <div class="av">{getInitials(agent.name)}</div>
          <div class="who">
            <div class="n">{agent.name}</div>
            <div class="m">
              {agent.provider}
              <span style="margin: 0 4px;">·</span>
              {agent.model}
            </div>
          </div>
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .agents-container {
    margin-bottom: var(--gutter);
  }

  .agents-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .agent {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px;
    border-radius: var(--radius);
    border: 1px solid var(--border-faint);
    cursor: pointer;
  }

  .agent:hover {
    border-color: var(--border);
    background: var(--surface-2);
  }

  .agent:focus {
    outline: 2px solid var(--brass);
    outline-offset: -1px;
  }

  .agent .av {
    width: 30px;
    height: 30px;
    border-radius: 6px;
    background: var(--surface-3);
    display: grid;
    place-items: center;
    color: var(--brass);
    font-weight: 700;
    font-family: var(--font-mono);
    font-size: var(--text-sm);
    border: 1px solid var(--border);
  }

  .agent .who {
    flex: 1;
    min-width: 0;
  }

  .agent .who .n {
    font-weight: 600;
    font-size: var(--text-md);
  }

  .agent .who .m {
    color: var(--text-faint);
    font-size: var(--text-xs);
    font-family: var(--font-mono);
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
