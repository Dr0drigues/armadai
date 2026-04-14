//! PTY-based runner for interactive CLI sessions.
//!
//! Spawns CLI tools (Claude, Gemini, etc.) in a pseudo-terminal so they
//! read their context files (CLAUDE.md, agents/, etc.) and can delegate
//! to sub-agents natively.

#![cfg(feature = "tui")]

use anyhow::{Context, Result};
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use std::io::{Read, Write};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Chunk received from the PTY output stream.
#[derive(Debug)]
pub enum PtyChunk {
    /// Raw text output (may contain ANSI codes)
    Text(String),
    /// PTY process has exited
    Done,
}

/// A running PTY session.
pub struct PtySession {
    /// Channel to receive output chunks from the reader thread
    rx: mpsc::Receiver<PtyChunk>,
    /// Writer to send input to the PTY
    writer: Box<dyn Write + Send>,
    /// Child process handle
    child: Box<dyn portable_pty::Child + Send + Sync>,
    /// When the session started
    started_at: Instant,
    /// Buffer of all raw output received
    raw_buffer: String,
}

/// Configuration for the PTY session.
pub struct PtyConfig {
    pub command: String,
    pub width: u16,
    pub height: u16,
}

impl Default for PtyConfig {
    fn default() -> Self {
        Self {
            command: "claude".to_string(),
            width: 120,
            height: 40,
        }
    }
}

impl PtySession {
    /// Spawn a CLI tool in a PTY.
    pub fn spawn(config: &PtyConfig) -> Result<Self> {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: config.height,
                cols: config.width,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("Failed to open PTY")?;

        let cmd = CommandBuilder::new(&config.command);
        let child = pair
            .slave
            .spawn_command(cmd)
            .context(format!("Failed to spawn '{}' in PTY", config.command))?;

        let writer = pair
            .master
            .take_writer()
            .context("Failed to take PTY writer")?;

        let mut reader = pair
            .master
            .try_clone_reader()
            .context("Failed to clone PTY reader")?;

        // Spawn reader thread (PTY reads are blocking)
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => {
                        let _ = tx.send(PtyChunk::Done);
                        break;
                    }
                    Ok(n) => {
                        let raw = &buf[..n];
                        let cleaned = strip_ansi_escapes::strip(raw);
                        let text = String::from_utf8_lossy(&cleaned).to_string();
                        if !text.is_empty() {
                            let _ = tx.send(PtyChunk::Text(text));
                        }
                    }
                    Err(_) => {
                        let _ = tx.send(PtyChunk::Done);
                        break;
                    }
                }
            }
        });

        Ok(Self {
            rx,
            writer,
            child,
            started_at: Instant::now(),
            raw_buffer: String::new(),
        })
    }

    /// Send a message to the PTY (simulates user typing + Enter).
    pub fn send(&mut self, message: &str) -> Result<()> {
        self.writer
            .write_all(message.as_bytes())
            .context("Failed to write to PTY")?;
        self.writer
            .write_all(b"\n")
            .context("Failed to write newline to PTY")?;
        self.writer.flush().context("Failed to flush PTY writer")?;
        Ok(())
    }

    /// Drain all available output chunks (non-blocking).
    /// Returns the new text received since last drain.
    pub fn drain(&mut self) -> (String, bool) {
        let mut new_text = String::new();
        let mut done = false;

        while let Ok(chunk) = self.rx.try_recv() {
            match chunk {
                PtyChunk::Text(text) => {
                    self.raw_buffer.push_str(&text);
                    new_text.push_str(&text);
                }
                PtyChunk::Done => {
                    done = true;
                }
            }
        }

        (new_text, done)
    }

    /// Drain with a silence timeout — wait until no new output for `silence` duration.
    /// Used during startup to consume initial CLI noise.
    pub fn drain_until_silence(&mut self, silence: Duration) -> String {
        let mut text = String::new();
        loop {
            match self.rx.recv_timeout(silence) {
                Ok(PtyChunk::Text(t)) => {
                    self.raw_buffer.push_str(&t);
                    text.push_str(&t);
                }
                Ok(PtyChunk::Done) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => break,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        text
    }

    /// Get elapsed time since session start.
    pub fn elapsed(&self) -> Duration {
        self.started_at.elapsed()
    }

    /// Kill the PTY process.
    pub fn kill(&mut self) {
        let _ = self.child.kill();
    }

    /// Check if the child process is still running.
    pub fn is_running(&mut self) -> bool {
        self.child.try_wait().ok().flatten().is_none()
    }
}

impl Drop for PtySession {
    fn drop(&mut self) {
        self.kill();
    }
}

/// Filter out common CLI startup noise (auth prompts, version banners, etc.)
pub fn filter_startup_noise(text: &str) -> String {
    let mut result = String::new();
    for line in text.lines() {
        let trimmed = line.trim();
        // Skip common noise patterns
        if trimmed.is_empty()
            || trimmed.contains("Keychain initialization")
            || trimmed.contains("Using FileKeychain")
            || trimmed.contains("Loaded cached credentials")
            || trimmed.contains("Require stack")
            || trimmed.contains("node_modules")
            || trimmed.contains("update available")
            || trimmed.contains("Homebrew")
            || trimmed.contains("Warning you are running")
            || trimmed.contains("disabled in /settings")
            || trimmed.starts_with('╭')
            || trimmed.starts_with('│')
            || trimmed.starts_with('╰')
            || trimmed.starts_with('┌')
            || trimmed.starts_with('└')
            || trimmed.starts_with('├')
            || trimmed.contains("Waiting for authentication")
            || trimmed.contains("Do you trust the files")
        {
            continue;
        }
        result.push_str(line);
        result.push('\n');
    }
    result
}

/// Detect if a line contains a delegation marker (even without our custom protocol).
/// CLIs in interactive mode may show agent switching differently.
pub fn detect_agent_activity(text: &str) -> Vec<String> {
    let mut agents = Vec::new();

    // Our protocol markers
    for line in text.lines() {
        if let Some(start) = line.find("<!--ARMADAI_DELEGATE:")
            && let Some(end) = line[start..].find("-->")
        {
            let agent = &line[start + 21..start + end];
            agents.push(agent.trim().to_string());
        }
    }

    // Claude Code agent switching pattern: "Using agent: agent-name"
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("Using agent:") {
            agents.push(rest.trim().to_string());
        }
        // Also detect "@agent-name" references in coordinator responses
        if line.contains("delegating to") || line.contains("Delegating to") {
            // Try to extract agent name after "to @" or "to "
            if let Some(pos) = line.find("to @") {
                let name: String = line[pos + 4..]
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
                    .collect();
                if !name.is_empty() {
                    agents.push(name);
                }
            }
        }
    }

    agents
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_startup_noise() {
        let input = "Keychain initialization error\nLoaded cached credentials\nActual response here\n";
        let filtered = filter_startup_noise(input);
        assert!(filtered.contains("Actual response here"));
        assert!(!filtered.contains("Keychain"));
    }

    #[test]
    fn test_filter_box_drawing() {
        let input = "╭───────╮\n│ Trust │\n╰───────╯\nReal content\n";
        let filtered = filter_startup_noise(input);
        assert!(filtered.contains("Real content"));
        assert!(!filtered.contains("Trust"));
    }

    #[test]
    fn test_detect_agent_activity_markers() {
        let text = "Some text\n<!--ARMADAI_DELEGATE:shell-expert-->\nMore text";
        let agents = detect_agent_activity(text);
        assert_eq!(agents, vec!["shell-expert"]);
    }

    #[test]
    fn test_detect_agent_activity_delegation_text() {
        let text = "I'll be delegating to @container-expert for this task.";
        let agents = detect_agent_activity(text);
        assert_eq!(agents, vec!["container-expert"]);
    }

    #[test]
    fn test_detect_agent_activity_none() {
        let text = "Just a regular response with no delegation.";
        let agents = detect_agent_activity(text);
        assert!(agents.is_empty());
    }
}
