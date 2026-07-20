use axum::Json;
use axum::extract::Path;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::IntoResponse;
use serde::Serialize;

use crate::core::agent::Agent;

/// Helper to convert a serializable value to JSON, returning an error response on failure.
fn to_json<T: Serialize>(value: T) -> Json<serde_json::Value> {
    Json(
        serde_json::to_value(value).unwrap_or_else(
            |e| serde_json::json!({"error": format!("Serialization failed: {}", e)}),
        ),
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
    use crate::core::config::is_force_global;
    use crate::core::project;

    // If in a project context (and not forced global), resolve from project config
    if !is_force_global()
        && let Some((root, config)) = project::find_project_config()
        && !config.agents.is_empty()
    {
        let (paths, _) = project::resolve_all_agents(&config, &root);
        let mut agents = Vec::new();
        for path in &paths {
            if let Ok(agent) = crate::parser::parse_agent_file(path) {
                agents.push(agent);
            }
        }
        return agents;
    }

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
    use crate::core::config::{is_force_global, user_prompts_dir};
    use crate::core::prompt::{Prompt, load_all_prompts};

    let prompts: Vec<Prompt> = if !is_force_global()
        && let Some((root, config)) = crate::core::project::find_project_config()
        && !config.prompts.is_empty()
    {
        let (paths, _) = crate::core::project::resolve_all_prompts(&config, &root);
        paths.iter().filter_map(|p| Prompt::load(p).ok()).collect()
    } else {
        load_all_prompts(&user_prompts_dir())
    };
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
    use crate::core::config::{is_force_global, user_skills_dir};
    use crate::core::skill::load_all_skills;

    let skills = if !is_force_global()
        && let Some((root, config)) = crate::core::project::find_project_config()
        && !config.skills.is_empty()
    {
        let (paths, _) = crate::core::project::resolve_all_skills(&config, &root);
        let mut result = Vec::new();
        for path in &paths {
            result.extend(load_all_skills(path));
        }
        result
    } else {
        load_all_skills(&user_skills_dir())
    };
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
pub struct OrchestrationTopology {
    enabled: bool,
    pattern: Option<String>,
    coordinator: Option<String>,
    teams: Vec<TeamResponse>,
    agents: Vec<String>,
}

#[derive(Serialize)]
pub struct TeamResponse {
    lead: Option<String>,
    agents: Vec<String>,
}

#[derive(Serialize)]
#[allow(dead_code)]
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

/// Get orchestration execution traces from storage.
pub async fn get_orchestration_trace() -> Json<serde_json::Value> {
    #[cfg(feature = "storage")]
    {
        use crate::storage::{init_db, queries};
        if let Ok(db) = init_db()
            && let Ok(runs) = queries::get_orchestration_runs(&db, 50)
        {
            let traces: Vec<serde_json::Value> = runs
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "id": r.run_id,
                        "pattern": r.pattern,
                        "config": r.config_json,
                        "outcome": r.outcome_json,
                        "rounds": r.rounds,
                        "halt_reason": r.halt_reason,
                    })
                })
                .collect();
            return Json(serde_json::json!({ "traces": traces }));
        }
    }

    // Also include shell session traces
    let sessions = crate::shell::session::list_sessions();
    let session_traces: Vec<serde_json::Value> = sessions
        .iter()
        .take(50)
        .map(|s| {
            serde_json::json!({
                "id": s.id,
                "name": s.name,
                "provider": s.provider,
                "model": s.model,
                "project_dir": s.project_dir,
                "turns": s.turn_count,
                "tokens_in": s.total_tokens_in,
                "tokens_out": s.total_tokens_out,
                "cost": s.total_cost,
                "created_at": s.created_at,
                "updated_at": s.updated_at,
                "messages_count": s.messages.len(),
            })
        })
        .collect();

    Json(serde_json::json!({
        "traces": [],
        "sessions": session_traces,
    }))
}

/// Get orchestration run detail (board entries, ring contributions, ring votes)
/// for a single run identified by `run_id`.
#[cfg(feature = "storage")]
pub async fn get_orchestration_trace_detail(Path(run_id): Path<String>) -> Json<serde_json::Value> {
    use crate::storage::{init_db, queries};

    let empty = || {
        serde_json::json!({
            "run": null,
            "board_entries": [],
            "ring_contributions": [],
            "ring_votes": [],
        })
    };

    let db = match init_db() {
        Ok(db) => db,
        Err(_) => return Json(empty()),
    };

    let run = queries::get_orchestration_run(&db, &run_id)
        .ok()
        .flatten()
        .map(|r| {
            serde_json::json!({
                "id": r.run_id,
                "pattern": r.pattern,
                "config": r.config_json,
                "outcome": r.outcome_json,
                "rounds": r.rounds,
                "halt_reason": r.halt_reason,
            })
        });

    let board_entries: Vec<serde_json::Value> = queries::get_board_entries(&db, &run_id)
        .unwrap_or_default()
        .into_iter()
        .map(|e| {
            serde_json::json!({
                "agent": e.agent,
                "round": e.round,
                "kind": e.kind,
                "content": e.content,
                "refs": e.refs_json,
                "confidence": e.confidence,
                "tokens_in": e.tokens_in,
                "tokens_out": e.tokens_out,
            })
        })
        .collect();

    let ring_contributions: Vec<serde_json::Value> = queries::get_ring_contributions(&db, &run_id)
        .unwrap_or_default()
        .into_iter()
        .map(|c| {
            serde_json::json!({
                "agent": c.agent,
                "lap": c.lap,
                "position_in_lap": c.position_in_lap,
                "action": c.action,
                "content": c.content,
                "reactions": c.reactions_json,
                "tokens_in": c.tokens_in,
                "tokens_out": c.tokens_out,
            })
        })
        .collect();

    let ring_votes: Vec<serde_json::Value> = queries::get_ring_votes(&db, &run_id)
        .unwrap_or_default()
        .into_iter()
        .map(|v| {
            serde_json::json!({
                "agent": v.agent,
                "position": v.position,
                "confidence": v.confidence,
                "supports": v.supports,
                "concerns": v.concerns,
            })
        })
        .collect();

    Json(serde_json::json!({
        "run": run,
        "board_entries": board_entries,
        "ring_contributions": ring_contributions,
        "ring_votes": ring_votes,
    }))
}

/// Get orchestration run detail — storage disabled, always returns empty shell.
#[cfg(not(feature = "storage"))]
pub async fn get_orchestration_trace_detail(
    Path(_run_id): Path<String>,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "run": null,
        "board_entries": [],
        "ring_contributions": [],
        "ring_votes": [],
    }))
}

pub async fn get_orchestration_topology() -> Json<serde_json::Value> {
    use crate::core::project::find_project_config;

    let disabled = OrchestrationTopology {
        enabled: false,
        pattern: None,
        coordinator: None,
        teams: vec![],
        agents: vec![],
    };

    // Try to find and load project config from current directory
    let project = match find_project_config() {
        Some((_, cfg)) => cfg,
        None => return to_json(disabled),
    };

    let orch = match project.orchestration {
        Some(cfg) => *cfg,
        None => return to_json(disabled),
    };

    // Collect all agents from teams
    let mut all_agents: Vec<String> = vec![];
    if let Some(ref coord) = orch.coordinator {
        all_agents.push(coord.clone());
    }

    let mut teams: Vec<TeamResponse> = vec![];
    for t in &orch.teams {
        if let Some(ref lead) = t.lead {
            all_agents.push(lead.clone());
        }
        all_agents.extend(t.agents.clone());
        teams.push(TeamResponse {
            lead: t.lead.clone(),
            agents: t.agents.clone(),
        });
    }

    all_agents.sort();
    all_agents.dedup();

    to_json(OrchestrationTopology {
        enabled: orch.enabled,
        pattern: Some(orch.pattern.to_string()),
        coordinator: orch.coordinator,
        teams,
        agents: all_agents,
    })
}

#[cfg(all(test, feature = "storage"))]
mod tests {
    use super::*;
    use crate::core::config::ENV_MUTEX;
    use crate::storage::queries::{
        BoardEntryRecord, OrchestrationRunRecord, RingVoteRecord, RunRecord, insert_board_entry,
        insert_orchestration_run, insert_ring_vote, insert_run_with_id,
    };

    /// Guard that points `ARMADAI_CONFIG_DIR` at a fresh temp dir with a
    /// `config.yaml` redirecting storage to a scratch sqlite file, so
    /// `init_db()` (as called by the handler under test) reads/writes there
    /// instead of the real user config. Restores the original env var and
    /// releases the shared env-mutation lock (`ENV_MUTEX`) on drop.
    struct TempStorageGuard {
        _dir: tempfile::TempDir,
        orig: Option<String>,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl TempStorageGuard {
        fn new() -> Self {
            let lock = ENV_MUTEX.lock().unwrap();
            let dir = tempfile::tempdir().unwrap();
            let db_path = dir.path().join("test.sqlite");
            let config_yaml = format!(
                "storage:\n  mode: embedded\n  path: \"{}\"\n",
                db_path.display()
            );
            std::fs::write(dir.path().join("config.yaml"), config_yaml).unwrap();

            let orig = std::env::var("ARMADAI_CONFIG_DIR").ok();
            // SAFETY: modifies the global environment; serialised via ENV_MUTEX.
            unsafe {
                std::env::set_var("ARMADAI_CONFIG_DIR", dir.path());
            }

            Self {
                _dir: dir,
                orig,
                _lock: lock,
            }
        }
    }

    impl Drop for TempStorageGuard {
        fn drop(&mut self) {
            match self.orig.take() {
                // SAFETY: restoring original env state at end of test scope.
                Some(v) => unsafe { std::env::set_var("ARMADAI_CONFIG_DIR", v) },
                None => unsafe { std::env::remove_var("ARMADAI_CONFIG_DIR") },
            }
        }
    }

    #[tokio::test]
    async fn test_get_orchestration_trace_detail_returns_run_and_entries() {
        let _guard = TempStorageGuard::new();
        let db = crate::storage::init_db().unwrap();

        // `orchestration_runs.run_id` references `runs(id)`, so seed the
        // parent row first (mirrors how the orchestration engine writes both
        // tables under the same id).
        insert_run_with_id(
            &db,
            "run-42",
            RunRecord {
                agent: "coordinator".to_string(),
                input: "orchestrate".to_string(),
                output: "done".to_string(),
                provider: "anthropic".to_string(),
                model: "claude-sonnet".to_string(),
                tokens_in: 10,
                tokens_out: 20,
                cost: 0.01,
                duration_ms: 500,
                status: "success".to_string(),
            },
        )
        .unwrap();

        insert_orchestration_run(
            &db,
            OrchestrationRunRecord {
                run_id: "run-42".to_string(),
                pattern: "ring".to_string(),
                config_json: "{}".to_string(),
                outcome_json: Some("{\"status\":\"ok\"}".to_string()),
                rounds: 3,
                halt_reason: None,
                parent_run_id: None,
            },
        )
        .unwrap();

        insert_board_entry(
            &db,
            BoardEntryRecord {
                run_id: "run-42".to_string(),
                agent: "core-specialist".to_string(),
                round: 1,
                kind: "proposal".to_string(),
                content: "Use trait Provider".to_string(),
                refs_json: "[]".to_string(),
                confidence: 0.9,
                tokens_in: 10,
                tokens_out: 20,
            },
        )
        .unwrap();

        insert_ring_vote(
            &db,
            RingVoteRecord {
                run_id: "run-42".to_string(),
                agent: "qa-specialist".to_string(),
                position: "approve".to_string(),
                confidence: 0.8,
                supports: "core-specialist".to_string(),
                concerns: "none".to_string(),
            },
        )
        .unwrap();

        // Drop the connection so the handler's own `init_db()` call can open
        // the same sqlite file freed of any exclusive lock.
        drop(db);

        let response = get_orchestration_trace_detail(Path("run-42".to_string())).await;
        let value = response.0;

        let run = &value["run"];
        assert_eq!(run["id"], "run-42");
        assert_eq!(run["pattern"], "ring");
        assert_eq!(run["rounds"], 3);

        let board_entries = value["board_entries"].as_array().unwrap();
        assert_eq!(board_entries.len(), 1);
        assert_eq!(board_entries[0]["agent"], "core-specialist");
        assert_eq!(board_entries[0]["kind"], "proposal");
        assert_eq!(board_entries[0]["tokens_out"], 20);

        let ring_votes = value["ring_votes"].as_array().unwrap();
        assert_eq!(ring_votes.len(), 1);
        assert_eq!(ring_votes[0]["agent"], "qa-specialist");
        assert_eq!(ring_votes[0]["position"], "approve");

        assert!(value["ring_contributions"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_get_orchestration_trace_detail_unknown_run_is_null() {
        let _guard = TempStorageGuard::new();
        // Ensure the DB/schema exists even though no run is inserted.
        drop(crate::storage::init_db().unwrap());

        let response = get_orchestration_trace_detail(Path("does-not-exist".to_string())).await;
        let value = response.0;
        assert!(value["run"].is_null());
        assert!(value["board_entries"].as_array().unwrap().is_empty());
        assert!(value["ring_contributions"].as_array().unwrap().is_empty());
        assert!(value["ring_votes"].as_array().unwrap().is_empty());
    }
}
