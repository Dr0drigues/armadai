use axum::Json;
use axum::extract::Path;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::IntoResponse;
use serde::Serialize;

use armadai_core::agent::Agent;

/// Helper to convert a serializable value to JSON, returning an error response on failure.
fn to_json<T: Serialize>(value: T) -> Json<serde_json::Value> {
    Json(
        serde_json::to_value(value).unwrap_or_else(
            |e| serde_json::json!({"error": format!("Serialization failed: {}", e)}),
        ),
    )
}

/// Whether `source`'s file stem (case-insensitive) equals `name`. Detail
/// lookups accept both the H1 display name and the file slug, since starters
/// and the orchestration topology reference agents/prompts/skills by their
/// file stem (e.g. "dev-lead") rather than their H1 title ("Dev Lead").
fn file_stem_matches(source: &std::path::Path, name: &str) -> bool {
    source
        .file_stem()
        .and_then(|s| s.to_str())
        .is_some_and(|stem| stem.eq_ignore_ascii_case(name))
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
    use armadai_core::agent_source;
    use armadai_core::config::is_force_global;
    use armadai_core::project;

    // If in a project context (and not forced global), resolve from project
    // config — file-backed and declared agents alike.
    // `project_declares_agents` (rather than a bare `!config.agents.is_empty()`
    // check) so a project that relies purely on `.armadai/agents.yaml` — no
    // `agents:` list at all — still takes this branch instead of falling
    // through to the global library below, and `load_all_agents` (rather
    // than `project::resolve_all_agents`, which only ever resolves file
    // paths and silently drops every declared agent) so a declared agent is
    // actually returned instead of leaving a same-named global agent to be
    // served in its place.
    if !is_force_global()
        && let Some((root, config)) = project::find_project_config()
        && agent_source::project_declares_agents(&root, &config)
    {
        let fragments = agent_source::project_fragments(&root);
        let (agents, warnings) = agent_source::load_all_agents(&config, &root, &fragments);
        // Read-only endpoint: never refuse over a load warning. The JSON
        // shape stays a bare agent list (unchanged API contract), so a
        // dropped/shadowed declared agent is surfaced via the server log
        // instead of vanishing unnoticed.
        for w in &warnings {
            tracing::warn!("agent load warning: {}", w.message());
        }
        return agents;
    }

    let agents_dir = armadai_core::config::AppPaths::resolve().agents_dir;
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
        .find(|a| a.name.eq_ignore_ascii_case(&name) || file_stem_matches(&a.source, &name))
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
    use crate::db::init_db;
    use armadai_storage::queries;

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
    use crate::db::init_db;
    use armadai_storage::queries;

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
    use armadai_core::config::{is_force_global, user_prompts_dir};
    use armadai_core::prompt::{Prompt, load_all_prompts};

    let prompts: Vec<Prompt> = if !is_force_global()
        && let Some((root, config)) = armadai_core::project::find_project_config()
        && !config.prompts.is_empty()
    {
        let (paths, _) = armadai_core::project::resolve_all_prompts(&config, &root);
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
    use armadai_core::config::{is_force_global, user_skills_dir};
    use armadai_core::skill::load_all_skills;

    let skills = if !is_force_global()
        && let Some((root, config)) = armadai_core::project::find_project_config()
        && !config.skills.is_empty()
    {
        let (paths, _) = armadai_core::project::resolve_all_skills(&config, &root);
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
    use armadai_core::config::user_prompts_dir;
    use armadai_core::prompt::load_all_prompts;

    let prompts = load_all_prompts(&user_prompts_dir());
    match prompts
        .into_iter()
        .find(|p| p.name.eq_ignore_ascii_case(&name) || file_stem_matches(&p.source, &name))
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
    use armadai_core::config::user_skills_dir;
    use armadai_core::skill::{load_all_skills, read_text_file};

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
        .find(|s| s.name.eq_ignore_ascii_case(&name) || file_stem_matches(&s.source, &name))
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
    use armadai_core::starter::load_all_packs;

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
    use armadai_core::starter::{StarterPack, find_pack_dir};

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
    use armadai_providers::model_registry::fetch::load_all_providers_cached;

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
    match armadai_providers::model_registry::fetch::refresh_registry().await {
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
    use armadai_core::starter::{StarterPack, find_pack_dir};

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
        use crate::db::init_db;
        use armadai_storage::queries;
        if let Ok(db) = init_db()
            && let Ok(runs) = queries::get_root_orchestration_runs(&db, 50)
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

/// Fetch the board entries, ring contributions, and ring votes for a single
/// run, serialized to JSON. Shared between the main run and each of its
/// nested children so their entries render identically.
#[cfg(feature = "storage")]
fn fetch_run_entries(
    db: &armadai_storage::Database,
    run_id: &str,
) -> (
    Vec<serde_json::Value>,
    Vec<serde_json::Value>,
    Vec<serde_json::Value>,
) {
    use armadai_storage::queries;

    let board_entries = queries::get_board_entries(db, run_id)
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
    let ring_contributions = queries::get_ring_contributions(db, run_id)
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
    let ring_votes = queries::get_ring_votes(db, run_id)
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
    (board_entries, ring_contributions, ring_votes)
}

/// Get orchestration run detail (board entries, ring contributions, ring votes,
/// delegation events, and nested children) for a single run identified by
/// `run_id`.
#[cfg(feature = "storage")]
pub async fn get_orchestration_trace_detail(Path(run_id): Path<String>) -> Json<serde_json::Value> {
    use crate::db::init_db;
    use armadai_storage::queries;

    let empty = || {
        serde_json::json!({
            "run": null,
            "board_entries": [],
            "ring_contributions": [],
            "ring_votes": [],
            "delegation_events": [],
            "children": [],
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
                "parent_run_id": r.parent_run_id,
            })
        });

    let (board_entries, ring_contributions, ring_votes) = fetch_run_entries(&db, &run_id);

    let delegation_events: Vec<serde_json::Value> = queries::get_delegation_events(&db, &run_id)
        .unwrap_or_default()
        .into_iter()
        .map(|e| {
            serde_json::json!({
                "seq": e.seq,
                "from": e.from_agent,
                "to": e.to_agent,
                "message": e.message,
                "depth": e.depth,
            })
        })
        .collect();

    let children: Vec<serde_json::Value> = queries::get_child_orchestration_runs(&db, &run_id)
        .unwrap_or_default()
        .into_iter()
        .map(|c| {
            let (cb, cc, cv) = fetch_run_entries(&db, &c.run_id);
            serde_json::json!({
                "run": {
                    "id": c.run_id,
                    "pattern": c.pattern,
                    "config": c.config_json,
                    "outcome": c.outcome_json,
                    "rounds": c.rounds,
                    "halt_reason": c.halt_reason,
                    "parent_run_id": c.parent_run_id,
                },
                "board_entries": cb,
                "ring_contributions": cc,
                "ring_votes": cv,
            })
        })
        .collect();

    Json(serde_json::json!({
        "run": run,
        "board_entries": board_entries,
        "ring_contributions": ring_contributions,
        "ring_votes": ring_votes,
        "delegation_events": delegation_events,
        "children": children,
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
        "delegation_events": [],
        "children": [],
    }))
}

pub async fn get_orchestration_topology() -> Json<serde_json::Value> {
    use armadai_core::project::find_project_config;

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
    use crate::test_support::TempStorageGuard;
    use armadai_storage::queries::{
        BoardEntryRecord, DelegationEventRecord, OrchestrationRunRecord, RingVoteRecord, RunRecord,
        insert_board_entry, insert_delegation_event, insert_orchestration_run, insert_ring_vote,
        insert_run_with_id,
    };

    #[tokio::test]
    async fn test_get_orchestration_trace_detail_returns_run_and_entries() {
        let _guard = TempStorageGuard::new();
        let db = crate::db::init_db().unwrap();

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
                project: None,
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
        drop(crate::db::init_db().unwrap());

        let response = get_orchestration_trace_detail(Path("does-not-exist".to_string())).await;
        let value = response.0;
        assert!(value["run"].is_null());
        assert!(value["board_entries"].as_array().unwrap().is_empty());
        assert!(value["ring_contributions"].as_array().unwrap().is_empty());
        assert!(value["ring_votes"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_trace_detail_hierarchical_has_delegation_events_and_children() {
        let _guard = TempStorageGuard::new();
        let db = crate::db::init_db().unwrap();

        // Parent hierarchical run.
        insert_run_with_id(
            &db,
            "h-1",
            RunRecord {
                agent: "coordinator".to_string(),
                input: "go".to_string(),
                output: "done".to_string(),
                provider: "orchestration".to_string(),
                model: String::new(),
                tokens_in: 10,
                tokens_out: 20,
                cost: 0.0,
                duration_ms: 0,
                status: "success".to_string(),
                project: None,
            },
        )
        .unwrap();
        insert_orchestration_run(
            &db,
            OrchestrationRunRecord {
                run_id: "h-1".to_string(),
                pattern: "hierarchical".to_string(),
                config_json: "{}".to_string(),
                outcome_json: None,
                rounds: 2,
                halt_reason: None,
                parent_run_id: None,
            },
        )
        .unwrap();
        insert_delegation_event(
            &db,
            DelegationEventRecord {
                run_id: "h-1".to_string(),
                seq: 0,
                from_agent: "coordinator".to_string(),
                to_agent: "research-lead".to_string(),
                message: "analyze".to_string(),
                depth: 1,
            },
        )
        .unwrap();

        // Nested child blackboard run linked to the parent.
        insert_run_with_id(
            &db,
            "c-1",
            RunRecord {
                agent: "orchestration:blackboard".to_string(),
                input: "analyze".to_string(),
                output: "x".to_string(),
                provider: "orchestration".to_string(),
                model: String::new(),
                tokens_in: 5,
                tokens_out: 5,
                cost: 0.0,
                duration_ms: 0,
                status: "success".to_string(),
                project: None,
            },
        )
        .unwrap();
        insert_orchestration_run(
            &db,
            OrchestrationRunRecord {
                run_id: "c-1".to_string(),
                pattern: "blackboard".to_string(),
                config_json: "{}".to_string(),
                outcome_json: None,
                rounds: 1,
                halt_reason: None,
                parent_run_id: Some("h-1".to_string()),
            },
        )
        .unwrap();
        insert_board_entry(
            &db,
            BoardEntryRecord {
                run_id: "c-1".to_string(),
                agent: "searcher".to_string(),
                round: 1,
                kind: "finding".to_string(),
                content: "a finding".to_string(),
                refs_json: "[]".to_string(),
                confidence: 0.9,
                tokens_in: 5,
                tokens_out: 5,
            },
        )
        .unwrap();

        drop(db);

        // Detail of the hierarchical run.
        let response = get_orchestration_trace_detail(Path("h-1".to_string())).await;
        let v = response.0;
        assert_eq!(v["run"]["pattern"], "hierarchical");
        let events = v["delegation_events"].as_array().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["to"], "research-lead");
        let children = v["children"].as_array().unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0]["run"]["pattern"], "blackboard");
        assert_eq!(children[0]["board_entries"].as_array().unwrap().len(), 1);

        // The list shows only the root (the nested child is hidden).
        let list = get_orchestration_trace().await.0;
        let traces = list["traces"].as_array().unwrap();
        assert!(traces.iter().any(|t| t["id"] == "h-1"));
        assert!(
            !traces.iter().any(|t| t["id"] == "c-1"),
            "nested child must not appear in the list"
        );
    }
}

#[cfg(test)]
mod load_agents_declarative_tests {
    use super::*;
    use armadai_core::test_support::IsolatedProjectDir;

    /// A declarations-only project: no `agents:` list at all (the layout
    /// this format exists to enable), two declared agents -- one plain, one
    /// named to collide with a global homonym a test can plant separately.
    fn declared_only_project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        std::fs::create_dir_all(root.join(".armadai/prompts")).unwrap();
        std::fs::write(root.join("armadai.yaml"), "link:\n  target: claude\n").unwrap();
        std::fs::write(
            root.join(".armadai/agents.yaml"),
            "defaults:\n  provider: claude\nagents:\n  \
             - name: healthy-declared\n    prompt: [base]\n  \
             - name: shared-name\n    prompt: [base]\n",
        )
        .unwrap();
        std::fs::write(root.join(".armadai/prompts/base.md"), "You are {{name}}.\n").unwrap();
        dir
    }

    /// Catches a regression of the gate alone (`!config.agents.is_empty()`
    /// instead of `agent_source::project_declares_agents`) as well as the
    /// body alone (`project::resolve_all_agents` instead of
    /// `agent_source::load_all_agents`): either mutation, on this fixture,
    /// leaves the response without these two names.
    #[test]
    fn declared_agents_appear_for_a_declarations_only_project() {
        let dir = declared_only_project();
        let root = dir.path().join("project");
        let _guard = IsolatedProjectDir::enter(&root);

        let mut names: Vec<String> = load_agents().into_iter().map(|a| a.name).collect();
        names.sort_unstable();
        assert_eq!(
            names,
            vec!["healthy-declared", "shared-name"],
            "a project relying purely on .armadai/agents.yaml (no `agents:` list) must still \
             have its declared agents returned by /api/agents, not fall through to the global \
             library"
        );
    }

    /// The actual defect (#339 level 1): reverting the fix makes this
    /// declarations-only project's empty `config.agents` fall through
    /// entirely to the global library fallback, which then returns the
    /// homonym's own content under the declared agent's name -- worse than
    /// omission, a silent substitution (measured: `armadai list` shows the
    /// project's declared agents while `/api/agents` returns the
    /// global-library agent of the same name instead).
    #[test]
    fn a_declared_agent_is_not_shadowed_by_a_same_named_global_agent() {
        let dir = declared_only_project();
        let root = dir.path().join("project");
        let guard = IsolatedProjectDir::enter(&root);

        let global_agents = guard.global_agents_dir();
        std::fs::create_dir_all(&global_agents).unwrap();
        std::fs::write(
            global_agents.join("shared-name.md"),
            "# shared-name\n\n## Metadata\n- provider: global-marker-provider\n\n\
             ## System Prompt\n\nGlobal homonym.\n",
        )
        .unwrap();

        let agents = load_agents();

        assert!(
            !agents
                .iter()
                .any(|a| a.metadata.provider == "global-marker-provider"),
            "the project's declared 'shared-name' must never resolve to the global \
             library's same-named agent's content: {:?}",
            agents
                .iter()
                .map(|a| (&a.name, &a.metadata.provider))
                .collect::<Vec<_>>()
        );
        // The colliding declaration is refused outright (no precedence
        // between the two sides), but the unrelated declared agent must
        // still load -- proving the assertion above isn't a vacuous pass
        // from a totally broken load.
        assert!(
            agents.iter().any(|a| a.name == "healthy-declared"),
            "an unrelated declared agent must still load despite the collision: {:?}",
            agents.iter().map(|a| &a.name).collect::<Vec<_>>()
        );
    }
}
