<script lang="ts">
  import Shell from "./lib/Shell.svelte";

  let active = $state("agents");

  const tabs = [
    { id: "agents", label: "Agents", count: 6 },
    { id: "prompts", label: "Prompts", count: 12 },
    { id: "skills", label: "Skills", count: 8 },
    { id: "starters", label: "Starters", count: 5 },
    { id: "history", label: "History", count: 148 },
    { id: "costs", label: "Costs" },
    { id: "models", label: "Models", count: 37 },
  ];

  function handleSelect(id: string) {
    active = id;
  }
</script>

<Shell {tabs} {active} onselect={handleSelect}>
  <div class="page-head">
    <h1>
      {#if active === "agents"}
        Agents
      {:else if active === "prompts"}
        Prompts
      {:else if active === "skills"}
        Skills
      {:else if active === "starters"}
        Starters
      {:else if active === "history"}
        History
      {:else if active === "costs"}
        Costs
      {:else if active === "models"}
        Models
      {:else}
        {active}
      {/if}
    </h1>
    <span class="sub">Page de la flotte ArmadAI</span>
  </div>

  {#if active === "agents"}
    <div class="panel">
      <p>Agents — coming in Task 4.</p>
    </div>
  {:else if active === "history"}
    <div class="panel">
      <p>History — coming in Task 4.</p>
    </div>
  {:else}
    <div class="panel">
      <p>Vue « {active} » — à venir en W2.</p>
    </div>
  {/if}
</Shell>

<style>
  .page-head {
    display: flex;
    align-items: baseline;
    gap: 12px;
    margin-bottom: var(--gutter);
  }

  .page-head h1 {
    font-size: var(--text-2xl);
    font-weight: 700;
    letter-spacing: -0.01em;
  }

  .page-head .sub {
    color: var(--text-muted);
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
  }
</style>
