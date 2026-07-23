<script lang="ts">
  import { onMount } from "svelte";
  import { getPrompts } from "../lib/api";
  import type { PromptSummary } from "../lib/api";

  let prompts = $state<PromptSummary[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);

  onMount(async () => {
    try {
      prompts = await getPrompts();
    } catch (e) {
      error = e instanceof Error ? e.message : "Failed to load prompts";
    } finally {
      loading = false;
    }
  });
</script>

<div class="prompts-container">
  {#if loading}
    <div class="panel">
      <p>…</p>
    </div>
  {:else if error}
    <div class="panel error">
      <p>Error: {error}</p>
    </div>
  {:else if prompts.length === 0}
    <div class="panel">
      <p>No prompts found.</p>
    </div>
  {:else}
    <div class="prompts-list">
      {#each prompts as prompt (prompt.name)}
        <div class="prompt">
          <div class="who">
            <div class="n">{prompt.name}</div>
            {#if prompt.description}
              <div class="d">{prompt.description}</div>
            {/if}
          </div>
          {#if prompt.apply_to && prompt.apply_to.length > 0}
            <div class="tags">
              {#each prompt.apply_to as tag}
                <span class="tag">{tag}</span>
              {/each}
            </div>
          {/if}
          <div class="source mono eyebrow">{prompt.source}</div>
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .prompts-container {
    margin-bottom: var(--gutter);
  }

  .prompts-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .prompt {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px;
    border-radius: var(--radius);
    border: 1px solid var(--border-faint);
  }

  .prompt:hover {
    border-color: var(--border);
    background: var(--surface-2);
  }

  .prompt .who {
    flex: 1;
    min-width: 0;
  }

  .prompt .who .n {
    font-weight: 600;
    font-size: var(--text-md);
  }

  .prompt .who .d {
    color: var(--text-faint);
    font-size: var(--text-xs);
    margin-top: 4px;
  }

  .prompt .tags {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }

  .tag {
    display: inline-block;
    padding: 3px 8px;
    background: var(--brass-bg);
    color: var(--brass);
    border-radius: 3px;
    font-size: var(--text-2xs);
    font-weight: 500;
  }

  .prompt .source {
    color: var(--text-faint);
    font-size: var(--text-2xs);
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
