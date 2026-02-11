use super::agent::Agent;
use super::task::Task;

/// Hub & spoke coordinator that dispatches tasks to specialized agents.
pub struct Coordinator {
    agents: Vec<Agent>,
}

impl Coordinator {
    pub fn new(agents: Vec<Agent>) -> Self {
        Self { agents }
    }

    /// Find the best agent for a given task based on tags and stacks.
    pub fn select_agent(&self, task: &Task) -> Option<&Agent> {
        self.agents.iter().find(|a| {
            task.required_tags
                .iter()
                .all(|tag| a.metadata.tags.contains(tag))
        })
    }

    /// List all available agents.
    pub fn agents(&self) -> &[Agent] {
        &self.agents
    }
}
