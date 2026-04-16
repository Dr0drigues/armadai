use crate::core::agent::Agent;
use crate::core::prompt::Prompt;
use crate::core::skill::Skill;
use crate::core::starter::StarterPack;
use crate::model_registry::ModelEntry;

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
        }
    }

    pub fn next_tab(&mut self) {
        self.tab_index = (self.tab_index + 1) % Tab::ALL.len();
        self.current_tab = Tab::ALL[self.tab_index];
    }

    pub fn prev_tab(&mut self) {
        self.tab_index = if self.tab_index == 0 {
            Tab::ALL.len() - 1
        } else {
            self.tab_index - 1
        };
        self.current_tab = Tab::ALL[self.tab_index];
    }

    pub fn switch_tab(&mut self, tab: Tab) {
        self.current_tab = tab;
        self.tab_index = tab.index();
    }

    pub fn load_agents(&mut self) {
        use crate::core::config::is_force_global;
        use crate::core::project;

        // Project-aware agent loading
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
            self.agents = agents;
            return;
        }

        let agents_dir = crate::core::config::AppPaths::resolve().agents_dir;
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
        use crate::core::config::{is_force_global, user_prompts_dir};
        use crate::core::prompt::{Prompt, load_all_prompts};

        if !is_force_global()
            && let Some((root, config)) = crate::core::project::find_project_config()
            && !config.prompts.is_empty()
        {
            let (paths, _) = crate::core::project::resolve_all_prompts(&config, &root);
            self.prompts = paths.iter().filter_map(|p| Prompt::load(p).ok()).collect();
            return;
        }

        self.prompts = load_all_prompts(&user_prompts_dir());
    }

    pub fn load_skills(&mut self) {
        use crate::core::config::{is_force_global, user_skills_dir};
        use crate::core::skill::load_all_skills;

        if !is_force_global()
            && let Some((root, config)) = crate::core::project::find_project_config()
            && !config.skills.is_empty()
        {
            let (paths, _) = crate::core::project::resolve_all_skills(&config, &root);
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
        use crate::core::starter::load_all_packs;
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
        use crate::model_registry::fetch::load_all_providers_cached;
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
        use crate::storage::{init_db, queries};

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
