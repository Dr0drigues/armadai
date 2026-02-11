use super::agent::Agent;

/// Executes a chain of agents sequentially, passing each output as input to the next.
pub struct Pipeline {
    agents: Vec<Agent>,
}

impl Pipeline {
    pub fn new(agents: Vec<Agent>) -> Self {
        Self { agents }
    }

    /// Returns the ordered list of agents in the pipeline.
    pub fn stages(&self) -> &[Agent] {
        &self.agents
    }
}
