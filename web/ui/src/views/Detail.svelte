<script lang="ts">
  import { navigate } from "../lib/route.svelte";
  import { getDetail } from "../lib/api";
  import Markdown from "../lib/Markdown.svelte";

  let {
    kind = "agents",
    name = "",
  }: { kind?: string; name?: string } = $props();

  let data = $state<Record<string, unknown> | null>(null);
  let loading = $state(true);
  let error = $state<string | null>(null);

  // Reload whenever kind/name changes (navigating between details of the same
  // kind, e.g. agents/foo -> agents/bar, only changes the prop — not a remount).
  $effect(() => {
    const k = kind;
    const n = name;
    if (!n) {
      data = null;
      error = "No name specified";
      loading = false;
      return;
    }
    loading = true;
    error = null;
    getDetail(k, n)
      .then((d) => {
        data = d;
      })
      .catch((e) => {
        error = e instanceof Error ? e.message : "Failed to load detail";
      })
      .finally(() => {
        loading = false;
      });
  });

  function renderValue(value: unknown): string {
    if (typeof value === "string") return value;
    if (typeof value === "number") return String(value);
    if (typeof value === "boolean") return value ? "true" : "false";
    if (value === null) return "—";
    if (Array.isArray(value)) return value.join(", ");
    return JSON.stringify(value, null, 2);
  }

  function isLongText(value: unknown): boolean {
    if (typeof value !== "string") return false;
    return value.length > 100 || value.includes("\n");
  }

  function isArray(value: unknown): boolean {
    return Array.isArray(value);
  }

  function isObject(value: unknown): boolean {
    return typeof value === "object" && value !== null && !Array.isArray(value);
  }
</script>

<div class="detail-container">
  {#if loading}
    <div class="panel">
      <p>…</p>
    </div>
  {:else if error}
    <div class="panel error">
      <p>Error: {error}</p>
    </div>
  {:else if data}
    <div class="detail">
      <div class="detail-header">
        <button
          class="back-btn"
          onclick={() => navigate(kind)}
          aria-label="Back"
        >
          ← Back
        </button>
        <h1>{name}</h1>
      </div>

      <div class="detail-content">
        {#each Object.entries(data) as [key, value]}
          <div class="field">
            <div class="field-name">{key}</div>
            <div class="field-value">
              {#if key === "description" && isLongText(value)}
                <Markdown source={String(value)} />
              {:else if isArray(value)}
                {#if (value as unknown[]).every((i) => typeof i === "string" || typeof i === "number")}
                  <div class="tags">
                    {#each value as unknown[] as item (item)}
                      <span class="tag">{renderValue(item)}</span>
                    {/each}
                  </div>
                {:else}
                  <div class="ref-list">
                    {#each value as unknown[] as item, i (i)}
                      {#if item && typeof item === "object" && typeof (item as Record<string, unknown>).content === "string"}
                        <div class="ref-item">
                          <Markdown source={(item as Record<string, unknown>).content as string} />
                        </div>
                      {:else}
                        <pre>{JSON.stringify(item, null, 2)}</pre>
                      {/if}
                    {/each}
                  </div>
                {/if}
              {:else if isObject(value)}
                <pre>{JSON.stringify(value, null, 2)}</pre>
              {:else if isLongText(value)}
                <Markdown source={String(value)} />
              {:else}
                <span>{renderValue(value)}</span>
              {/if}
            </div>
          </div>
        {/each}
      </div>
    </div>
  {:else}
    <div class="panel">
      <p>No data found.</p>
    </div>
  {/if}
</div>

<style>
  .detail-container {
    margin-bottom: var(--gutter);
  }

  .detail {
    display: flex;
    flex-direction: column;
    gap: var(--gutter);
  }

  .detail-header {
    display: flex;
    align-items: center;
    gap: 16px;
  }

  .back-btn {
    appearance: none;
    background: var(--surface-2);
    border: 1px solid var(--border);
    padding: 8px 12px;
    border-radius: var(--radius);
    cursor: pointer;
    font-size: var(--text-sm);
    color: var(--text-primary);
    font-weight: 500;
    transition: all 150ms ease;
  }

  .back-btn:hover {
    background: var(--surface-3);
    border-color: var(--border);
  }

  .back-btn:focus {
    outline: 2px solid var(--brass);
    outline-offset: 2px;
  }

  .detail-header h1 {
    font-size: var(--text-2xl);
    font-weight: 700;
    margin: 0;
  }

  .detail-content {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .field {
    padding: 12px;
    border: 1px solid var(--border-faint);
    border-radius: var(--radius);
    background: var(--surface-1);
  }

  .field-name {
    font-weight: 600;
    font-size: var(--text-sm);
    color: var(--text-primary);
    margin-bottom: 8px;
    text-transform: uppercase;
    letter-spacing: var(--tracking-caps);
  }

  .field-value {
    color: var(--text-secondary);
    font-size: var(--text-sm);
  }

  .tags {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }

  .tag {
    display: inline-block;
    padding: 4px 8px;
    background: var(--brass-bg);
    color: var(--brass);
    border-radius: 3px;
    font-size: var(--text-2xs);
    font-weight: 500;
  }

  pre {
    background: var(--surface-3);
    padding: var(--panel-pad);
    border-radius: 6px;
    overflow-x: auto;
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    margin: 0;
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
    border-color: var(--signal-critical-bg);
  }

  .panel.error p {
    color: var(--signal-critical-fg);
  }

  .ref-list {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .ref-item {
    border: 1px solid var(--border-faint);
    border-radius: 6px;
    padding: 10px 14px;
    background: var(--surface-2);
  }
</style>
