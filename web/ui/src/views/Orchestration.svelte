<script lang="ts">
  import { onMount } from "svelte";
  import { router, navigate } from "../lib/route.svelte";
  import { getTopology, getTraces, getTraceDetail } from "../lib/api";
  import Topology from "../lib/Topology.svelte";

  let topology = $state(null as any);
  let traces = $state(null as any);
  let traceDetail = $state(null as any);
  let loading = $state(true);
  let error = $state("");

  const r = $derived(router.current);

  onMount(async () => {
    try {
      [topology, traces] = await Promise.all([getTopology(), getTraces()]);
      if (r.param) {
        traceDetail = await getTraceDetail(r.param);
      }
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
  });

</script>

{#if loading}
  <div class="panel">
    <p>Chargement…</p>
  </div>
{:else if error}
  <div class="panel error">
    <p><strong>Erreur</strong></p>
    <pre>{error}</pre>
  </div>
{:else if r.param && traceDetail}
  <!-- Detail view -->
  <div class="detail-view">
    <button class="btn-back" onclick={() => navigate("orchestration")}>← Retour</button>
    <h2>{r.param}</h2>
    <div class="panel">
      {#each Object.entries(traceDetail) as [key, value]}
        <div class="detail-row">
          <span class="key">{key}</span>
          <span class="value">
            {#if typeof value === "object"}
              <pre>{JSON.stringify(value, null, 2)}</pre>
            {:else}
              {value}
            {/if}
          </span>
        </div>
      {/each}
    </div>
  </div>
{:else}
  <!-- List and topology view -->
  <div class="orchestration-view">
    {#if topology && topology.enabled}
      <div class="section">
        <h2>Topologie</h2>
        <Topology {topology} />
      </div>
    {:else}
      <div class="panel">
        <p>Aucune orchestration configurée.</p>
      </div>
    {/if}

    {#if traces && traces.length > 0}
      <div class="section">
        <h2>Traces récentes</h2>
        <div class="traces-list">
          {#each traces as trace, idx}
            <div
              class="trace-item"
              role="button"
              tabindex="0"
              onclick={() => navigate(`orchestration/${trace.id || idx}`)}
              onkeydown={(e) => e.key === "Enter" && navigate(`orchestration/${trace.id || idx}`)}
            >
              <span class="trace-id">{trace.id || trace.run_id || `Run ${idx}`}</span>
              <span class="trace-meta">
                {#if trace.status}
                  <span class="badge">{trace.status}</span>
                {/if}
                {#if trace.timestamp}
                  <span class="timestamp">{new Date(trace.timestamp).toLocaleString("fr-FR")}</span>
                {/if}
              </span>
            </div>
          {/each}
        </div>
      </div>
    {:else}
      <div class="panel">
        <p>Pas de traces disponibles.</p>
      </div>
    {/if}
  </div>
{/if}

<style>
  .orchestration-view {
    display: flex;
    flex-direction: column;
    gap: var(--gutter);
  }

  .section h2 {
    font-size: var(--text-lg);
    font-weight: 600;
    margin: 0 0 12px;
    color: var(--text-primary);
  }

  .detail-view {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .btn-back {
    background: transparent;
    border: 1px solid var(--border);
    color: var(--text-secondary);
    padding: 8px 12px;
    border-radius: var(--radius);
    cursor: pointer;
    font-size: var(--text-sm);
    font-family: var(--font-ui);
    width: fit-content;
  }

  .btn-back:hover {
    background: var(--surface-2);
    border-color: var(--border-strong);
    color: var(--text-primary);
  }

  .detail-view h2 {
    font-size: var(--text-xl);
    font-weight: 600;
    margin: 0;
  }

  .detail-row {
    display: grid;
    grid-template-columns: 120px 1fr;
    gap: 12px;
    padding: 8px 0;
    border-bottom: 1px solid var(--border);
  }

  .detail-row:last-child {
    border-bottom: none;
  }

  .detail-row .key {
    font-weight: 500;
    color: var(--text-primary);
    font-family: var(--font-mono);
    font-size: var(--text-sm);
  }

  .detail-row .value {
    color: var(--text-secondary);
    word-break: break-all;
  }

  .detail-row pre {
    background: var(--surface-2);
    border-radius: 4px;
    padding: 8px;
    font-size: var(--text-xs);
    overflow-x: auto;
    margin: 0;
  }

  .traces-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .trace-item {
    display: grid;
    grid-template-columns: 1fr auto;
    align-items: center;
    gap: 12px;
    padding: 12px;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    cursor: pointer;
    transition: background-color 0.15s, border-color 0.15s;
  }

  .trace-item:hover {
    background: var(--surface-3);
    border-color: var(--border-strong);
  }

  .trace-item:focus-visible {
    outline: 2px solid var(--focus-ring);
    outline-offset: 1px;
  }

  .trace-id {
    font-family: var(--font-mono);
    font-size: var(--text-sm);
    font-weight: 500;
    color: var(--text-primary);
  }

  .trace-meta {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: var(--text-sm);
  }

  .badge {
    background: var(--surface-1);
    color: var(--text-secondary);
    padding: 2px 8px;
    border-radius: 3px;
    font-size: var(--text-2xs);
    font-weight: 500;
  }

  .timestamp {
    color: var(--text-faint);
    font-size: var(--text-sm);
  }

  .panel {
    background: var(--surface-1);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: var(--panel-pad);
  }

  .panel p {
    color: var(--text-secondary);
    margin: 0;
  }

  .panel.error {
    border-color: var(--border);
  }

  .panel.error pre {
    background: var(--surface-2);
    padding: 8px;
    border-radius: 4px;
    font-size: var(--text-xs);
    overflow-x: auto;
    margin: 0;
    color: var(--text-secondary);
  }
</style>
