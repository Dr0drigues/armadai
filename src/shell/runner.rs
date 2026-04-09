//! Shell runner - executes CLI commands and manages conversation state
//!
//! This module provides the core engine for the shell interactive mode:
//! - Conversation history management
//! - CLI execution via tokio::process::Command
//! - Token and cost tracking
//! - Prompt building with context

use std::time::{Duration, Instant};
use tokio::process::Command;

use super::parser::{ParsedResponse, parse_response};

/// Cost constants for Gemini Flash (per 1M tokens)
const COST_PER_1M_INPUT: f64 = 0.15;
const COST_PER_1M_OUTPUT: f64 = 0.60;

/// Metrics for a single turn.
#[derive(Debug, Clone)]
pub struct TurnMetrics {
    pub tokens_in_estimate: usize,
    pub tokens_out_estimate: usize,
    pub duration: Duration,
    pub turn_number: u32,
}

/// A single message in the conversation.
#[derive(Debug, Clone)]
pub struct Message {
    pub role: MessageRole,
    pub content: String,
    pub metrics: Option<TurnMetrics>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MessageRole {
    User,
    Assistant,
    System,
}

/// Configuration for the shell runner.
#[derive(Debug, Clone)]
pub struct RunnerConfig {
    /// CLI command to run (e.g., "gemini")
    pub command: String,
    /// CLI args before the prompt (e.g., ["-p"])
    pub args: Vec<String>,
    /// Max history turns to include (for token economy)
    pub max_history_turns: usize,
    /// Timeout for CLI execution
    pub timeout: Duration,
}

impl Default for RunnerConfig {
    fn default() -> Self {
        Self {
            command: "gemini".to_string(),
            args: vec!["-p".to_string()],
            max_history_turns: 5,
            timeout: Duration::from_secs(120),
        }
    }
}

/// Session-level cumulative metrics
#[derive(Debug, Clone)]
pub struct SessionMetrics {
    pub turn_count: u32,
    pub total_tokens_in: usize,
    pub total_tokens_out: usize,
    pub total_cost_estimate: f64,
}

/// The shell runner manages conversation state and CLI execution.
pub struct ShellRunner {
    config: RunnerConfig,
    history: Vec<Message>,
    turn_count: u32,
    total_tokens_in: usize,
    total_tokens_out: usize,
    total_cost_estimate: f64,
}

impl ShellRunner {
    pub fn new(config: RunnerConfig) -> Self {
        Self {
            config,
            history: Vec::new(),
            turn_count: 0,
            total_tokens_in: 0,
            total_tokens_out: 0,
            total_cost_estimate: 0.0,
        }
    }

    /// Execute a turn: send user message, get response.
    /// Returns the parsed response and metrics.
    pub async fn send(
        &mut self,
        user_input: &str,
    ) -> anyhow::Result<(ParsedResponse, TurnMetrics)> {
        // 1. Add user message to history
        self.history.push(Message {
            role: MessageRole::User,
            content: user_input.to_string(),
            metrics: None,
        });

        // 2. Build the full prompt (history + current message)
        let prompt = self.build_prompt(user_input);

        // 3. Call the CLI with tokio::process::Command
        let start = Instant::now();

        let output = tokio::time::timeout(
            self.config.timeout,
            Command::new(&self.config.command)
                .args(&self.config.args)
                .arg(&prompt)
                .output(),
        )
        .await??;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("CLI command failed ({}): {stderr}", output.status);
        }

        let raw = String::from_utf8_lossy(&output.stdout).to_string();
        let duration = start.elapsed();

        // 4. Parse the response with parse_response()
        let parsed = parse_response(&raw);

        // 5. Calculate metrics
        let tokens_in = Self::estimate_tokens(&prompt);
        let tokens_out = Self::estimate_tokens(&parsed.content);

        self.turn_count += 1;
        self.total_tokens_in += tokens_in;
        self.total_tokens_out += tokens_out;

        // Calculate cost estimate (Gemini Flash pricing)
        let turn_cost = (tokens_in as f64 / 1_000_000.0) * COST_PER_1M_INPUT
            + (tokens_out as f64 / 1_000_000.0) * COST_PER_1M_OUTPUT;
        self.total_cost_estimate += turn_cost;

        let metrics = TurnMetrics {
            tokens_in_estimate: tokens_in,
            tokens_out_estimate: tokens_out,
            duration,
            turn_number: self.turn_count,
        };

        // 6. Add assistant message to history
        self.history.push(Message {
            role: MessageRole::Assistant,
            content: parsed.content.clone(),
            metrics: Some(metrics.clone()),
        });

        // 7. Return
        Ok((parsed, metrics))
    }

    /// Build the prompt string including history context.
    /// Note: This assumes the current user message has already been added to history.
    fn build_prompt(&self, _current_input: &str) -> String {
        let mut prompt = String::new();

        // Calculate how many messages to include
        // Each turn = 2 messages (user + assistant)
        let max_messages = self.config.max_history_turns * 2;
        let start = if self.history.len() > max_messages {
            self.history.len() - max_messages
        } else {
            0
        };

        // Format all messages (including the most recent user message)
        for msg in &self.history[start..] {
            match msg.role {
                MessageRole::User => {
                    prompt.push_str(&format!("[User]: {}\n", msg.content));
                }
                MessageRole::Assistant => {
                    prompt.push_str(&format!("[Assistant]: {}\n", msg.content));
                }
                MessageRole::System => {
                    // System messages are not included in the prompt
                }
            }
        }

        prompt
    }

    /// Get the CLI command name.
    pub fn command(&self) -> &str {
        &self.config.command
    }

    /// Get the CLI args.
    pub fn args(&self) -> &[String] {
        &self.config.args
    }

    /// Build the prompt for a given input (public for app.rs integration).
    pub fn build_prompt_for(&mut self, user_input: &str) -> String {
        self.history.push(Message {
            role: MessageRole::User,
            content: user_input.to_string(),
            metrics: None,
        });
        self.build_prompt(user_input)
    }

    /// Record a completed turn in history (called by app.rs after CLI returns).
    pub fn record_turn(&mut self, _user_input: &str, assistant_content: &str, duration: Duration) {
        let tokens_in = Self::estimate_tokens(
            self.history
                .last()
                .map(|m| m.content.as_str())
                .unwrap_or(""),
        );
        let tokens_out = Self::estimate_tokens(assistant_content);

        self.turn_count += 1;
        self.total_tokens_in += tokens_in;
        self.total_tokens_out += tokens_out;

        let turn_cost = (tokens_in as f64 / 1_000_000.0) * COST_PER_1M_INPUT
            + (tokens_out as f64 / 1_000_000.0) * COST_PER_1M_OUTPUT;
        self.total_cost_estimate += turn_cost;

        self.history.push(Message {
            role: MessageRole::Assistant,
            content: assistant_content.to_string(),
            metrics: Some(TurnMetrics {
                tokens_in_estimate: tokens_in,
                tokens_out_estimate: tokens_out,
                duration,
                turn_number: self.turn_count,
            }),
        });
    }

    /// Estimate token count from text (rough: chars / 4)
    pub fn estimate_tokens(text: &str) -> usize {
        text.len() / 4
    }

    /// Get cumulative session metrics
    pub fn session_metrics(&self) -> SessionMetrics {
        SessionMetrics {
            turn_count: self.turn_count,
            total_tokens_in: self.total_tokens_in,
            total_tokens_out: self.total_tokens_out,
            total_cost_estimate: self.total_cost_estimate,
        }
    }

    /// Get conversation history
    pub fn history(&self) -> &[Message] {
        &self.history
    }

    /// Clear history
    pub fn clear(&mut self) {
        self.history.clear();
        self.turn_count = 0;
        self.total_tokens_in = 0;
        self.total_tokens_out = 0;
        self.total_cost_estimate = 0.0;
    }

    /// Switch to a different provider while keeping conversation history.
    ///
    /// This allows changing the CLI tool mid-session while preserving context.
    /// The history is kept because it can still be useful for the new provider.
    pub fn switch_provider(&mut self, command: String, args: Vec<String>) {
        self.config.command = command;
        self.config.args = args;
        // Keep history — it's still useful as context
    }

    /// Restore session state from a saved session.
    pub fn restore_from_session(&mut self, messages: Vec<Message>) {
        self.clear();
        self.history = messages;
        // Recalculate metrics from history
        for msg in &self.history {
            if let MessageRole::Assistant = msg.role
                && let Some(ref metrics) = msg.metrics
            {
                self.total_tokens_in += metrics.tokens_in_estimate;
                self.total_tokens_out += metrics.tokens_out_estimate;
                self.turn_count = metrics.turn_number;
                // Recalculate cost
                let turn_cost = (metrics.tokens_in_estimate as f64 / 1_000_000.0)
                    * COST_PER_1M_INPUT
                    + (metrics.tokens_out_estimate as f64 / 1_000_000.0) * COST_PER_1M_OUTPUT;
                self.total_cost_estimate += turn_cost;
            }
        }
    }

    /// Get the current provider command
    pub fn provider_command(&self) -> &str {
        &self.config.command
    }

    /// Get turn count
    pub fn turn_count(&self) -> u32 {
        self.turn_count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_prompt_no_history() {
        let config = RunnerConfig::default();
        let mut runner = ShellRunner::new(config);

        // Simulate adding user message (as send() does)
        runner.history.push(Message {
            role: MessageRole::User,
            content: "Hello".to_string(),
            metrics: None,
        });

        let prompt = runner.build_prompt("Hello");
        assert_eq!(prompt, "[User]: Hello\n");
    }

    #[test]
    fn test_build_prompt_with_history() {
        let config = RunnerConfig::default();
        let mut runner = ShellRunner::new(config);

        // Add some history
        runner.history.push(Message {
            role: MessageRole::User,
            content: "First question".to_string(),
            metrics: None,
        });
        runner.history.push(Message {
            role: MessageRole::Assistant,
            content: "First answer".to_string(),
            metrics: None,
        });
        // Add current user message (as send() does)
        runner.history.push(Message {
            role: MessageRole::User,
            content: "Second question".to_string(),
            metrics: None,
        });

        let prompt = runner.build_prompt("Second question");

        assert!(prompt.contains("[User]: First question"));
        assert!(prompt.contains("[Assistant]: First answer"));
        assert!(prompt.contains("[User]: Second question"));
    }

    #[test]
    fn test_build_prompt_truncates_history() {
        let config = RunnerConfig {
            max_history_turns: 2,
            ..Default::default()
        };
        let mut runner = ShellRunner::new(config);

        // Add 5 turns (10 messages)
        for i in 1..=5 {
            runner.history.push(Message {
                role: MessageRole::User,
                content: format!("Question {i}"),
                metrics: None,
            });
            runner.history.push(Message {
                role: MessageRole::Assistant,
                content: format!("Answer {i}"),
                metrics: None,
            });
        }

        // Add current user message (as send() does)
        runner.history.push(Message {
            role: MessageRole::User,
            content: "New question".to_string(),
            metrics: None,
        });

        let prompt = runner.build_prompt("New question");

        // Should only include last 2 turns (4 messages)
        // Since we added a new user message, we have 11 messages total
        // max_history_turns = 2 means 4 messages max
        // So we should see only the last 4 messages
        assert!(!prompt.contains("Question 1"));
        assert!(!prompt.contains("Question 2"));
        assert!(!prompt.contains("Question 3"));
        assert!(!prompt.contains("Question 4"));
        assert!(prompt.contains("Question 5"));
        assert!(prompt.contains("New question"));
    }

    #[test]
    fn test_estimate_tokens() {
        // Rough estimate: 4 chars = 1 token
        assert_eq!(ShellRunner::estimate_tokens("test"), 1);
        assert_eq!(ShellRunner::estimate_tokens("hello world"), 2);
        assert_eq!(ShellRunner::estimate_tokens("a".repeat(100).as_str()), 25);
    }

    #[test]
    fn test_session_metrics_initial() {
        let config = RunnerConfig::default();
        let runner = ShellRunner::new(config);

        let metrics = runner.session_metrics();
        assert_eq!(metrics.turn_count, 0);
        assert_eq!(metrics.total_tokens_in, 0);
        assert_eq!(metrics.total_tokens_out, 0);
        assert_eq!(metrics.total_cost_estimate, 0.0);
    }

    #[test]
    fn test_clear_history() {
        let config = RunnerConfig::default();
        let mut runner = ShellRunner::new(config);

        // Add some data
        runner.history.push(Message {
            role: MessageRole::User,
            content: "Test".to_string(),
            metrics: None,
        });
        runner.turn_count = 5;
        runner.total_tokens_in = 100;
        runner.total_tokens_out = 200;
        runner.total_cost_estimate = 0.5;

        // Clear
        runner.clear();

        // Verify everything is reset
        assert_eq!(runner.history.len(), 0);
        assert_eq!(runner.turn_count, 0);
        assert_eq!(runner.total_tokens_in, 0);
        assert_eq!(runner.total_tokens_out, 0);
        assert_eq!(runner.total_cost_estimate, 0.0);
    }

    #[test]
    fn test_message_role_equality() {
        assert_eq!(MessageRole::User, MessageRole::User);
        assert_eq!(MessageRole::Assistant, MessageRole::Assistant);
        assert_ne!(MessageRole::User, MessageRole::Assistant);
    }
}
