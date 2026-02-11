use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A task to be executed by an agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: Uuid,
    pub input: String,
    pub required_tags: Vec<String>,
    pub created_at: DateTime<Utc>,
}

/// The result of an agent execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResult {
    pub task_id: Uuid,
    pub agent_name: String,
    pub output: String,
    pub provider: String,
    pub model: String,
    pub tokens_in: u32,
    pub tokens_out: u32,
    pub cost: f64,
    pub duration_ms: u64,
    pub status: TaskStatus,
    pub completed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TaskStatus {
    Success,
    Error(String),
    Timeout,
}

impl Task {
    pub fn new(input: String) -> Self {
        Self {
            id: Uuid::new_v4(),
            input,
            required_tags: Vec::new(),
            created_at: Utc::now(),
        }
    }

    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.required_tags = tags;
        self
    }
}
