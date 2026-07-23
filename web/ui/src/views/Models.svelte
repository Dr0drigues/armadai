<script lang="ts">
  import { onMount } from "svelte";
  import { getModels, fmtContext, fmtCost, type ProviderModels } from "../lib/api";

  let providerModels = $state<ProviderModels[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);

  onMount(async () => {
    try {
      providerModels = await getModels();
    } catch (e) {
      error = e instanceof Error ? e.message : "Failed to load models";
    } finally {
      loading = false;
    }
  });

  function formatCost(n: number | null): string {
    if (n === null) return "—";
    return fmtCost(n);
  }
</script>

<div class="models-container">
  {#if loading}
    <div class="panel">
      <p>…</p>
    </div>
  {:else if error}
    <div class="panel error">
      <p>Error: {error}</p>
    </div>
  {:else if providerModels.length === 0}
    <div class="panel">
      <p>No models available.</p>
    </div>
  {:else}
    {#each providerModels as providerGroup (providerGroup.provider)}
      <div class="panel">
        <div class="panel-head">
          <h2>{providerGroup.provider}</h2>
          <span class="mono count">{providerGroup.models.length} model{providerGroup.models.length !== 1 ? "s" : ""}</span>
        </div>
        <table>
          <thead>
            <tr>
              <th>Model ID</th>
              <th>Name</th>
              <th class="num">Context</th>
              <th class="num">Max Output</th>
              <th class="num">Cost Input</th>
              <th class="num">Cost Output</th>
            </tr>
          </thead>
          <tbody>
            {#each providerGroup.models as model (model.id)}
              <tr>
                <td class="mono" style="font-size: var(--text-xs);">{model.id}</td>
                <td>{model.name ?? "—"}</td>
                <td class="num mono">{fmtContext(model.context)}</td>
                <td class="num mono">{fmtContext(model.max_output)}</td>
                <td class="num mono">{formatCost(model.cost_input)}/1M</td>
                <td class="num mono">{formatCost(model.cost_output)}/1M</td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    {/each}
  {/if}
</div>

<style>
  .models-container {
    margin-bottom: var(--gutter);
    display: flex;
    flex-direction: column;
    gap: 14px;
  }

  .panel {
    background: var(--surface-1);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: var(--panel-pad);
  }

  .panel.error {
    border-color: var(--signal-critical);
    color: var(--signal-critical-fg);
  }

  .panel-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 12px;
  }

  .panel-head h2 {
    font-size: var(--text-md);
    font-weight: 600;
  }

  .count {
    font-size: var(--text-sm);
    color: var(--text-muted);
  }

  table {
    width: 100%;
    border-collapse: collapse;
  }

  thead th {
    text-align: left;
    font-size: var(--text-2xs);
    letter-spacing: var(--tracking-caps);
    text-transform: uppercase;
    color: var(--text-muted);
    font-weight: 600;
    padding: 0 10px 8px;
    border-bottom: 1px solid var(--border);
  }

  tbody td {
    height: var(--row-h);
    padding: 0 10px;
    border-bottom: 1px solid var(--border-faint);
    font-size: var(--text-sm);
  }

  tbody tr:hover {
    background: var(--surface-2);
  }

  td.num {
    text-align: right;
    font-family: var(--font-mono);
    font-variant-numeric: tabular-nums;
    color: var(--text-secondary);
  }

  .mono {
    font-family: var(--font-mono);
    font-variant-numeric: tabular-nums;
  }
</style>
