<script lang="ts">
  import { marked } from "marked";

  let { source = "" }: { source?: string } = $props();

  // Security note: marked.parse() does not sanitize HTML; the source comes from
  // local config (agents/prompts/skills metadata), considered trusted. Do not use
  // with untrusted user input.
  const html = $derived(
    source ? (marked.parse(source, { async: false }) as string) : ""
  );
</script>

<div class="md">{@html html}</div>

<style>
  .md :global(h1),
  .md :global(h2),
  .md :global(h3) {
    font-weight: 600;
    margin: 0.6em 0 0.3em;
  }

  .md :global(p) {
    margin: 0.4em 0;
    color: var(--text-secondary);
  }

  .md :global(ul),
  .md :global(ol) {
    margin: 0.4em 0;
    padding-left: 1.5em;
    color: var(--text-secondary);
  }

  .md :global(li) {
    margin: 0.15em 0;
  }

  .md :global(code) {
    font-family: var(--font-mono);
    font-size: var(--text-sm);
    background: var(--surface-3);
    padding: 1px 4px;
    border-radius: 3px;
  }

  .md :global(pre) {
    background: var(--surface-3);
    padding: var(--panel-pad);
    border-radius: 6px;
    overflow-x: auto;
  }

  .md :global(a) {
    color: var(--brass);
  }
</style>
