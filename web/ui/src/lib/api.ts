export interface AgentSummary {
  name: string;
  provider: string;
  model: string;
  tags: string[];
  stacks: string[];
  scope: string[];
  model_fallback: string[];
}

export interface HistoryEntry {
  agent: string;
  provider: string;
  model: string;
  tokens_in: number;
  tokens_out: number;
  cost: number;
  duration_ms: number;
  status: string;
}

export interface PromptSummary {
  name: string;
  description: string | null;
  apply_to: string[];
  source: string;
}

export interface SkillSummary {
  name: string;
  description: string | null;
  version: string | null;
  tools: string[];
  source: string;
}

export interface StarterSummary {
  name: string;
  description: string;
  agents_count: number;
  prompts_count: number;
  skills_count: number;
}

export interface CostSummary {
  agent: string;
  total_runs: number;
  total_cost: number;
  total_tokens_in: number;
  total_tokens_out: number;
}

export interface ModelSummary {
  id: string;
  name: string | null;
  context: number | null;
  max_output: number | null;
  cost_input: number | null;
  cost_output: number | null;
}

export interface ProviderModels {
  provider: string;
  models: ModelSummary[];
}

async function getJson<T>(path: string): Promise<T> {
  const r = await fetch(path);
  if (!r.ok) throw new Error(`${path}: ${r.status}`);
  return (await r.json()) as T;
}

export const getAgents = () => getJson<AgentSummary[]>("/api/agents");
export const getHistory = () => getJson<HistoryEntry[]>("/api/history");
export const getPrompts = () => getJson<PromptSummary[]>("/api/prompts");
export const getSkills = () => getJson<SkillSummary[]>("/api/skills");
export const getStarters = () => getJson<StarterSummary[]>("/api/starters");
export const getCosts = () => getJson<CostSummary[]>("/api/costs");
export const getModels = () => getJson<ProviderModels[]>("/api/models");
export const getDetail = (kind: string, name: string) =>
  getJson<Record<string, unknown>>(`/api/${kind}/${encodeURIComponent(name)}`);

export const fmtCost = (n: number) => `$${n.toFixed(2)}`;
export const fmtTokens = (n: number) => {
  const str = n.toLocaleString("fr-FR");
  return str.replace(/ /g, " ");
};
export const fmtContext = (n: number | null): string => {
  if (n === null) return "—";
  return Math.round(n / 1000) + "k";
};
