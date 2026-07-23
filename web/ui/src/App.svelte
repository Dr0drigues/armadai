<script lang="ts">
  import Shell from "./lib/Shell.svelte";
  import Agents from "./views/Agents.svelte";
  import History from "./views/History.svelte";
  import Prompts from "./views/Prompts.svelte";
  import Skills from "./views/Skills.svelte";
  import Starters from "./views/Starters.svelte";
  import Costs from "./views/Costs.svelte";
  import Models from "./views/Models.svelte";

  let active = $state("agents");

  const tabs = [
    { id: "agents", label: "Agents", count: 6, icon: "agents" },
    { id: "prompts", label: "Prompts", count: 12, icon: "prompts" },
    { id: "skills", label: "Skills", count: 8, icon: "skills" },
    { id: "starters", label: "Starters", count: 5, icon: "starters" },
    { id: "history", label: "History", count: 148, icon: "history" },
    { id: "costs", label: "Costs", icon: "costs" },
    { id: "models", label: "Models", count: 37, icon: "models" },
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
    <Agents />
  {:else if active === "prompts"}
    <Prompts />
  {:else if active === "skills"}
    <Skills />
  {:else if active === "starters"}
    <Starters />
  {:else if active === "history"}
    <History />
  {:else if active === "costs"}
    <Costs />
  {:else if active === "models"}
    <Models />
  {:else}
    <div class="panel">
      <p>Vue « {active} » — à venir.</p>
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
