<script lang="ts">
  import { onMount } from "svelte";
  import { getHistory, fmtTokens, fmtCost } from "../lib/api";
  import type { HistoryEntry } from "../lib/api";

  let entries = $state<HistoryEntry[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);

  onMount(async () => {
    try {
      entries = await getHistory();
    } catch (e) {
      error = e instanceof Error ? e.message : "Failed to load history";
    } finally {
      loading = false;
    }
  });

  function statusClass(status: string): string {
    if (status === "success") return "ok";
    if (status === "running") return "running";
    if (status === "halted") return "halted";
    return "warning";
  }

  function statusLabel(status: string): string {
    if (status === "success") return "ok";
    return status;
  }
</script>

<div class="history-container">
  {#if loading}
    <div class="panel">
      <p>…</p>
    </div>
  {:else if error}
    <div class="panel error">
      <p>Error: {error}</p>
    </div>
  {:else if entries.length === 0}
    <div class="panel">
      <p>No history entries found.</p>
    </div>
  {:else}
    <div class="panel">
      <table>
        <thead>
          <tr>
            <th>Agent</th>
            <th>Provider · Model</th>
            <th>State</th>
            <th class="num">Tokens</th>
            <th class="num">Cost</th>
            <th class="num">Duration</th>
          </tr>
        </thead>
        <tbody>
          {#each entries as entry (entry.agent + entry.duration_ms)}
            <tr>
              <td class="mono">{entry.agent}</td>
              <td class="mono">{entry.provider} · {entry.model}</td>
              <td>
                <span class="badge {statusClass(entry.status)}">
                  {statusLabel(entry.status)}
                </span>
              </td>
              <td class="num">{fmtTokens(entry.tokens_in + entry.tokens_out)}</td>
              <td class="num">{fmtCost(entry.cost)}</td>
              <td class="num">{entry.duration_ms}ms</td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {/if}
</div>

<style>
  .history-container {
    margin-bottom: var(--gutter);
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

  td.mono {
    font-family: var(--font-mono);
    font-variant-numeric: tabular-nums;
    color: var(--text-secondary);
  }

  .badge {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 20px;
    padding: 0 8px;
    border-radius: 4px;
    font-size: var(--text-2xs);
    font-weight: 600;
    letter-spacing: var(--tracking-wide);
    text-transform: uppercase;
  }

  .badge::before {
    content: "";
    width: 6px;
    height: 6px;
    border-radius: 50%;
  }

  .badge.running {
    background: var(--signal-running-bg);
    color: var(--signal-running-fg);
  }

  .badge.running::before {
    background: var(--signal-running);
    box-shadow: 0 0 6px var(--signal-running);
  }

  .badge.ok {
    background: var(--signal-ok-bg);
    color: var(--signal-ok-fg);
  }

  .badge.ok::before {
    background: var(--signal-ok);
  }

  .badge.halted {
    background: var(--signal-halted-bg);
    color: var(--signal-halted-fg);
  }

  .badge.halted::before {
    background: var(--signal-halted);
  }

  .badge.warning {
    background: var(--signal-warning-bg);
    color: var(--signal-warning-fg);
  }

  .badge.warning::before {
    background: var(--signal-warning);
  }
</style>
