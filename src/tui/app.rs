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

pub struct App {
    pub current_tab: Tab,
    pub tab_index: usize,
}

impl App {
    pub fn new() -> Self {
        Self {
            current_tab: Tab::Dashboard,
            tab_index: 0,
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
}
