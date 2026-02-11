use serde::{Deserialize, Serialize};

/// Shared context that can be passed between agents in a pipeline or hub & spoke execution.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SharedContext {
    /// Key-value pairs of contextual data
    pub data: std::collections::HashMap<String, String>,
    /// Accumulated conversation history
    pub messages: Vec<Message>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Role {
    System,
    User,
    Assistant,
}

impl SharedContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.data.insert(key.into(), value.into());
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.data.get(key).map(|s| s.as_str())
    }

    pub fn push_message(&mut self, role: Role, content: impl Into<String>) {
        self.messages.push(Message {
            role,
            content: content.into(),
        });
    }
}
