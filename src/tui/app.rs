use crate::core::agent::Agent;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Dashboard,
    Execution,
    History,
    Costs,
}

impl Tab {
    pub const ALL: [Tab; 4] = [Tab::Dashboard, Tab::Execution, Tab::History, Tab::Costs];

    pub fn title(self) -> &'static str {
        match self {
            Tab::Dashboard => "Dashboard",
            Tab::Execution => "Execution",
            Tab::History => "History",
            Tab::Costs => "Costs",
        }
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

pub struct App {
    pub current_tab: Tab,
    pub tab_index: usize,
    // Dashboard
    pub agents: Vec<Agent>,
    pub selected_agent: usize,
    // Execution
    pub exec_output: Vec<String>,
    pub exec_running: bool,
    // History
    pub history: Vec<RunEntry>,
    pub selected_history: usize,
    // Costs
    pub costs: Vec<CostEntry>,
}

impl App {
    pub fn new() -> Self {
        Self {
            current_tab: Tab::Dashboard,
            tab_index: 0,
            agents: Vec::new(),
            selected_agent: 0,
            exec_output: Vec::new(),
            exec_running: false,
            history: Vec::new(),
            selected_history: 0,
            costs: Vec::new(),
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

    pub fn load_agents(&mut self) {
        let agents_dir = std::path::Path::new("agents");
        match Agent::load_all(agents_dir) {
            Ok(agents) => self.agents = agents,
            Err(e) => {
                self.exec_output.push(format!("Failed to load agents: {e}"));
            }
        }
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
