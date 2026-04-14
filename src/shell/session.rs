//! Session persistence for the ArmadAI shell.
//!
//! Sessions are stored as JSON files in `~/.config/armadai/sessions/`.
//! Each session captures the full conversation state, provider config, and metrics.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use super::runner::{Message, MessageRole};

/// A saved shell session with full conversation state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShellSession {
    /// Unique session ID (timestamp-based or UUID)
    pub id: String,
    /// Human-readable name (auto-generated or user-set)
    pub name: String,
    /// Provider command used (e.g., "gemini", "claude")
    pub provider: String,
    /// Model name
    pub model: String,
    /// Project directory where session was started
    pub project_dir: String,
    /// Creation timestamp (ISO 8601)
    pub created_at: String,
    /// Last update timestamp
    pub updated_at: String,
    /// Conversation messages
    pub messages: Vec<SessionMessage>,
    /// Cumulative metrics
    pub total_tokens_in: usize,
    pub total_tokens_out: usize,
    pub total_cost: f64,
    pub turn_count: u32,
}

/// A message within a session (serializable version of runner::Message).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMessage {
    pub role: String, // "user", "assistant", "system"
    pub content: String,
    pub timestamp: String,
}

impl SessionMessage {
    /// Convert from runner::Message to SessionMessage.
    pub fn from_message(msg: &Message) -> Self {
        Self {
            role: match msg.role {
                MessageRole::User => "user".to_string(),
                MessageRole::Assistant => "assistant".to_string(),
                MessageRole::System => "system".to_string(),
            },
            content: msg.content.clone(),
            timestamp: chrono::Utc::now().to_rfc3339(),
        }
    }

    /// Convert SessionMessage back to runner::Message.
    pub fn to_message(&self) -> Message {
        Message {
            role: match self.role.as_str() {
                "user" => MessageRole::User,
                "assistant" => MessageRole::Assistant,
                "system" => MessageRole::System,
                _ => MessageRole::System, // Fallback for unknown roles
            },
            content: self.content.clone(),
            metrics: None, // Metrics are not preserved on reload
        }
    }
}

/// Get the sessions directory (`~/.config/armadai/sessions/`).
pub fn sessions_dir() -> PathBuf {
    use crate::core::config::config_dir;
    let dir = config_dir().join("sessions");
    std::fs::create_dir_all(&dir).ok();
    dir
}

/// Generate a new session ID based on current timestamp.
pub fn new_session_id() -> String {
    chrono::Utc::now().format("%Y%m%d_%H%M%S").to_string()
}

/// Generate a human-readable session name from project directory.
pub fn generate_session_name(project_dir: &str) -> String {
    let path = std::path::Path::new(project_dir);
    let dir_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");
    format!("{} session", dir_name)
}

/// Save a session to disk.
pub fn save_session(session: &ShellSession) -> Result<()> {
    let path = sessions_dir().join(format!("{}.json", session.id));
    let json = serde_json::to_string_pretty(session)?;
    std::fs::write(path, json)?;
    Ok(())
}

/// Append a raw stream event to the session debug log.
pub fn log_stream_event(session_id: &str, event: &str) {
    let path = sessions_dir().join(format!("{}.log", session_id));
    use std::io::Write;
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(file, "{}", event);
    }
}

/// Load a session from disk.
pub fn load_session(id: &str) -> Result<ShellSession> {
    let path = sessions_dir().join(format!("{}.json", id));
    let json = std::fs::read_to_string(path)?;
    Ok(serde_json::from_str(&json)?)
}

/// List all saved sessions (sorted by last update, newest first).
pub fn list_sessions() -> Vec<ShellSession> {
    let dir = sessions_dir();
    let mut sessions = Vec::new();

    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            if entry.path().extension().is_some_and(|e| e == "json")
                && let Ok(json) = std::fs::read_to_string(entry.path())
                && let Ok(session) = serde_json::from_str::<ShellSession>(&json)
            {
                sessions.push(session);
            }
        }
    }

    // Sort by updated_at, newest first
    sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    sessions
}

/// Delete a session.
pub fn delete_session(id: &str) -> Result<()> {
    let path = sessions_dir().join(format!("{}.json", id));
    std::fs::remove_file(path)?;
    Ok(())
}

/// Format a relative time string (e.g., "2 hours ago", "yesterday").
pub fn format_relative_time(timestamp: &str) -> String {
    use chrono::{DateTime, Utc};

    let Ok(dt) = DateTime::parse_from_rfc3339(timestamp) else {
        return "unknown".to_string();
    };

    let now = Utc::now();
    let duration = now.signed_duration_since(dt.with_timezone(&Utc));

    if duration.num_seconds() < 60 {
        "just now".to_string()
    } else if duration.num_minutes() < 60 {
        let mins = duration.num_minutes();
        format!("{} min{} ago", mins, if mins == 1 { "" } else { "s" })
    } else if duration.num_hours() < 24 {
        let hours = duration.num_hours();
        format!("{} hour{} ago", hours, if hours == 1 { "" } else { "s" })
    } else if duration.num_days() == 1 {
        "yesterday".to_string()
    } else if duration.num_days() < 7 {
        format!("{} days ago", duration.num_days())
    } else if duration.num_weeks() < 4 {
        let weeks = duration.num_weeks();
        format!("{} week{} ago", weeks, if weeks == 1 { "" } else { "s" })
    } else if duration.num_days() < 365 {
        let months = duration.num_days() / 30;
        format!("{} month{} ago", months, if months == 1 { "" } else { "s" })
    } else {
        let years = duration.num_days() / 365;
        format!("{} year{} ago", years, if years == 1 { "" } else { "s" })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_session_id() {
        let id = new_session_id();
        // Should be in format YYYYMMDD_HHMMSS
        assert_eq!(id.len(), 15); // "20260401_143022" = 15 chars
        assert!(id.contains('_'));
    }

    #[test]
    fn test_generate_session_name() {
        let name = generate_session_name("/home/user/projects/armadai");
        assert_eq!(name, "armadai session");

        let name = generate_session_name(".");
        assert!(name.contains("session"));
    }

    #[test]
    fn test_session_message_conversion() {
        let msg = Message {
            role: MessageRole::User,
            content: "Hello".to_string(),
            metrics: None,
        };

        let session_msg = SessionMessage::from_message(&msg);
        assert_eq!(session_msg.role, "user");
        assert_eq!(session_msg.content, "Hello");

        let converted = session_msg.to_message();
        assert_eq!(converted.role, MessageRole::User);
        assert_eq!(converted.content, "Hello");
    }

    #[test]
    fn test_format_relative_time() {
        use chrono::{Duration, Utc};

        // Just now
        let now = Utc::now();
        let time = format_relative_time(&now.to_rfc3339());
        assert_eq!(time, "just now");

        // 5 minutes ago
        let past = (now - Duration::minutes(5)).to_rfc3339();
        let time = format_relative_time(&past);
        assert_eq!(time, "5 mins ago");

        // 1 hour ago
        let past = (now - Duration::hours(1)).to_rfc3339();
        let time = format_relative_time(&past);
        assert_eq!(time, "1 hour ago");

        // Yesterday
        let past = (now - Duration::days(1)).to_rfc3339();
        let time = format_relative_time(&past);
        assert_eq!(time, "yesterday");

        // 5 days ago
        let past = (now - Duration::days(5)).to_rfc3339();
        let time = format_relative_time(&past);
        assert_eq!(time, "5 days ago");
    }

    #[test]
    fn test_sessions_dir() {
        let dir = sessions_dir();
        assert!(dir.ends_with("sessions"));
    }
}
