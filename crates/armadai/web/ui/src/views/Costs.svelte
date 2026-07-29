<script lang="ts">
  import { onMount } from "svelte";
  import { getCosts, fmtCost, fmtTokens, type CostSummary } from "../lib/api";
  import Gauge from "../lib/Gauge.svelte";

  let costs = $state<CostSummary[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);

  onMount(async () => {
    try {
      costs = await getCosts();
    } catch (e) {
      error = e instanceof Error ? e.message : "Failed to load costs";
    } finally {
      loading = false;
    }
  });

  const totalRuns = $derived(costs.reduce((sum, c) => sum + c.total_runs, 0));
  const totalCost = $derived(costs.reduce((sum, c) => sum + c.total_cost, 0));
  const totalTokensIn = $derived(costs.reduce((sum, c) => sum + c.total_tokens_in, 0));
  const totalTokensOut = $derived(costs.reduce((sum, c) => sum + c.total_tokens_out, 0));
  const maxCost = $derived(costs.length > 0 ? Math.max(...costs.map(c => c.total_cost)) : 0);
</script>

<div class="costs-container">
  {#if loading}
    <div class="panel">
      <p>…</p>
    </div>
  {:else if error}
    <div class="panel error">
      <p>Error: {error}</p>
    </div>
  {:else if costs.length === 0}
    <div class="panel">
      <p>No cost data available.</p>
    </div>
  {:else}
    <div class="metrics">
      <div class="panel metric">
        <div class="label"><span class="eyebrow">Total Runs</span></div>
        <div class="val mono">{totalRuns}</div>
      </div>
      <div class="panel metric">
        <div class="label"><span class="eyebrow">Total Cost</span></div>
        <div class="val mono">{fmtCost(totalCost)}</div>
      </div>
      <div class="panel metric">
        <div class="label"><span class="eyebrow">Total Tokens</span></div>
        <div class="val mono">{fmtTokens(totalTokensIn + totalTokensOut)}</div>
      </div>
    </div>

    <div class="panel">
      <div class="panel-head"><h2>Costs by Agent</h2></div>
      <table>
        <thead>
          <tr>
            <th>Agent</th>
            <th class="num">Runs</th>
            <th class="num">Cost</th>
            <th class="num">Tokens In</th>
            <th class="num">Tokens Out</th>
            <th>Gauge</th>
          </tr>
        </thead>
        <tbody>
          {#each costs as cost (cost.agent)}
            <tr>
              <td>{cost.agent}</td>
              <td class="num">{cost.total_runs}</td>
              <td class="num">{fmtCost(cost.total_cost)}</td>
              <td class="num">{fmtTokens(cost.total_tokens_in)}</td>
              <td class="num">{fmtTokens(cost.total_tokens_out)}</td>
              <td class="gauge-cell">
                <Gauge value={cost.total_cost} max={maxCost} variant="brass" />
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {/if}
</div>

<style>
  .costs-container {
    margin-bottom: var(--gutter);
  }

  .metrics {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(190px, 1fr));
    gap: 14px;
    margin-bottom: var(--gutter);
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

  .metric .label {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }

  .metric .val {
    font-size: var(--text-3xl);
    font-weight: 600;
    margin-top: 6px;
    font-family: var(--font-mono);
    font-variant-numeric: tabular-nums;
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

  .gauge-cell {
    padding: 6px 10px !important;
  }

  .mono {
    font-family: var(--font-mono);
    font-variant-numeric: tabular-nums;
  }

  .eyebrow {
    font-size: var(--text-2xs);
    letter-spacing: var(--tracking-caps);
    text-transform: uppercase;
    color: var(--text-muted);
    font-weight: 600;
  }
</style>
