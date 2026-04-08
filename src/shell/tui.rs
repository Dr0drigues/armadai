//! TUI for the ArmadAI interactive shell.
//!
//! Provides a conversational interface with:
//! - Message area showing user and assistant exchanges
//! - Input box at the bottom
//! - Status bar with provider info and metrics

#![cfg(feature = "tui")]

use crossterm::event::KeyEvent;
use ratatui::{
    prelude::*,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};
use std::time::Duration;

/// A single message in the conversation
#[derive(Debug, Clone)]
pub struct DisplayMessage {
    pub role: String, // "You" or agent name
    pub content: String,
    pub is_user: bool,
}

/// Application state for the shell TUI
pub struct ShellApp {
    /// Conversation messages for display
    messages: Vec<DisplayMessage>,
    /// Current user input
    input: String,
    /// Cursor position in input
    cursor: usize,
    /// Scroll offset for messages area
    scroll: u16,
    /// Whether we're waiting for a response
    loading: bool,
    /// Provider name for statusbar
    provider_name: String,
    /// Session metrics for statusbar
    turn_count: u32,
    tokens_in: usize,
    tokens_out: usize,
    cost: f64,
    last_duration: Duration,
    /// Should quit
    should_quit: bool,
}

impl ShellApp {
    /// Create a new shell app
    pub fn new(provider_name: String) -> Self {
        Self {
            messages: Vec::new(),
            input: String::new(),
            cursor: 0,
            scroll: 0,
            loading: false,
            provider_name,
            turn_count: 0,
            tokens_in: 0,
            tokens_out: 0,
            cost: 0.0,
            last_duration: Duration::from_secs(0),
            should_quit: false,
        }
    }

    /// Add a user message to the display
    pub fn add_user_message(&mut self, content: &str) {
        self.messages.push(DisplayMessage {
            role: "You".to_string(),
            content: content.to_string(),
            is_user: true,
        });
        self.scroll_to_bottom();
    }

    /// Add an assistant response to the display
    pub fn add_assistant_message(&mut self, content: &str) {
        self.messages.push(DisplayMessage {
            role: self.provider_name.clone(),
            content: content.to_string(),
            is_user: false,
        });
        self.scroll_to_bottom();
    }

    /// Update metrics after a turn
    pub fn update_metrics(
        &mut self,
        tokens_in: usize,
        tokens_out: usize,
        cost: f64,
        duration: Duration,
    ) {
        self.tokens_in += tokens_in;
        self.tokens_out += tokens_out;
        self.cost += cost;
        self.last_duration = duration;
        self.turn_count += 1;
    }

    /// Take the current input (returns it and clears the input box)
    pub fn take_input(&mut self) -> Option<String> {
        if self.input.is_empty() {
            return None;
        }
        let result = self.input.clone();
        self.input.clear();
        self.cursor = 0;
        Some(result)
    }

    /// Set loading state
    pub fn set_loading(&mut self, loading: bool) {
        self.loading = loading;
    }

    /// Clear the conversation
    pub fn clear_conversation(&mut self) {
        self.messages.clear();
        self.scroll = 0;
    }

    /// Scroll messages area
    fn scroll_to_bottom(&mut self) {
        // Will be calculated based on content height in render
    }

    fn scroll_up(&mut self) {
        if self.scroll > 0 {
            self.scroll = self.scroll.saturating_sub(5);
        }
    }

    fn scroll_down(&mut self) {
        self.scroll = self.scroll.saturating_add(5);
    }

    /// Handle a key event, returns true if should quit
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        use crossterm::event::{KeyCode, KeyModifiers};

        match key.code {
            // Handle Ctrl+C and Esc for quit
            KeyCode::Esc => {
                self.should_quit = true;
                true
            }
            KeyCode::Char('c') if key.modifiers == KeyModifiers::CONTROL => {
                self.should_quit = true;
                true
            }
            // Handle Ctrl+L for clear
            KeyCode::Char('l') if key.modifiers == KeyModifiers::CONTROL => {
                self.clear_conversation();
                false
            }
            // Regular character input
            KeyCode::Char(c) => {
                self.input.insert(self.cursor, c);
                self.cursor += 1;
                false
            }
            KeyCode::Backspace => {
                if self.cursor > 0 {
                    self.input.remove(self.cursor - 1);
                    self.cursor -= 1;
                }
                false
            }
            KeyCode::Delete => {
                if self.cursor < self.input.len() {
                    self.input.remove(self.cursor);
                }
                false
            }
            KeyCode::Left => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                }
                false
            }
            KeyCode::Right => {
                if self.cursor < self.input.len() {
                    self.cursor += 1;
                }
                false
            }
            KeyCode::Home => {
                self.cursor = 0;
                false
            }
            KeyCode::End => {
                self.cursor = self.input.len();
                false
            }
            KeyCode::PageUp => {
                self.scroll_up();
                false
            }
            KeyCode::PageDown => {
                self.scroll_down();
                false
            }
            KeyCode::Enter => {
                // Submit will be handled by take_input
                false
            }
            _ => false,
        }
    }

    pub fn should_quit(&self) -> bool {
        self.should_quit
    }

    /// Render the shell TUI
    pub fn render(&self, frame: &mut Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // Header
                Constraint::Min(0),    // Messages area
                Constraint::Length(1), // Statusbar
                Constraint::Length(1), // Input line
            ])
            .split(frame.area());

        // Header
        let header_text = format!(
            "ArmadAI Shell — provider: {} — Turn #{}",
            self.provider_name, self.turn_count
        );
        let header = Paragraph::new(header_text).style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );
        frame.render_widget(header, chunks[0]);

        // Messages area
        self.render_messages_area(frame, chunks[1]);

        // Status bar
        self.render_statusbar(frame, chunks[2]);

        // Input line
        self.render_input_line(frame, chunks[3]);
    }

    fn render_messages_area(&self, frame: &mut Frame, area: Rect) {
        if self.messages.is_empty() {
            let placeholder = Paragraph::new("Welcome to ArmadAI Shell!\n\nType your message and press Enter to get started. Press Ctrl+L to clear conversation, Ctrl+C or Esc to quit.")
                .block(Block::default().borders(Borders::ALL))
                .wrap(Wrap { trim: true });
            frame.render_widget(placeholder, area);
            return;
        }

        // Format messages for display
        let mut lines: Vec<Line> = Vec::new();

        for msg in &self.messages {
            // Add role label
            let role_style = if msg.is_user {
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            };

            lines.push(Line::from(vec![Span::styled(
                format!("{}: ", msg.role),
                role_style,
            )]));

            // Add content lines with wrapping
            for line in msg.content.lines() {
                lines.push(Line::from(line.to_string()));
            }

            // Add blank line between messages
            lines.push(Line::from(""));
        }

        // Create paragraph with message content
        let messages_text = Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL))
            .wrap(Wrap { trim: true });

        frame.render_widget(messages_text, area);
    }

    fn render_statusbar(&self, frame: &mut Frame, area: Rect) {
        let status_text = if self.loading {
            format!(
                "{} │ {} in │ {} out │ ${:.3} │ {:.1}s │ thinking...",
                self.provider_name,
                self.tokens_in,
                self.tokens_out,
                self.cost,
                self.last_duration.as_secs_f64()
            )
        } else {
            format!(
                "{} │ {} in │ {} out │ ${:.3} │ {:.1}s │ #{}",
                self.provider_name,
                self.tokens_in,
                self.tokens_out,
                self.cost,
                self.last_duration.as_secs_f64(),
                self.turn_count
            )
        };

        let statusbar = Paragraph::new(status_text)
            .style(Style::default().fg(Color::DarkGray))
            .block(Block::default().borders(Borders::ALL));

        frame.render_widget(statusbar, area);
    }

    fn render_input_line(&self, frame: &mut Frame, area: Rect) {
        let cursor_indicator = if self.loading { "..." } else { ">" };

        // Build the input display with cursor
        let mut input_spans = vec![Span::raw(format!("{} ", cursor_indicator))];

        for (i, c) in self.input.chars().enumerate() {
            if i == self.cursor {
                input_spans.push(Span::styled(
                    c.to_string(),
                    Style::default().bg(Color::White).fg(Color::Black),
                ));
            } else {
                input_spans.push(Span::raw(c.to_string()));
            }
        }

        // If cursor is at end, show cursor
        if self.cursor >= self.input.len() && !self.loading {
            input_spans.push(Span::styled(
                " ",
                Style::default().bg(Color::White).fg(Color::Black),
            ));
        }

        let input_line = Paragraph::new(Line::from(input_spans)).block(
            Block::default()
                .borders(Borders::TOP | Borders::LEFT | Borders::RIGHT)
                .style(Style::default()),
        );

        frame.render_widget(input_line, area);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_shell_app() {
        let app = ShellApp::new("Gemini".to_string());
        assert_eq!(app.provider_name, "Gemini");
        assert!(app.messages.is_empty());
        assert!(!app.should_quit);
    }

    #[test]
    fn test_add_messages() {
        let mut app = ShellApp::new("Gemini".to_string());
        app.add_user_message("Hello");
        app.add_assistant_message("Hi there!");

        assert_eq!(app.messages.len(), 2);
        assert!(app.messages[0].is_user);
        assert!(!app.messages[1].is_user);
    }

    #[test]
    fn test_take_input() {
        let mut app = ShellApp::new("Gemini".to_string());
        app.input = "test".to_string();
        app.cursor = 4;

        let result = app.take_input();
        assert_eq!(result, Some("test".to_string()));
        assert!(app.input.is_empty());
        assert_eq!(app.cursor, 0);
    }

    #[test]
    fn test_update_metrics() {
        let mut app = ShellApp::new("Gemini".to_string());
        app.update_metrics(100, 50, 0.001, Duration::from_secs(1));

        assert_eq!(app.tokens_in, 100);
        assert_eq!(app.tokens_out, 50);
        assert_eq!(app.turn_count, 1);
    }
}
