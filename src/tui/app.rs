use crate::core::agent::Agent;
use crate::core::prompt::Prompt;
use crate::core::skill::Skill;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Dashboard,
    AgentDetail,
    History,
    Costs,
    Prompts,
    Skills,
}

impl Tab {
    pub const ALL: [Tab; 6] = [
        Tab::Dashboard,
        Tab::AgentDetail,
        Tab::History,
        Tab::Costs,
        Tab::Prompts,
        Tab::Skills,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Tab::Dashboard => "Agents",
            Tab::AgentDetail => "Detail",
            Tab::History => "History",
            Tab::Costs => "Costs",
            Tab::Prompts => "Prompts",
            Tab::Skills => "Skills",
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
        vec![
            PaletteCommand {
                name: "agents".to_string(),
                description: "Switch to Agents dashboard".to_string(),
                action: PaletteAction::SwitchTab(Tab::Dashboard),
            },
            PaletteCommand {
                name: "detail".to_string(),
                description: "View selected agent detail".to_string(),
                action: PaletteAction::SwitchTab(Tab::AgentDetail),
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
        ]
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

pub struct App {
    pub current_tab: Tab,
    pub tab_index: usize,
    // Dashboard
    pub agents: Vec<Agent>,
    pub selected_agent: usize,
    // History
    pub history: Vec<RunEntry>,
    pub selected_history: usize,
    // Costs
    pub costs: Vec<CostEntry>,
    // Prompts
    pub prompts: Vec<Prompt>,
    // Skills
    pub skills: Vec<Skill>,
    // Command palette
    pub palette: CommandPalette,
    // Status message (bottom bar)
    pub status_msg: Option<String>,
}

impl App {
    pub fn new() -> Self {
        Self {
            current_tab: Tab::Dashboard,
            tab_index: 0,
            agents: Vec::new(),
            selected_agent: 0,
            history: Vec::new(),
            selected_history: 0,
            costs: Vec::new(),
            prompts: Vec::new(),
            skills: Vec::new(),
            palette: CommandPalette::new(),
            status_msg: None,
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
        let agents_dir = crate::core::config::AppPaths::resolve().agents_dir;
        match Agent::load_all(&agents_dir) {
            Ok(agents) => self.agents = agents,
            Err(e) => {
                self.status_msg = Some(format!("Failed to load agents: {e}"));
            }
        }
    }

    pub fn load_prompts(&mut self) {
        use crate::core::config::user_prompts_dir;
        use crate::core::prompt::load_all_prompts;
        self.prompts = load_all_prompts(&user_prompts_dir());
    }

    pub fn load_skills(&mut self) {
        use crate::core::config::user_skills_dir;
        use crate::core::skill::load_all_skills;
        self.skills = load_all_skills(&user_skills_dir());
    }

    pub fn selected_agent(&self) -> Option<&Agent> {
        self.agents.get(self.selected_agent)
    }

    pub fn select_next(&mut self) {
        match self.current_tab {
            Tab::Dashboard => {
                if !self.agents.is_empty() {
                    self.selected_agent = (self.selected_agent + 1) % self.agents.len();
                }
            }
            Tab::History => {
                if !self.history.is_empty() {
                    self.selected_history = (self.selected_history + 1) % self.history.len();
                }
            }
            _ => {}
        }
    }

    pub fn select_prev(&mut self) {
        match self.current_tab {
            Tab::Dashboard => {
                if !self.agents.is_empty() {
                    self.selected_agent = if self.selected_agent == 0 {
                        self.agents.len() - 1
                    } else {
                        self.selected_agent - 1
                    };
                }
            }
            Tab::History => {
                if !self.history.is_empty() {
                    self.selected_history = if self.selected_history == 0 {
                        self.history.len() - 1
                    } else {
                        self.selected_history - 1
                    };
                }
            }
            _ => {}
        }
    }
}
