<script lang="ts">
  import { navigate } from "./route.svelte";
  import type { OrchestrationTopology } from "./api";

  interface Props {
    topology: OrchestrationTopology;
  }

  let { topology }: Props = $props();

  // Pan and zoom state
  let tx = $state(0);
  let ty = $state(0);
  let scale = $state(1);

  // Pan tracking
  let isPanning = $state(false);
  let panStartX = $state(0);
  let panStartY = $state(0);
  let panStartTx = $state(0);
  let panStartTy = $state(0);

  // Click vs pan threshold (pixels)
  const CLICK_THRESHOLD = 5;

  // Compute layout
  const diamondSize = 26;
  const agentRadius = 13;
  const padding = 40;
  const verticalGap = 100;

  // Coordinator is at top center
  const coordX = $derived(padding + 100); // Will adjust based on SVG width
  const coordY = padding;

  // Calculate team positions
  interface LayoutTeam {
    x: number;
    y: number;
    agents: Array<{ x: number; y: number; name: string }>;
  }

  const layoutTeams = $derived.by(() => {
    const teams: LayoutTeam[] = [];
    if (!topology.teams || topology.teams.length === 0) {
      return teams;
    }

    const teamCount = topology.teams.length;
    const totalWidth = 200 * teamCount; // Estimate width
    const baseX = padding;
    const teamSpacing = totalWidth / Math.max(teamCount, 1);

    topology.teams.forEach((team, idx) => {
      const x = baseX + teamSpacing / 2 + idx * teamSpacing - totalWidth / 2 + coordX;
      const y = coordY + verticalGap;

      const agentCount = team.agents?.length || 0;
      const agents: Array<{ x: number; y: number; name: string }> = [];

      if (agentCount > 0) {
        const agentSpacing = Math.max(60, 80 / Math.max(agentCount, 1));
        team.agents?.forEach((agent, agentIdx) => {
          const agentX = x - (agentSpacing * (agentCount - 1)) / 2 + agentIdx * agentSpacing;
          const agentY = y + verticalGap / 2;
          agents.push({ x: agentX, y: agentY, name: agent });
        });
      }

      teams.push({ x, y, agents });
    });

    return teams;
  });

  // Calculate standalone agent positions
  const standaloneAgents = $derived.by(() => {
    const agents: Array<{ x: number; y: number; name: string }> = [];
    if (!topology.agents || topology.agents.length === 0) {
      return agents;
    }

    const agentCount = topology.agents.length;
    const totalWidth = 80 * agentCount;
    const baseX = coordX - totalWidth / 2;
    const baseY = coordY + (topology.teams && topology.teams.length > 0 ? 200 : verticalGap);

    topology.agents.forEach((agent, idx) => {
      const x = baseX + idx * (totalWidth / agentCount);
      agents.push({ x, y: baseY, name: agent });
    });

    return agents;
  });

  // Truncate label to initials or short name
  function getLabel(name: string): string {
    if (!name) return "?";
    if (name.length <= 3) return name;
    // Try to get initials from camelCase or snake_case
    const parts = name.split(/[_-]/).filter((p) => p);
    if (parts.length > 1) {
      return parts.map((p) => p[0]).join("").toUpperCase();
    }
    // Fallback: first 3 chars
    return name.substring(0, 3);
  }

  // Compute SVG dimensions
  const svgWidth = $derived(topology.teams && topology.teams.length > 0 ? 600 : 400);
  const svgHeight = $derived(
    topology.teams && topology.teams.length > 0
      ? coordY + 200 + (topology.agents && topology.agents.length > 0 ? 80 : 0)
      : coordY + 150 + (topology.agents && topology.agents.length > 0 ? 80 : 0)
  );

  // Pan handlers
  function onPointerDown(e: PointerEvent) {
    if (e.button !== 0) return; // Only left button
    isPanning = true;
    panStartX = e.clientX;
    panStartY = e.clientY;
    panStartTx = tx;
    panStartTy = ty;
    (e.target as SVGElement).setPointerCapture(e.pointerId);
  }

  function onPointerMove(e: PointerEvent) {
    if (!isPanning) return;
    const deltaX = e.clientX - panStartX;
    const deltaY = e.clientY - panStartY;
    tx = panStartTx + deltaX;
    ty = panStartTy + deltaY;
  }

  function onPointerUp() {
    isPanning = false;
  }

  // Zoom handler (centered at cursor if possible, or globally)
  function onWheel(e: WheelEvent) {
    e.preventDefault();
    const zoomSpeed = 0.1;
    const newScale = Math.max(0.4, Math.min(3, scale - (e.deltaY > 0 ? zoomSpeed : -zoomSpeed)));

    // Optional: center zoom at cursor position
    // For simplicity, we'll just zoom globally
    scale = newScale;
  }

  // Reset pan/zoom
  function resetView() {
    tx = 0;
    ty = 0;
    scale = 1;
  }

  // Node click handler: distinguish click from pan
  let nodeClickStart: { x: number; y: number } | null = null;

  function onNodePointerDown(e: PointerEvent) {
    nodeClickStart = { x: e.clientX, y: e.clientY };
  }

  function onNodeClick(name: string, e: PointerEvent) {
    // Check if this is actually a click (not a pan)
    if (!nodeClickStart) return;
    const distance = Math.sqrt(
      Math.pow(e.clientX - nodeClickStart.x, 2) + Math.pow(e.clientY - nodeClickStart.y, 2)
    );
    if (distance > CLICK_THRESHOLD) {
      // This was a pan, not a click
      nodeClickStart = null;
      return;
    }
    nodeClickStart = null;
    navigate(`agents/${encodeURIComponent(name)}`);
  }

  function onNodeKeyDown(name: string, e: KeyboardEvent) {
    if (e.key === "Enter") {
      navigate(`agents/${encodeURIComponent(name)}`);
    }
  }
</script>

<div class="topology-container">
  {#if !topology.enabled || !topology.coordinator}
    <p class="no-topology">Aucune topologie disponible.</p>
  {:else}
    <div class="topology-wrapper">
      <svg
        width={svgWidth}
        height={svgHeight}
        viewBox="0 0 {svgWidth} {svgHeight}"
        class="topology-svg"
        class:panning={isPanning}
        onpointerdown={onPointerDown}
        onpointermove={onPointerMove}
        onpointerup={onPointerUp}
        onpointercancel={onPointerUp}
        onwheel={onWheel}
        style="touch-action: none"
        role="application"
        aria-label="Orchestration topology graph with pan and zoom"
      >
        <!-- Transformed content group -->
        <g transform="translate({tx} {ty}) scale({scale})">
          <!-- Draw edges: coordinator -> teams -->
          {#each layoutTeams as team}
            <line x1={coordX} y1={coordY} x2={team.x} y2={team.y} class="edge" />

            <!-- Draw edges: team -> agents -->
            {#each team.agents as agent}
              <line x1={team.x} y1={team.y} x2={agent.x} y2={agent.y} class="edge" />
            {/each}
          {/each}

          <!-- Draw edges: coordinator -> standalone agents -->
          {#each standaloneAgents as agent}
            <line x1={coordX} y1={coordY} x2={agent.x} y2={agent.y} class="edge" />
          {/each}

          <!-- Draw coordinator (diamond) -->
          <g
            class="node coordinator clickable"
            role="button"
            tabindex="0"
            onpointerdown={onNodePointerDown}
            onpointerup={(e) => onNodeClick(topology.coordinator || "", e)}
            onkeydown={(e) => onNodeKeyDown(topology.coordinator || "", e)}
            aria-label="Coordinator {topology.coordinator}"
          >
            <rect
              x={coordX - diamondSize / 2}
              y={coordY - diamondSize / 2}
              width={diamondSize}
              height={diamondSize}
              rx="2"
              class="diamond"
              transform="rotate(45 {coordX} {coordY})"
            />
            <text x={coordX} y={coordY} class="label" text-anchor="middle" dominant-baseline="middle">
              <title>{topology.coordinator}</title>
              {getLabel(topology.coordinator)}
            </text>
          </g>

          <!-- Draw team nodes and agents -->
          {#each layoutTeams as team, teamIdx}
            <!-- Team lead (diamond) -->
            {#if team && team.x !== undefined && team.y !== undefined}
              {#if topology.teams[teamIdx]?.lead}
                <g
                  class="node team clickable"
                  role="button"
                  tabindex="0"
                  onpointerdown={onNodePointerDown}
                  onpointerup={(e) => onNodeClick(topology.teams[teamIdx].lead || "", e)}
                  onkeydown={(e) => onNodeKeyDown(topology.teams[teamIdx].lead || "", e)}
                  aria-label="Team lead {topology.teams[teamIdx].lead}"
                >
                  <rect
                    x={team.x - diamondSize / 2}
                    y={team.y - diamondSize / 2}
                    width={diamondSize}
                    height={diamondSize}
                    rx="2"
                    class="diamond"
                    transform="rotate(45 {team.x} {team.y})"
                  />
                  <text
                    x={team.x}
                    y={team.y}
                    class="label"
                    text-anchor="middle"
                    dominant-baseline="middle"
                  >
                    <title>{topology.teams[teamIdx].lead}</title>
                    {getLabel(topology.teams[teamIdx].lead || "")}
                  </text>
                </g>
              {:else}
                <g class="node team">
                  <rect
                    x={team.x - diamondSize / 2}
                    y={team.y - diamondSize / 2}
                    width={diamondSize}
                    height={diamondSize}
                    rx="2"
                    class="diamond"
                    transform="rotate(45 {team.x} {team.y})"
                  />
                  <text
                    x={team.x}
                    y={team.y}
                    class="label"
                    text-anchor="middle"
                    dominant-baseline="middle"
                  >
                    <title>Équipe</title>
                    T
                  </text>
                </g>
              {/if}

              <!-- Agents under this team -->
              {#each team.agents as agent}
                <g
                  class="node agent clickable"
                  role="button"
                  tabindex="0"
                  onpointerdown={onNodePointerDown}
                  onpointerup={(e) => onNodeClick(agent.name, e)}
                  onkeydown={(e) => onNodeKeyDown(agent.name, e)}
                  aria-label="Agent {agent.name}"
                >
                  <circle cx={agent.x} cy={agent.y} r={agentRadius} class="agent-circle" />
                  <text
                    x={agent.x}
                    y={agent.y}
                    class="label agent-label"
                    text-anchor="middle"
                    dominant-baseline="middle"
                  >
                    <title>{agent.name}</title>
                    {getLabel(agent.name)}
                  </text>
                </g>
              {/each}
            {/if}
          {/each}

          <!-- Standalone agents (not in teams) -->
          {#each standaloneAgents as agent}
            <g
              class="node agent clickable"
              role="button"
              tabindex="0"
              onpointerdown={onNodePointerDown}
              onpointerup={(e) => onNodeClick(agent.name, e)}
              onkeydown={(e) => onNodeKeyDown(agent.name, e)}
              aria-label="Agent {agent.name}"
            >
              <circle cx={agent.x} cy={agent.y} r={agentRadius} class="agent-circle" />
              <text
                x={agent.x}
                y={agent.y}
                class="label agent-label"
                text-anchor="middle"
                dominant-baseline="middle"
              >
                <title>{agent.name}</title>
                {getLabel(agent.name)}
              </text>
            </g>
          {/each}
        </g>
      </svg>

      <!-- Reset button (discreet, positioned absolutely) -->
      <button class="reset-btn" onclick={resetView} title="Recentrer le graphe">
        ↺
      </button>
    </div>
  {/if}
</div>

<style>
  .topology-container {
    background: var(--surface-1);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: var(--panel-pad);
    margin: var(--gutter) 0;
  }

  .no-topology {
    color: var(--text-secondary);
    margin: 0;
    text-align: center;
    padding: 24px;
  }

  .topology-wrapper {
    position: relative;
    overflow: hidden;
    border-radius: 6px;
    background: var(--surface-0);
  }

  .topology-svg {
    width: 100%;
    max-width: 100%;
    height: auto;
    display: block;
    cursor: grab;
    transition: cursor 150ms ease;
  }

  .topology-svg.panning {
    cursor: grabbing;
  }

  .edge {
    stroke: var(--border-strong);
    stroke-width: 1.4;
    fill: none;
  }

  .node {
    pointer-events: auto;
  }

  .node.clickable {
    cursor: pointer;
  }

  .node.clickable:hover .diamond,
  .node.clickable:hover .agent-circle {
    stroke-width: 2;
    filter: brightness(1.1);
  }

  .node.clickable:focus-visible {
    outline: 2px solid var(--focus-ring);
    outline-offset: 2px;
  }

  .diamond {
    fill: var(--brass-bg);
    stroke: var(--brass-border);
    stroke-width: 1.5;
    transition: stroke-width 150ms ease, filter 150ms ease;
  }

  .node.coordinator .diamond {
    fill: var(--brass-bg);
    stroke: var(--brass-border);
  }

  .node.team .diamond {
    fill: var(--brass-bg);
    stroke: var(--brass-border);
  }

  .agent-circle {
    fill: var(--surface-3);
    stroke: var(--border-strong);
    stroke-width: 1.4;
    transition: stroke-width 150ms ease, filter 150ms ease;
  }

  .label {
    font-family: var(--font-mono);
    font-size: 9px;
    font-weight: 600;
    fill: var(--text-primary);
    pointer-events: none;
  }

  .node.coordinator .label {
    fill: var(--brass-strong);
  }

  .node.team .label {
    fill: var(--brass-strong);
  }

  .agent-label {
    fill: var(--text-secondary);
  }

  .reset-btn {
    position: absolute;
    top: 12px;
    right: 12px;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 6px;
    width: 32px;
    height: 32px;
    padding: 0;
    cursor: pointer;
    font-size: 14px;
    color: var(--text-secondary);
    display: flex;
    align-items: center;
    justify-content: center;
    transition: all 150ms ease;
    z-index: 10;
  }

  .reset-btn:hover {
    background: var(--surface-3);
    color: var(--text-primary);
    border-color: var(--border-strong);
  }

  .reset-btn:focus {
    outline: 2px solid var(--brass);
    outline-offset: 2px;
  }

  .reset-btn:active {
    transform: scale(0.95);
  }
</style>
