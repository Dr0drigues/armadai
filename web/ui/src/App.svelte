<script lang="ts">
  import { onMount } from "svelte";
  import { router } from "./lib/route.svelte";
  import Shell from "./lib/Shell.svelte";
  import Agents from "./views/Agents.svelte";
  import History from "./views/History.svelte";
  import Prompts from "./views/Prompts.svelte";
  import Skills from "./views/Skills.svelte";
  import Starters from "./views/Starters.svelte";
  import Costs from "./views/Costs.svelte";
  import Models from "./views/Models.svelte";
  import Orchestration from "./views/Orchestration.svelte";
  import Detail from "./views/Detail.svelte";
  import {
    getAgents,
    getPrompts,
    getSkills,
    getStarters,
    getHistory,
    getModels,
  } from "./lib/api";

  let counts = $state<Record<string, number>>({});

  const tabs = [
    { id: "agents", label: "Agents", icon: "agents" },
    { id: "prompts", label: "Prompts", icon: "prompts" },
    { id: "skills", label: "Skills", icon: "skills" },
    { id: "starters", label: "Starters", icon: "starters" },
    { id: "history", label: "History", icon: "history" },
    { id: "costs", label: "Costs", icon: "costs" },
    { id: "models", label: "Models", icon: "models" },
    { id: "orchestration", label: "Orchestration", icon: "orchestration" },
  ];

  const SUBTITLES: Record<string, string> = {
    agents: "agents de la flotte",
    prompts: "fragments de prompt",
    skills: "compétences",
    starters: "packs de démarrage",
    history: "historique des runs",
    costs: "coûts & tokens",
    models: "catalogue de modèles",
    orchestration: "topologie & traces",
  };

  onMount(async () => {
    try {
      const [agents, prompts, skills, starters, history, models] = await Promise.all([
        getAgents(),
        getPrompts(),
        getSkills(),
        getStarters(),
        getHistory(),
        getModels(),
      ]);

      counts.agents = agents.length;
      counts.prompts = prompts.length;
      counts.skills = skills.length;
      counts.starters = starters.length;
      counts.history = history.length;
      // Models is ProviderModels[] → sum of all models across providers
      counts.models = models.reduce((sum, pm) => sum + pm.models.length, 0);
    } catch (err) {
      console.error("Failed to load counts:", err);
      // Continue without counts if fetch fails
    }
  });

  const r = $derived(router.current);
  const active = $derived(r.view);
  const subtitle = $derived(r.param ? "détail" : (SUBTITLES[active] ?? ""));

  // Inject counts into tabs
  const tabsWithCounts = $derived(
    tabs.map((tab) => ({
      ...tab,
      count: counts[tab.id],
    }))
  );
</script>

<Shell tabs={tabsWithCounts} {active}>
  <div class="page-head">
    <h1>
      {#if active === "agents"}
        {r.param ? r.param : "Agents"}
      {:else if active === "prompts"}
        {r.param ? r.param : "Prompts"}
      {:else if active === "skills"}
        {r.param ? r.param : "Skills"}
      {:else if active === "starters"}
        {r.param ? r.param : "Starters"}
      {:else if active === "history"}
        History
      {:else if active === "costs"}
        Costs
      {:else if active === "models"}
        Models
      {:else if active === "orchestration"}
        Orchestration
      {:else}
        {active}
      {/if}
    </h1>
    {#if subtitle}<span class="sub">{subtitle}</span>{/if}
  </div>

  {#if active === "agents"}
    {#if r.param}
      <Detail kind="agents" name={r.param} />
    {:else}
      <Agents />
    {/if}
  {:else if active === "prompts"}
    {#if r.param}
      <Detail kind="prompts" name={r.param} />
    {:else}
      <Prompts />
    {/if}
  {:else if active === "skills"}
    {#if r.param}
      <Detail kind="skills" name={r.param} />
    {:else}
      <Skills />
    {/if}
  {:else if active === "starters"}
    {#if r.param}
      <Detail kind="starters" name={r.param} />
    {:else}
      <Starters />
    {/if}
  {:else if active === "history"}
    <History />
  {:else if active === "costs"}
    <Costs />
  {:else if active === "models"}
    <Models />
  {:else if active === "orchestration"}
    <Orchestration />
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
