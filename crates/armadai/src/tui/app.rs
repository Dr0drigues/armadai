use armadai_core::agent::Agent;
use armadai_core::prompt::Prompt;
use armadai_core::skill::Skill;
use armadai_core::starter::StarterPack;
use armadai_providers::model_registry::ModelEntry;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum Tab {
    Dashboard,
    AgentDetail,
    Prompts,
    PromptDetail,
    Skills,
    SkillDetail,
    Starters,
    StarterDetail,
    History,
    Costs,
    Models,
    ModelDetail,
    Orchestration,
    OrchestrationDetail,
}

impl Tab {
    /// Tabs visible in the tab bar (detail tabs are accessed via Enter).
    pub const ALL: [Tab; 8] = [
        Tab::Dashboard,
        Tab::Prompts,
        Tab::Skills,
        Tab::Starters,
        Tab::History,
        Tab::Costs,
        Tab::Models,
        Tab::Orchestration,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Tab::Dashboard => "Agents",
            Tab::AgentDetail => "Detail",
            Tab::Prompts => "Prompts",
            Tab::PromptDetail => "Prompt",
            Tab::Skills => "Skills",
            Tab::SkillDetail => "Skill",
            Tab::Starters => "Starters",
            Tab::StarterDetail => "Starter",
            Tab::History => "History",
            Tab::Costs => "Costs",
            Tab::Models => "Models",
            Tab::ModelDetail => "Model",
            Tab::Orchestration => "Orchestration",
            Tab::OrchestrationDetail => "Run",
        }
    }

    pub fn index(self) -> usize {
        Tab::ALL.iter().position(|&t| t == self).unwrap_or(0)
    }
}

/// Lightweight copy of RunRecord for TUI display (no storage dependency).
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct RunEntry {
    pub agent: String,
    pub provider: String,
    pub model: String,
    pub tokens_in: i64,
    pub tokens_out: i64,
    pub cost: f64,
    pub duration_ms: i64,
    pub status: String,
    pub input_preview: String,
    pub output_preview: String,
    pub project: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CostEntry {
    pub agent: String,
    pub total_runs: i64,
    pub total_cost: f64,
    pub total_tokens_in: i64,
    pub total_tokens_out: i64,
}

/// Lightweight copy of OrchestrationRunRecord for TUI display (gated by storage feature).
#[derive(Debug, Clone)]
#[cfg(feature = "storage")]
pub struct OrchestrationEntry {
    pub run_id: String,
    pub pattern: String,
    pub rounds: i64,
    pub halt_reason: Option<String>,
}

/// Command palette state
pub struct CommandPalette {
    pub visible: bool,
    pub input: String,
    pub filtered: Vec<PaletteCommand>,
    pub selected: usize,
}

#[derive(Debug, Clone)]
pub struct PaletteCommand {
    pub name: String,
    pub description: String,
    pub action: PaletteAction,
}

#[derive(Debug, Clone)]
pub enum PaletteAction {
    SwitchTab(Tab),
    Refresh,
    Quit,
    NewAgent,
}

impl CommandPalette {
    pub fn new() -> Self {
        Self {
            visible: false,
            input: String::new(),
            filtered: Self::all_commands(),
            selected: 0,
        }
    }

    fn all_commands() -> Vec<PaletteCommand> {
        let mut cmds = vec![
            PaletteCommand {
                name: "agents".to_string(),
                description: "Switch to Agents dashboard".to_string(),
                action: PaletteAction::SwitchTab(Tab::Dashboard),
            },
            PaletteCommand {
                name: "prompts".to_string(),
                description: "View prompts library".to_string(),
                action: PaletteAction::SwitchTab(Tab::Prompts),
            },
            PaletteCommand {
                name: "skills".to_string(),
                description: "View skills library".to_string(),
                action: PaletteAction::SwitchTab(Tab::Skills),
            },
            PaletteCommand {
                name: "starters".to_string(),
                description: "View starter packs".to_string(),
                action: PaletteAction::SwitchTab(Tab::Starters),
            },
            PaletteCommand {
                name: "history".to_string(),
                description: "View execution history".to_string(),
                action: PaletteAction::SwitchTab(Tab::History),
            },
            PaletteCommand {
                name: "costs".to_string(),
                description: "View cost tracking".to_string(),
                action: PaletteAction::SwitchTab(Tab::Costs),
            },
            PaletteCommand {
                name: "models".to_string(),
                description: "View model catalog".to_string(),
                action: PaletteAction::SwitchTab(Tab::Models),
            },
        ];

        #[cfg(feature = "storage")]
        cmds.push(PaletteCommand {
            name: "orchestration".to_string(),
            description: "View orchestration runs".to_string(),
            action: PaletteAction::SwitchTab(Tab::Orchestration),
        });

        cmds.extend(vec![
            PaletteCommand {
                name: "refresh".to_string(),
                description: "Reload agents and data".to_string(),
                action: PaletteAction::Refresh,
            },
            PaletteCommand {
                name: "new".to_string(),
                description: "Create a new agent (run armadai new)".to_string(),
                action: PaletteAction::NewAgent,
            },
            PaletteCommand {
                name: "quit".to_string(),
                description: "Exit the application".to_string(),
                action: PaletteAction::Quit,
            },
        ]);

        cmds
    }

    pub fn open(&mut self) {
        self.visible = true;
        self.input.clear();
        self.filtered = Self::all_commands();
        self.selected = 0;
    }

    pub fn close(&mut self) {
        self.visible = false;
        self.input.clear();
    }

    pub fn update_filter(&mut self) {
        let query = self.input.to_lowercase();
        self.filtered = Self::all_commands()
            .into_iter()
            .filter(|cmd| {
                cmd.name.contains(&query) || cmd.description.to_lowercase().contains(&query)
            })
            .collect();
        if self.selected >= self.filtered.len() {
            self.selected = 0;
        }
    }

    pub fn select_next(&mut self) {
        if !self.filtered.is_empty() {
            self.selected = (self.selected + 1) % self.filtered.len();
        }
    }

    pub fn select_prev(&mut self) {
        if !self.filtered.is_empty() {
            self.selected = if self.selected == 0 {
                self.filtered.len() - 1
            } else {
                self.selected - 1
            };
        }
    }

    pub fn execute(&self) -> Option<PaletteAction> {
        self.filtered.get(self.selected).map(|c| c.action.clone())
    }
}

/// Sort mode for lists
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortMode {
    Default,
    NameAsc,
    NameDesc,
}

/// How many lines a single PageUp/PageDown jumps in a scrollable detail
/// view. Mirrors the shell TUI's popup page-scroll step
/// (`src/shell/tui.rs`'s `popup_scroll` PageUp/PageDown handling).
const DETAIL_PAGE_SIZE: u16 = 10;

pub struct App {
    pub current_tab: Tab,
    pub tab_index: usize,
    // Dashboard
    pub agents: Vec<Agent>,
    pub selected_agent: usize,
    // Prompts
    pub prompts: Vec<Prompt>,
    pub selected_prompt: usize,
    // Skills
    pub skills: Vec<Skill>,
    pub selected_skill: usize,
    // Starters
    pub starters: Vec<StarterPack>,
    pub selected_starter: usize,
    // History
    pub history: Vec<RunEntry>,
    pub selected_history: usize,
    // Costs
    pub costs: Vec<CostEntry>,
    pub selected_cost: usize,
    // Models (from model registry cache)
    pub models_flat: Vec<(String, ModelEntry)>,
    pub selected_model: usize,
    // Orchestration (gated by storage feature)
    #[cfg(feature = "storage")]
    pub orchestration_runs: Vec<OrchestrationEntry>,
    #[cfg(feature = "storage")]
    pub selected_orchestration: usize,
    // Command palette
    pub palette: CommandPalette,
    // Status message (bottom bar)
    pub status_msg: Option<String>,
    // Search & sort
    pub search_mode: bool,
    pub search_query: String,
    pub sort_mode: SortMode,
    // Detail view scroll offset (reset on tab switch / selection change).
    pub detail_scroll: u16,
    // Upper bound for `detail_scroll`, recomputed each render pass by the
    // active detail view from its actual content + panel size (see
    // `tui::wrap::wrapped_line_count`). Zero when content fits without
    // scrolling, or no detail view is active.
    pub detail_scroll_max: u16,
    // "Press Esc again to quit" arming (top-level safety net — mirrors
    // `src/shell/tui.rs`'s `esc_armed`). Set when Esc is pressed on a
    // top-level list view (no popup/search/detail active); a second,
    // consecutive Esc then confirms the quit. Any other key disarms it.
    pub esc_armed: bool,
}

impl App {
    pub fn new() -> Self {
        Self {
            current_tab: Tab::Dashboard,
            tab_index: 0,
            agents: Vec::new(),
            selected_agent: 0,
            prompts: Vec::new(),
            selected_prompt: 0,
            skills: Vec::new(),
            selected_skill: 0,
            starters: Vec::new(),
            selected_starter: 0,
            history: Vec::new(),
            selected_history: 0,
            costs: Vec::new(),
            selected_cost: 0,
            models_flat: Vec::new(),
            selected_model: 0,
            #[cfg(feature = "storage")]
            orchestration_runs: Vec::new(),
            #[cfg(feature = "storage")]
            selected_orchestration: 0,
            palette: CommandPalette::new(),
            status_msg: None,
            search_mode: false,
            search_query: String::new(),
            sort_mode: SortMode::Default,
            detail_scroll: 0,
            detail_scroll_max: 0,
            esc_armed: false,
        }
    }

    pub fn next_tab(&mut self) {
        self.tab_index = (self.tab_index + 1) % Tab::ALL.len();
        self.current_tab = Tab::ALL[self.tab_index];
        self.detail_scroll = 0;
        self.detail_scroll_max = 0;
    }

    pub fn prev_tab(&mut self) {
        self.tab_index = if self.tab_index == 0 {
            Tab::ALL.len() - 1
        } else {
            self.tab_index - 1
        };
        self.current_tab = Tab::ALL[self.tab_index];
        self.detail_scroll = 0;
        self.detail_scroll_max = 0;
    }

    pub fn switch_tab(&mut self, tab: Tab) {
        self.current_tab = tab;
        self.tab_index = tab.index();
        self.detail_scroll = 0;
        self.detail_scroll_max = 0;
    }

    /// Scroll a detail view down by one line, bounds-checked against
    /// `detail_scroll_max` (computed from the panel's actual content on the
    /// last render pass) so it never scrolls past the content's end.
    pub fn scroll_detail_down(&mut self) {
        self.detail_scroll = self
            .detail_scroll
            .saturating_add(1)
            .min(self.detail_scroll_max);
    }

    /// Scroll a detail view up by one line (saturates at 0).
    pub fn scroll_detail_up(&mut self) {
        self.detail_scroll = self.detail_scroll.saturating_sub(1);
    }

    /// Scroll a detail view down by one page, bounds-checked.
    pub fn scroll_detail_page_down(&mut self) {
        self.detail_scroll = self
            .detail_scroll
            .saturating_add(DETAIL_PAGE_SIZE)
            .min(self.detail_scroll_max);
    }

    /// Scroll a detail view up by one page (saturates at 0).
    pub fn scroll_detail_page_up(&mut self) {
        self.detail_scroll = self.detail_scroll.saturating_sub(DETAIL_PAGE_SIZE);
    }

    /// Update the active detail view's scroll bound. Called once per render
    /// pass by whichever detail view is currently on screen, from its
    /// actual content + panel size — also clamps the stored offset in case
    /// a terminal resize (or switching to shorter content) shrank the
    /// scrollable range out from under it.
    pub fn set_detail_scroll_max(&mut self, max: u16) {
        self.detail_scroll_max = max;
        if self.detail_scroll > max {
            self.detail_scroll = max;
        }
    }

    pub fn load_agents(&mut self) {
        use armadai_core::agent_source;
        use armadai_core::config::is_force_global;
        use armadai_core::project;

        // Project-aware agent loading. `project_declares_agents` (rather
        // than a bare `!config.agents.is_empty()` check) so a project that
        // relies purely on `.armadai/agents.yaml` — no `agents:` list at
        // all — still takes this branch instead of falling through to the
        // global library below, and `load_all_agents` (rather than
        // `project::resolve_all_agents`, which only ever resolves file
        // paths and silently drops every declared agent) so a declared
        // agent is actually loaded instead of leaving a same-named global
        // agent to be shown in its place.
        if !is_force_global()
            && let Some((root, config)) = project::find_project_config()
            && agent_source::project_declares_agents(&root, &config)
        {
            let fragments = agent_source::project_fragments(&root);
            let (agents, warnings) = agent_source::load_all_agents(&config, &root, &fragments);
            self.agents = agents;
            // Read-only view: never refuse over a load warning, just
            // surface it instead of letting it vanish unnoticed (the
            // silent-substitution defect this wiring fixes lived
            // unnoticed for exactly that reason).
            self.status_msg = warnings.first().map(|w| {
                if warnings.len() > 1 {
                    format!(
                        "{} agent load warning(s) — first: {}",
                        warnings.len(),
                        w.message()
                    )
                } else {
                    format!("agent load warning: {}", w.message())
                }
            });
            return;
        }

        let agents_dir = armadai_core::config::AppPaths::resolve().agents_dir;
        match Agent::load_all_with_skipped(&agents_dir) {
            Ok((agents, skipped)) => {
                self.agents = agents;
                if !skipped.is_empty() {
                    self.status_msg = Some(format!(
                        "{} agent file(s) skipped (malformed)",
                        skipped.len()
                    ));
                }
            }
            Err(e) => {
                self.status_msg = Some(format!("Failed to load agents: {e}"));
            }
        }
    }

    pub fn load_prompts(&mut self) {
        use armadai_core::config::{is_force_global, user_prompts_dir};
        use armadai_core::prompt::{Prompt, load_all_prompts};

        if !is_force_global()
            && let Some((root, config)) = armadai_core::project::find_project_config()
            && !config.prompts.is_empty()
        {
            let (paths, _) = armadai_core::project::resolve_all_prompts(&config, &root);
            self.prompts = paths.iter().filter_map(|p| Prompt::load(p).ok()).collect();
            return;
        }

        self.prompts = load_all_prompts(&user_prompts_dir());
    }

    pub fn load_skills(&mut self) {
        use armadai_core::config::{is_force_global, user_skills_dir};
        use armadai_core::skill::load_all_skills;

        if !is_force_global()
            && let Some((root, config)) = armadai_core::project::find_project_config()
            && !config.skills.is_empty()
        {
            let (paths, _) = armadai_core::project::resolve_all_skills(&config, &root);
            let mut skills = Vec::new();
            for path in &paths {
                skills.extend(load_all_skills(path));
            }
            self.skills = skills;
            return;
        }

        self.skills = load_all_skills(&user_skills_dir());
    }

    pub fn load_starters(&mut self) {
        use armadai_core::starter::load_all_packs;
        self.starters = load_all_packs();
    }

    pub fn selected_agent(&self) -> Option<&Agent> {
        use crate::tui::filter;
        let display_indices =
            filter::apply_filter_and_sort_agents(&self.agents, &self.search_query, self.sort_mode);
        display_indices
            .get(self.selected_agent)
            .and_then(|&idx| self.agents.get(idx))
    }

    pub fn selected_prompt(&self) -> Option<&Prompt> {
        use crate::tui::filter;
        let display_indices = filter::apply_filter_and_sort_prompts(
            &self.prompts,
            &self.search_query,
            self.sort_mode,
        );
        display_indices
            .get(self.selected_prompt)
            .and_then(|&idx| self.prompts.get(idx))
    }

    pub fn selected_skill(&self) -> Option<&Skill> {
        use crate::tui::filter;
        let display_indices =
            filter::apply_filter_and_sort_skills(&self.skills, &self.search_query, self.sort_mode);
        display_indices
            .get(self.selected_skill)
            .and_then(|&idx| self.skills.get(idx))
    }

    pub fn selected_starter(&self) -> Option<&StarterPack> {
        use crate::tui::filter;
        let display_indices = filter::apply_filter_and_sort_starters(
            &self.starters,
            &self.search_query,
            self.sort_mode,
        );
        display_indices
            .get(self.selected_starter)
            .and_then(|&idx| self.starters.get(idx))
    }

    pub fn selected_model_entry(&self) -> Option<&(String, ModelEntry)> {
        use crate::tui::filter;
        let display_indices = filter::apply_filter_and_sort_models(
            &self.models_flat,
            &self.search_query,
            self.sort_mode,
        );
        display_indices
            .get(self.selected_model)
            .and_then(|&idx| self.models_flat.get(idx))
    }

    #[cfg(feature = "storage")]
    pub fn selected_orchestration_entry(&self) -> Option<&OrchestrationEntry> {
        use crate::tui::filter;
        let display_indices = filter::apply_filter_and_sort_orchestration(
            &self.orchestration_runs,
            &self.search_query,
            self.sort_mode,
        );
        display_indices
            .get(self.selected_orchestration)
            .and_then(|&idx| self.orchestration_runs.get(idx))
    }

    pub fn cycle_sort_mode(&mut self) {
        self.sort_mode = match self.sort_mode {
            SortMode::Default => SortMode::NameAsc,
            SortMode::NameAsc => SortMode::NameDesc,
            SortMode::NameDesc => SortMode::Default,
        };
        // Reset selection to 0 when sorting changes
        self.selected_agent = 0;
        self.selected_prompt = 0;
        self.selected_skill = 0;
        self.selected_starter = 0;
        self.selected_history = 0;
        self.selected_cost = 0;
        self.selected_model = 0;
        #[cfg(feature = "storage")]
        {
            self.selected_orchestration = 0;
        }
    }

    pub fn clear_search(&mut self) {
        self.search_query.clear();
        self.search_mode = false;
        // Reset selection when clearing search
        self.selected_agent = 0;
        self.selected_prompt = 0;
        self.selected_skill = 0;
        self.selected_starter = 0;
        self.selected_history = 0;
        self.selected_cost = 0;
        self.selected_model = 0;
        #[cfg(feature = "storage")]
        {
            self.selected_orchestration = 0;
        }
    }

    pub fn sort_indicator(&self) -> &'static str {
        match self.sort_mode {
            SortMode::Default => "",
            SortMode::NameAsc => " (A→Z)",
            SortMode::NameDesc => " (Z→A)",
        }
    }

    pub fn load_models(&mut self) {
        use armadai_providers::model_registry::fetch::load_all_providers_cached;
        if let Some(providers) = load_all_providers_cached() {
            let mut flat: Vec<(String, ModelEntry)> = Vec::new();
            let mut keys: Vec<String> = providers.keys().cloned().collect();
            keys.sort();
            for provider in keys {
                if let Some(models) = providers.get(&provider) {
                    for entry in models {
                        flat.push((provider.clone(), entry.clone()));
                    }
                }
            }
            self.models_flat = flat;
        }
    }

    #[cfg(feature = "storage")]
    pub fn load_orchestration_runs(&mut self) {
        use crate::db::init_db;
        use armadai_storage::queries;

        let db = match init_db() {
            Ok(db) => db,
            Err(_) => return,
        };

        if let Ok(records) = queries::get_orchestration_runs(&db, 100) {
            self.orchestration_runs = records
                .into_iter()
                .map(|r| OrchestrationEntry {
                    run_id: r.run_id,
                    pattern: r.pattern,
                    rounds: r.rounds,
                    halt_reason: r.halt_reason,
                })
                .collect();
        }
    }

    pub fn select_next(&mut self) {
        use crate::tui::filter;
        match self.current_tab {
            Tab::Dashboard => {
                let display_indices = filter::apply_filter_and_sort_agents(
                    &self.agents,
                    &self.search_query,
                    self.sort_mode,
                );
                if !display_indices.is_empty() {
                    self.selected_agent = (self.selected_agent + 1) % display_indices.len();
                }
            }
            Tab::Prompts => {
                let display_indices = filter::apply_filter_and_sort_prompts(
                    &self.prompts,
                    &self.search_query,
                    self.sort_mode,
                );
                if !display_indices.is_empty() {
                    self.selected_prompt = (self.selected_prompt + 1) % display_indices.len();
                }
            }
            Tab::Skills => {
                let display_indices = filter::apply_filter_and_sort_skills(
                    &self.skills,
                    &self.search_query,
                    self.sort_mode,
                );
                if !display_indices.is_empty() {
                    self.selected_skill = (self.selected_skill + 1) % display_indices.len();
                }
            }
            Tab::Starters => {
                let display_indices = filter::apply_filter_and_sort_starters(
                    &self.starters,
                    &self.search_query,
                    self.sort_mode,
                );
                if !display_indices.is_empty() {
                    self.selected_starter = (self.selected_starter + 1) % display_indices.len();
                }
            }
            Tab::History => {
                let display_indices = filter::apply_filter_and_sort_history(
                    &self.history,
                    &self.search_query,
                    self.sort_mode,
                );
                if !display_indices.is_empty() {
                    self.selected_history = (self.selected_history + 1) % display_indices.len();
                }
            }
            Tab::Costs => {
                let display_indices = filter::apply_filter_and_sort_costs(
                    &self.costs,
                    &self.search_query,
                    self.sort_mode,
                );
                if !display_indices.is_empty() {
                    self.selected_cost = (self.selected_cost + 1) % display_indices.len();
                }
            }
            Tab::Models => {
                let display_indices = filter::apply_filter_and_sort_models(
                    &self.models_flat,
                    &self.search_query,
                    self.sort_mode,
                );
                if !display_indices.is_empty() {
                    self.selected_model = (self.selected_model + 1) % display_indices.len();
                }
            }
            #[cfg(feature = "storage")]
            Tab::Orchestration => {
                let display_indices = filter::apply_filter_and_sort_orchestration(
                    &self.orchestration_runs,
                    &self.search_query,
                    self.sort_mode,
                );
                if !display_indices.is_empty() {
                    self.selected_orchestration =
                        (self.selected_orchestration + 1) % display_indices.len();
                }
            }
            _ => {}
        }
    }

    pub fn select_prev(&mut self) {
        use crate::tui::filter;
        match self.current_tab {
            Tab::Dashboard => {
                let display_indices = filter::apply_filter_and_sort_agents(
                    &self.agents,
                    &self.search_query,
                    self.sort_mode,
                );
                if !display_indices.is_empty() {
                    self.selected_agent = if self.selected_agent == 0 {
                        display_indices.len() - 1
                    } else {
                        self.selected_agent - 1
                    };
                }
            }
            Tab::Prompts => {
                let display_indices = filter::apply_filter_and_sort_prompts(
                    &self.prompts,
                    &self.search_query,
                    self.sort_mode,
                );
                if !display_indices.is_empty() {
                    self.selected_prompt = if self.selected_prompt == 0 {
                        display_indices.len() - 1
                    } else {
                        self.selected_prompt - 1
                    };
                }
            }
            Tab::Skills => {
                let display_indices = filter::apply_filter_and_sort_skills(
                    &self.skills,
                    &self.search_query,
                    self.sort_mode,
                );
                if !display_indices.is_empty() {
                    self.selected_skill = if self.selected_skill == 0 {
                        display_indices.len() - 1
                    } else {
                        self.selected_skill - 1
                    };
                }
            }
            Tab::Starters => {
                let display_indices = filter::apply_filter_and_sort_starters(
                    &self.starters,
                    &self.search_query,
                    self.sort_mode,
                );
                if !display_indices.is_empty() {
                    self.selected_starter = if self.selected_starter == 0 {
                        display_indices.len() - 1
                    } else {
                        self.selected_starter - 1
                    };
                }
            }
            Tab::History => {
                let display_indices = filter::apply_filter_and_sort_history(
                    &self.history,
                    &self.search_query,
                    self.sort_mode,
                );
                if !display_indices.is_empty() {
                    self.selected_history = if self.selected_history == 0 {
                        display_indices.len() - 1
                    } else {
                        self.selected_history - 1
                    };
                }
            }
            Tab::Costs => {
                let display_indices = filter::apply_filter_and_sort_costs(
                    &self.costs,
                    &self.search_query,
                    self.sort_mode,
                );
                if !display_indices.is_empty() {
                    self.selected_cost = if self.selected_cost == 0 {
                        display_indices.len() - 1
                    } else {
                        self.selected_cost - 1
                    };
                }
            }
            Tab::Models => {
                let display_indices = filter::apply_filter_and_sort_models(
                    &self.models_flat,
                    &self.search_query,
                    self.sort_mode,
                );
                if !display_indices.is_empty() {
                    self.selected_model = if self.selected_model == 0 {
                        display_indices.len() - 1
                    } else {
                        self.selected_model - 1
                    };
                }
            }
            #[cfg(feature = "storage")]
            Tab::Orchestration => {
                let display_indices = filter::apply_filter_and_sort_orchestration(
                    &self.orchestration_runs,
                    &self.search_query,
                    self.sort_mode,
                );
                if !display_indices.is_empty() {
                    self.selected_orchestration = if self.selected_orchestration == 0 {
                        display_indices.len() - 1
                    } else {
                        self.selected_orchestration - 1
                    };
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod detail_scroll_tests {
    use super::*;

    #[test]
    fn scroll_down_is_bounds_checked_against_max() {
        let mut app = App::new();
        app.set_detail_scroll_max(3);
        for _ in 0..10 {
            app.scroll_detail_down();
        }
        assert_eq!(
            app.detail_scroll, 3,
            "scrolling down must never exceed the content's end"
        );
    }

    #[test]
    fn scroll_up_never_goes_negative() {
        let mut app = App::new();
        app.set_detail_scroll_max(5);
        app.detail_scroll = 1;
        app.scroll_detail_up();
        app.scroll_detail_up();
        assert_eq!(
            app.detail_scroll, 0,
            "scrolling up must never go past the content's start"
        );
    }

    #[test]
    fn page_down_is_bounds_checked_against_max() {
        let mut app = App::new();
        app.set_detail_scroll_max(3);
        app.scroll_detail_page_down();
        assert_eq!(
            app.detail_scroll, 3,
            "a page jump larger than the remaining content must clamp to the end"
        );
    }

    #[test]
    fn shrinking_max_clamps_an_out_of_range_offset() {
        let mut app = App::new();
        app.set_detail_scroll_max(20);
        app.detail_scroll = 15;
        // Simulates a render pass discovering the panel/content got
        // smaller (e.g. terminal resize) — the stored offset must not be
        // left pointing past the new end.
        app.set_detail_scroll_max(5);
        assert_eq!(app.detail_scroll, 5);
        assert_eq!(app.detail_scroll_max, 5);
    }

    #[test]
    fn switching_tab_resets_scroll_and_bound() {
        let mut app = App::new();
        app.detail_scroll = 7;
        app.set_detail_scroll_max(12);
        app.switch_tab(Tab::AgentDetail);
        assert_eq!(app.detail_scroll, 0);
        assert_eq!(app.detail_scroll_max, 0);
    }
}

#[cfg(test)]
mod esc_armed_tests {
    use super::*;

    #[test]
    fn esc_armed_starts_disarmed() {
        let app = App::new();
        assert!(!app.esc_armed);
    }
}

#[cfg(test)]
mod load_agents_declarative_tests {
    use super::*;
    use armadai_core::config::ENV_MUTEX;

    /// Points `ARMADAI_CONFIG_DIR` at a fresh, empty temp dir and the
    /// process's cwd at `project_root` for its lifetime, restoring both on
    /// drop (even on panic via a plain `Drop` impl, not a try/finally).
    /// `App::load_agents` resolves its project via the cwd-based
    /// `project::find_project_config()` and its global fallback via the
    /// env-var-based `user_agents_dir()` -- both process-global state, so
    /// this is serialised on `ENV_MUTEX` (shared with the rest of the
    /// workspace's env-mutating tests) to avoid racing a concurrently
    /// running test that reads either one unguarded.
    struct ProjectDirGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
        orig_cwd: std::path::PathBuf,
        orig_config_dir: Option<String>,
        config_tmp: tempfile::TempDir,
    }

    impl ProjectDirGuard {
        fn enter(project_root: &std::path::Path) -> Self {
            let lock = ENV_MUTEX.lock().unwrap();
            let orig_cwd = std::env::current_dir().unwrap();
            let orig_config_dir = std::env::var("ARMADAI_CONFIG_DIR").ok();
            let config_tmp = tempfile::tempdir().unwrap();
            // SAFETY: serialised via ENV_MUTEX above.
            unsafe {
                std::env::set_var("ARMADAI_CONFIG_DIR", config_tmp.path());
            }
            std::env::set_current_dir(project_root).unwrap();
            Self {
                _lock: lock,
                orig_cwd,
                orig_config_dir,
                config_tmp,
            }
        }

        /// The isolated global agent library, for a test that wants to
        /// plant a same-named `.md` there to check it isn't shadowing a
        /// declared agent.
        fn global_agents_dir(&self) -> std::path::PathBuf {
            self.config_tmp.path().join("agents")
        }
    }

    impl Drop for ProjectDirGuard {
        fn drop(&mut self) {
            let _ = std::env::set_current_dir(&self.orig_cwd);
            // SAFETY: restoring original env state, still under the guard
            // held by `self._lock` until this `Drop` returns.
            unsafe {
                match &self.orig_config_dir {
                    Some(v) => std::env::set_var("ARMADAI_CONFIG_DIR", v),
                    None => std::env::remove_var("ARMADAI_CONFIG_DIR"),
                }
            }
        }
    }

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
    /// leaves `app.agents` without these two names.
    #[test]
    fn declared_agents_appear_for_a_declarations_only_project() {
        let dir = declared_only_project();
        let root = dir.path().join("project");
        let _guard = ProjectDirGuard::enter(&root);

        let mut app = App::new();
        app.load_agents();

        let mut names: Vec<&str> = app.agents.iter().map(|a| a.name.as_str()).collect();
        names.sort_unstable();
        assert_eq!(
            names,
            vec!["healthy-declared", "shared-name"],
            "a project relying purely on .armadai/agents.yaml (no `agents:` list) must still \
             have its declared agents loaded by the TUI, not fall through to the global library"
        );
    }

    /// The actual defect (#339 level 1): reverting the fix makes this
    /// declarations-only project's empty `config.agents` fall through
    /// entirely to the global library fallback, which then returns the
    /// homonym's own content under the declared agent's name -- worse than
    /// omission, a silent substitution.
    #[test]
    fn a_declared_agent_is_not_shadowed_by_a_same_named_global_agent() {
        let dir = declared_only_project();
        let root = dir.path().join("project");
        let guard = ProjectDirGuard::enter(&root);

        let global_agents = guard.global_agents_dir();
        std::fs::create_dir_all(&global_agents).unwrap();
        std::fs::write(
            global_agents.join("shared-name.md"),
            "# shared-name\n\n## Metadata\n- provider: global-marker-provider\n\n\
             ## System Prompt\n\nGlobal homonym.\n",
        )
        .unwrap();

        let mut app = App::new();
        app.load_agents();

        assert!(
            !app.agents
                .iter()
                .any(|a| a.metadata.provider == "global-marker-provider"),
            "the project's declared 'shared-name' must never resolve to the global \
             library's same-named agent's content: {:?}",
            app.agents
                .iter()
                .map(|a| (&a.name, &a.metadata.provider))
                .collect::<Vec<_>>()
        );
        // The colliding declaration is refused outright (no precedence
        // between the two sides), but the unrelated declared agent must
        // still load -- proving the assertion above isn't a vacuous pass
        // from a totally broken load.
        assert!(
            app.agents.iter().any(|a| a.name == "healthy-declared"),
            "an unrelated declared agent must still load despite the collision: {:?}",
            app.agents.iter().map(|a| &a.name).collect::<Vec<_>>()
        );
        assert!(
            app.status_msg
                .as_deref()
                .is_some_and(|m| m.contains("shared-name")),
            "the collision must be surfaced to the user, not silently dropped: {:?}",
            app.status_msg
        );
    }
}
