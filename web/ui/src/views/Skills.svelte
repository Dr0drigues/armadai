<script lang="ts">
  import { onMount } from "svelte";
  import { getSkills } from "../lib/api";
  import type { SkillSummary } from "../lib/api";

  let skills = $state<SkillSummary[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);

  onMount(async () => {
    try {
      skills = await getSkills();
    } catch (e) {
      error = e instanceof Error ? e.message : "Failed to load skills";
    } finally {
      loading = false;
    }
  });
</script>

<div class="skills-container">
  {#if loading}
    <div class="panel">
      <p>…</p>
    </div>
  {:else if error}
    <div class="panel error">
      <p>Error: {error}</p>
    </div>
  {:else if skills.length === 0}
    <div class="panel">
      <p>No skills found.</p>
    </div>
  {:else}
    <div class="skills-list">
      {#each skills as skill (skill.name)}
        <div class="skill">
          <div class="who">
            <div class="n">
              {skill.name}
              {#if skill.version}
                <span class="version">v{skill.version}</span>
              {/if}
            </div>
            {#if skill.description}
              <div class="d">{skill.description}</div>
            {/if}
          </div>
          {#if skill.tools && skill.tools.length > 0}
            <div class="tags">
              {#each skill.tools as tool}
                <span class="tag">{tool}</span>
              {/each}
            </div>
          {/if}
          <div class="source mono eyebrow">{skill.source}</div>
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .skills-container {
    margin-bottom: var(--gutter);
  }

  .skills-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .skill {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px;
    border-radius: var(--radius);
    border: 1px solid var(--border-faint);
  }

  .skill:hover {
    border-color: var(--border);
    background: var(--surface-2);
  }

  .skill .who {
    flex: 1;
    min-width: 0;
  }

  .skill .who .n {
    font-weight: 600;
    font-size: var(--text-md);
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .version {
    font-size: var(--text-2xs);
    color: var(--text-faint);
    font-weight: 400;
  }

  .skill .who .d {
    color: var(--text-faint);
    font-size: var(--text-xs);
    margin-top: 4px;
  }

  .skill .tags {
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

  .skill .source {
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
