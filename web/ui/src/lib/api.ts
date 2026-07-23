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

export const fmtCost = (n: number) => `$${n.toFixed(2)}`;
export const fmtTokens = (n: number) => {
  const str = n.toLocaleString("fr-FR");
  return str.replace(/ /g, " ");
};
