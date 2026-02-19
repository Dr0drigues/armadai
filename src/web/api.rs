use axum::Json;
use axum::extract::Path;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::IntoResponse;
use serde::Serialize;

use crate::core::agent::Agent;

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
    system_prompt: String,
    instructions: Option<String>,
    output_format: Option<String>,
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
                system_prompt: a.system_prompt,
                instructions: a.instructions,
                output_format: a.output_format,
            };
            Json(serde_json::to_value(detail).unwrap())
        }
        None => Json(
            serde_json::to_value(ErrorResponse {
                error: format!("Agent '{name}' not found"),
            })
            .unwrap(),
        ),
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
            Json(serde_json::to_value(detail).unwrap())
        }
        None => Json(
            serde_json::to_value(ErrorResponse {
                error: format!("Prompt '{name}' not found"),
            })
            .unwrap(),
        ),
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
            Json(serde_json::to_value(detail).unwrap())
        }
        None => Json(
            serde_json::to_value(ErrorResponse {
                error: format!("Skill '{name}' not found"),
            })
            .unwrap(),
        ),
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
            return Json(
                serde_json::to_value(ErrorResponse {
                    error: format!("Starter '{name}' not found"),
                })
                .unwrap(),
            );
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
            Json(serde_json::to_value(detail).unwrap())
        }
        Err(_) => Json(
            serde_json::to_value(ErrorResponse {
                error: format!("Failed to load starter '{name}'"),
            })
            .unwrap(),
        ),
    }
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
