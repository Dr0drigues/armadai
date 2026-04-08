use axum::Json;
use axum::extract::Path;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::IntoResponse;
use serde::Serialize;

use crate::core::agent::Agent;

/// Helper to convert a serializable value to JSON, returning an error response on failure.
fn to_json<T: Serialize>(value: T) -> Json<serde_json::Value> {
    Json(
        serde_json::to_value(value).unwrap_or_else(|e| {
            serde_json::json!({"error": format!("Serialization failed: {}", e)})
        }),
    )
}

#[derive(Serialize)]
pub struct AgentSummary {
    name: String,
    provider: String,
    model: String,
    tags: Vec<String>,
    stacks: Vec<String>,
    scope: Vec<String>,
    model_fallback: Vec<String>,
}

#[derive(Serialize)]
pub struct AgentDetail {
    name: String,
    source: String,
    provider: String,
    model: String,
    tags: Vec<String>,
    stacks: Vec<String>,
    scope: Vec<String>,
    model_fallback: Vec<String>,
    temperature: f32,
    max_tokens: Option<u32>,
    timeout: Option<u64>,
    rate_limit: Option<String>,
    orchestration: Option<String>,
    triggers: Option<AgentTriggersInfo>,
    ring_config: Option<AgentRingInfo>,
    system_prompt: String,
    instructions: Option<String>,
    output_format: Option<String>,
    model_resolution: Vec<ModelResolutionEntry>,
}

#[derive(Serialize)]
pub struct AgentTriggersInfo {
    requires: Vec<String>,
    excludes: Vec<String>,
    min_round: u32,
    max_round: Option<u32>,
    priority: u8,
}

#[derive(Serialize)]
pub struct AgentRingInfo {
    role: String,
    position: Option<usize>,
    vote_weight: f32,
}

#[derive(Serialize)]
pub struct HistoryEntry {
    agent: String,
    provider: String,
    model: String,
    tokens_in: i64,
    tokens_out: i64,
    cost: f64,
    duration_ms: i64,
    status: String,
}

#[derive(Serialize)]
pub struct CostSummary {
    agent: String,
    total_runs: i64,
    total_cost: f64,
    total_tokens_in: i64,
    total_tokens_out: i64,
}

#[derive(Serialize)]
pub struct PromptSummary {
    name: String,
    description: Option<String>,
    apply_to: Vec<String>,
    source: String,
}

#[derive(Serialize)]
pub struct SkillSummary {
    name: String,
    description: Option<String>,
    version: Option<String>,
    tools: Vec<String>,
    source: String,
}

#[derive(Serialize)]
pub struct PromptDetail {
    name: String,
    description: Option<String>,
    apply_to: Vec<String>,
    body: String,
    source: String,
}

#[derive(Serialize)]
pub struct SkillFile {
    name: String,
    content: Option<String>,
}

#[derive(Serialize)]
pub struct SkillDetail {
    name: String,
    description: Option<String>,
    version: Option<String>,
    tools: Vec<String>,
    body: String,
    source: String,
    scripts: Vec<SkillFile>,
    references: Vec<SkillFile>,
    assets: Vec<SkillFile>,
}

#[derive(Serialize)]
pub struct StarterSummary {
    name: String,
    description: String,
    agents_count: usize,
    prompts_count: usize,
    skills_count: usize,
}

#[derive(Serialize)]
pub struct StarterDetail {
    name: String,
    description: String,
    agents: Vec<String>,
    prompts: Vec<String>,
    skills: Vec<String>,
}

#[derive(Serialize)]
pub struct ProviderModels {
    provider: String,
    models: Vec<ModelSummary>,
}

#[derive(Serialize)]
pub struct ModelSummary {
    id: String,
    name: Option<String>,
    context: Option<u64>,
    max_output: Option<u64>,
    cost_input: Option<f64>,
    cost_output: Option<f64>,
}

#[derive(Serialize)]
pub struct ModelResolutionEntry {
    target: String,
    resolved_model: String,
}

#[derive(Serialize)]
pub struct ErrorResponse {
    error: String,
}

fn load_agents() -> Vec<Agent> {
    let agents_dir = crate::core::config::AppPaths::resolve().agents_dir;
    Agent::load_all(&agents_dir).unwrap_or_default()
}

pub async fn list_agents() -> Json<Vec<AgentSummary>> {
    let agents = load_agents();
    let summaries = agents
        .into_iter()
        .map(|a| {
            let model = a.model_display();
            AgentSummary {
                name: a.name,
                provider: a.metadata.provider,
                model,
                tags: a.metadata.tags,
                stacks: a.metadata.stacks,
                scope: a.metadata.scope,
                model_fallback: a.metadata.model_fallback,
            }
        })
        .collect();
    Json(summaries)
}

pub async fn get_agent(Path(name): Path<String>) -> Json<serde_json::Value> {
    let agents = load_agents();
    match agents
        .into_iter()
        .find(|a| a.name.eq_ignore_ascii_case(&name))
    {
        Some(a) => {
            let model = a.model_display();
            let resolution = crate::linker::model_resolution::preview_model_resolution(
                a.metadata.model.as_deref(),
            );
            let model_resolution = resolution
                .into_iter()
                .map(|(target, resolved)| ModelResolutionEntry {
                    target: target.to_string(),
                    resolved_model: resolved,
                })
                .collect();
            let orchestration = a.metadata.orchestration.map(|p| p.to_string());
            let triggers = a.metadata.triggers.map(|t| AgentTriggersInfo {
                requires: t.requires,
                excludes: t.excludes,
                min_round: t.min_round,
                max_round: t.max_round,
                priority: t.priority,
            });
            let ring_config = a.metadata.ring_config.map(|r| AgentRingInfo {
                role: r.role,
                position: r.position,
                vote_weight: r.vote_weight,
            });
            let detail = AgentDetail {
                name: a.name,
                source: a.source.display().to_string(),
                provider: a.metadata.provider,
                model,
                tags: a.metadata.tags,
                stacks: a.metadata.stacks,
                scope: a.metadata.scope,
                model_fallback: a.metadata.model_fallback,
                temperature: a.metadata.temperature,
                max_tokens: a.metadata.max_tokens,
                timeout: a.metadata.timeout,
                rate_limit: a.metadata.rate_limit,
                orchestration,
                triggers,
                ring_config,
                system_prompt: a.system_prompt,
                instructions: a.instructions,
                output_format: a.output_format,
                model_resolution,
            };
            to_json(detail)
        }
        None => to_json(ErrorResponse {
            error: format!("Agent '{name}' not found"),
        }),
    }
}

#[cfg(feature = "storage")]
pub async fn get_history() -> Json<Vec<HistoryEntry>> {
    use crate::storage::{init_db, queries};

    let db = match init_db() {
        Ok(db) => db,
        Err(_) => return Json(vec![]),
    };

    match queries::get_history(&db, None, 100) {
        Ok(records) => Json(
            records
                .into_iter()
                .map(|r| HistoryEntry {
                    agent: r.agent,
                    provider: r.provider,
                    model: r.model,
                    tokens_in: r.tokens_in,
                    tokens_out: r.tokens_out,
                    cost: r.cost,
                    duration_ms: r.duration_ms,
                    status: r.status,
                })
                .collect(),
        ),
        Err(_) => Json(vec![]),
    }
}

#[cfg(not(feature = "storage"))]
pub async fn get_history() -> Json<Vec<HistoryEntry>> {
    Json(vec![])
}

#[cfg(feature = "storage")]
pub async fn get_costs() -> Json<Vec<CostSummary>> {
    use crate::storage::{init_db, queries};

    let db = match init_db() {
        Ok(db) => db,
        Err(_) => return Json(vec![]),
    };

    match queries::get_costs_summary(&db, None) {
        Ok(summaries) => Json(
            summaries
                .into_iter()
                .map(|s| CostSummary {
                    agent: s.agent,
                    total_runs: s.total_runs,
                    total_cost: s.total_cost,
                    total_tokens_in: s.total_tokens_in,
                    total_tokens_out: s.total_tokens_out,
                })
                .collect(),
        ),
        Err(_) => Json(vec![]),
    }
}

#[cfg(not(feature = "storage"))]
pub async fn get_costs() -> Json<Vec<CostSummary>> {
    Json(vec![])
}

pub async fn list_prompts() -> Json<Vec<PromptSummary>> {
    use crate::core::config::user_prompts_dir;
    use crate::core::prompt::load_all_prompts;

    let prompts = load_all_prompts(&user_prompts_dir());
    let summaries = prompts
        .into_iter()
        .map(|p| PromptSummary {
            name: p.name,
            description: p.description,
            apply_to: p.apply_to,
            source: p.source.display().to_string(),
        })
        .collect();
    Json(summaries)
}

pub async fn list_skills() -> Json<Vec<SkillSummary>> {
    use crate::core::config::user_skills_dir;
    use crate::core::skill::load_all_skills;

    let skills = load_all_skills(&user_skills_dir());
    let summaries = skills
        .into_iter()
        .map(|s| SkillSummary {
            name: s.name,
            description: s.description,
            version: s.version,
            tools: s.tools,
            source: s.source.display().to_string(),
        })
        .collect();
    Json(summaries)
}

pub async fn get_prompt(Path(name): Path<String>) -> Json<serde_json::Value> {
    use crate::core::config::user_prompts_dir;
    use crate::core::prompt::load_all_prompts;

    let prompts = load_all_prompts(&user_prompts_dir());
    match prompts
        .into_iter()
        .find(|p| p.name.eq_ignore_ascii_case(&name))
    {
        Some(p) => {
            let detail = PromptDetail {
                name: p.name,
                description: p.description,
                apply_to: p.apply_to,
                body: p.body,
                source: p.source.display().to_string(),
            };
            to_json(detail)
        }
        None => to_json(ErrorResponse {
            error: format!("Prompt '{name}' not found"),
        }),
    }
}

pub async fn get_skill(Path(name): Path<String>) -> Json<serde_json::Value> {
    use crate::core::config::user_skills_dir;
    use crate::core::skill::{load_all_skills, read_text_file};

    let to_skill_file = |p: &std::path::Path| -> SkillFile {
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let content = read_text_file(p);
        SkillFile { name, content }
    };

    let skills = load_all_skills(&user_skills_dir());
    match skills
        .into_iter()
        .find(|s| s.name.eq_ignore_ascii_case(&name))
    {
        Some(s) => {
            let detail = SkillDetail {
                name: s.name,
                description: s.description,
                version: s.version,
                tools: s.tools,
                body: s.body,
                source: s.source.display().to_string(),
                scripts: s.scripts.iter().map(|p| to_skill_file(p)).collect(),
                references: s.references.iter().map(|p| to_skill_file(p)).collect(),
                assets: s.assets.iter().map(|p| to_skill_file(p)).collect(),
            };
            to_json(detail)
        }
        None => to_json(ErrorResponse {
            error: format!("Skill '{name}' not found"),
        }),
    }
}

pub async fn list_starters() -> Json<Vec<StarterSummary>> {
    use crate::core::starter::load_all_packs;

    let packs = load_all_packs();
    let summaries = packs
        .into_iter()
        .map(|p| StarterSummary {
            name: p.name,
            description: p.description,
            agents_count: p.agents.len(),
            prompts_count: p.prompts.len(),
            skills_count: p.skills.len(),
        })
        .collect();
    Json(summaries)
}

pub async fn get_starter(Path(name): Path<String>) -> Json<serde_json::Value> {
    use crate::core::starter::{StarterPack, find_pack_dir};

    let pack_dir = match find_pack_dir(&name) {
        Some(dir) => dir,
        None => {
            return to_json(ErrorResponse {
                error: format!("Starter '{name}' not found"),
            });
        }
    };

    match StarterPack::load(&pack_dir) {
        Ok(p) => {
            let detail = StarterDetail {
                name: p.name,
                description: p.description,
                agents: p.agents,
                prompts: p.prompts,
                skills: p.skills,
            };
            to_json(detail)
        }
        Err(_) => to_json(ErrorResponse {
            error: format!("Failed to load starter '{name}'"),
        }),
    }
}

pub async fn list_models() -> Json<Vec<ProviderModels>> {
    use crate::model_registry::fetch::load_all_providers_cached;

    let providers = load_all_providers_cached().unwrap_or_default();
    let mut keys: Vec<String> = providers.keys().cloned().collect();
    keys.sort();

    let result: Vec<ProviderModels> = keys
        .into_iter()
        .filter_map(|provider| {
            let entries = providers.get(&provider)?;
            let models = entries
                .iter()
                .map(|e| ModelSummary {
                    id: e.id.clone(),
                    name: e.name.clone(),
                    context: e.limit.as_ref().and_then(|l| l.context),
                    max_output: e.limit.as_ref().and_then(|l| l.output),
                    cost_input: e.cost.as_ref().and_then(|c| c.input),
                    cost_output: e.cost.as_ref().and_then(|c| c.output),
                })
                .collect();
            Some(ProviderModels { provider, models })
        })
        .collect();

    Json(result)
}

#[derive(Serialize)]
pub struct RefreshResult {
    status: String,
    providers: usize,
}

#[cfg(feature = "providers-api")]
pub async fn refresh_models() -> Json<serde_json::Value> {
    match crate::model_registry::fetch::refresh_registry().await {
        Ok(count) => to_json(RefreshResult {
            status: "ok".to_string(),
            providers: count,
        }),
        Err(e) => to_json(ErrorResponse {
            error: format!("Refresh failed: {e}"),
        }),
    }
}

#[cfg(not(feature = "providers-api"))]
pub async fn refresh_models() -> Json<serde_json::Value> {
    to_json(ErrorResponse {
        error: "Model sync requires providers-api feature".to_string(),
    })
}

pub async fn get_starter_config(Path(name): Path<String>) -> impl IntoResponse {
    use crate::core::starter::{StarterPack, find_pack_dir};

    let pack_dir = match find_pack_dir(&name) {
        Some(dir) => dir,
        None => {
            return (
                StatusCode::NOT_FOUND,
                HeaderMap::new(),
                format!("Starter '{name}' not found"),
            );
        }
    };

    let pack = match StarterPack::load(&pack_dir) {
        Ok(p) => p,
        Err(_) => {
            return (
                StatusCode::NOT_FOUND,
                HeaderMap::new(),
                format!("Failed to load starter '{name}'"),
            );
        }
    };

    let yaml = crate::cli::init::generate_project_yaml(&pack, &name);
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/x-yaml"),
    );
    headers.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_static("attachment; filename=\"config.yaml\""),
    );
    (StatusCode::OK, headers, yaml)
}
