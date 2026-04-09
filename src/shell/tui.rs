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
use std::time::{Duration, Instant};
use tui_markdown::StyleSheet;

/// Custom stylesheet for ArmadAI shell — designed for dark terminal themes.
#[derive(Clone, Copy, Debug, Default)]
struct ArmadaiStyleSheet;

impl tui_markdown::StyleSheet for ArmadaiStyleSheet {
    fn heading(&self, level: u8) -> Style {
        match level {
            1 => Style::new().fg(Color::Rgb(88, 166, 255)).bold().underlined(),
            2 => Style::new().fg(Color::Rgb(63, 185, 80)).bold(),
            3 => Style::new().fg(Color::Rgb(210, 153, 34)).bold(),
            _ => Style::new().fg(Color::Rgb(139, 148, 158)).italic(),
        }
    }

    fn code(&self) -> Style {
        Style::new().fg(Color::Rgb(230, 237, 243)).bg(Color::Rgb(55, 62, 71))
    }

    fn link(&self) -> Style {
        Style::new().fg(Color::Rgb(88, 166, 255)).underlined()
    }

    fn blockquote(&self) -> Style {
        Style::new().fg(Color::Rgb(139, 148, 158)).italic()
    }

    fn heading_meta(&self) -> Style {
        Style::new().dim()
    }

    fn metadata_block(&self) -> Style {
        Style::new().fg(Color::Rgb(210, 153, 34))
    }
}

const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

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
    /// Spinner frame index
    spinner_frame: usize,
    /// When loading started
    loading_start: Option<Instant>,
    /// Input history (previous prompts)
    input_history: Vec<String>,
    /// Current position in input history (None = not browsing)
    history_index: Option<usize>,
    /// Saved current input when browsing history
    saved_input: String,
    /// Provider name for statusbar
    provider_name: String,
    /// Model name for header
    model_name: String,
    /// Session metrics for statusbar
    turn_count: u32,
    tokens_in: usize,
    tokens_out: usize,
    cost: f64,
    last_duration: Duration,
    /// Whether user has manually scrolled (disables auto-scroll to bottom)
    manual_scroll: bool,
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
            spinner_frame: 0,
            loading_start: None,
            input_history: Vec::new(),
            history_index: None,
            saved_input: String::new(),
            manual_scroll: false,
            provider_name,
            model_name: String::new(),
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
        // Reset to auto-scroll on new content
        self.manual_scroll = false;
        self.scroll = 0;
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
        // Save to history
        self.input_history.push(result.clone());
        self.history_index = None;
        self.saved_input.clear();
        self.input.clear();
        self.cursor = 0;
        Some(result)
    }

    /// Set model name for display
    pub fn set_model_name(&mut self, name: String) {
        self.model_name = name;
    }

    /// Set session metrics from the runner (replaces update_metrics for cumulative data)
    pub fn set_session_metrics(
        &mut self,
        tokens_in: usize,
        tokens_out: usize,
        cost: f64,
        turn_count: u32,
        last_duration: Duration,
    ) {
        self.tokens_in = tokens_in;
        self.tokens_out = tokens_out;
        self.cost = cost;
        self.turn_count = turn_count;
        self.last_duration = last_duration;
    }

    /// Set loading state
    pub fn set_loading(&mut self, loading: bool) {
        self.loading = loading;
        if loading {
            self.loading_start = Some(Instant::now());
            self.spinner_frame = 0;
        } else {
            self.loading_start = None;
        }
    }

    /// Advance the spinner animation (call on each render tick during loading)
    pub fn tick_spinner(&mut self) {
        if self.loading {
            self.spinner_frame = (self.spinner_frame + 1) % SPINNER_FRAMES.len();
        }
    }

    /// Clear the conversation
    pub fn clear_conversation(&mut self) {
        self.messages.clear();
        self.scroll = 0;
    }

    /// Convert a char-based cursor position to a byte index in the input string.
    fn char_to_byte(&self, char_pos: usize) -> usize {
        self.input
            .char_indices()
            .nth(char_pos)
            .map(|(i, _)| i)
            .unwrap_or(self.input.len())
    }

    /// Scroll messages area
    fn scroll_to_bottom(&mut self) {
        // Will be calculated based on content height in render
    }

    fn scroll_up(&mut self) {
        self.scroll = self.scroll.saturating_sub(2);
        self.manual_scroll = true;
    }

    fn scroll_down(&mut self) {
        self.scroll = self.scroll.saturating_add(2);
        // Mark as manually scrolled so auto-scroll doesn't override
        self.manual_scroll = true;
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
                let byte_idx = self.char_to_byte(self.cursor);
                self.input.insert(byte_idx, c);
                self.cursor += 1;
                false
            }
            KeyCode::Backspace => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                    let byte_idx = self.char_to_byte(self.cursor);
                    self.input.remove(byte_idx);
                }
                false
            }
            KeyCode::Delete => {
                if self.cursor < self.input.chars().count() {
                    let byte_idx = self.char_to_byte(self.cursor);
                    self.input.remove(byte_idx);
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
                if self.cursor < self.input.chars().count() {
                    self.cursor += 1;
                }
                false
            }
            KeyCode::Home => {
                self.cursor = 0;
                false
            }
            KeyCode::End => {
                self.cursor = self.input.chars().count();
                false
            }
            KeyCode::Up => {
                // Navigate input history (older)
                if !self.input_history.is_empty() {
                    match self.history_index {
                        None => {
                            // Start browsing: save current input, show last history item
                            self.saved_input = self.input.clone();
                            let idx = self.input_history.len() - 1;
                            self.history_index = Some(idx);
                            self.input = self.input_history[idx].clone();
                            self.cursor = self.input.chars().count();
                        }
                        Some(idx) if idx > 0 => {
                            let new_idx = idx - 1;
                            self.history_index = Some(new_idx);
                            self.input = self.input_history[new_idx].clone();
                            self.cursor = self.input.chars().count();
                        }
                        _ => {} // At oldest item, do nothing
                    }
                }
                false
            }
            KeyCode::Down => {
                // Navigate input history (newer)
                if let Some(idx) = self.history_index {
                    if idx + 1 < self.input_history.len() {
                        let new_idx = idx + 1;
                        self.history_index = Some(new_idx);
                        self.input = self.input_history[new_idx].clone();
                        self.cursor = self.input.chars().count();
                    } else {
                        // Back to current input
                        self.history_index = None;
                        self.input = self.saved_input.clone();
                        self.cursor = self.input.chars().count();
                    }
                }
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

    /// Handle mouse events (scroll wheel)
    pub fn handle_mouse(&mut self, mouse: crossterm::event::MouseEvent) {
        use crossterm::event::MouseEventKind;
        match mouse.kind {
            MouseEventKind::ScrollUp => self.scroll_up(),
            MouseEventKind::ScrollDown => self.scroll_down(),
            _ => {}
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
                Constraint::Length(3), // Input line (with borders)
            ])
            .split(frame.area());

        // Header
        let model_info = if self.model_name.is_empty() {
            self.provider_name.clone()
        } else {
            format!("{} ({})", self.provider_name, self.model_name)
        };
        let header_text = format!(
            "ArmadAI Shell — {} — Turn #{}",
            model_info, self.turn_count
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

            if msg.is_user {
                // User messages: plain text
                for line in msg.content.lines() {
                    lines.push(Line::from(line.to_string()));
                }
            } else {
                // Assistant messages: rich markdown rendering
                let opts = tui_markdown::Options::new(ArmadaiStyleSheet);
                let md_text = tui_markdown::from_str_with_options(&msg.content, &opts);
                for line in md_text.lines {
                    // Post-process: strip leading ### markers from headings,
                    // replace with clean styled text
                    let first_span_str: String = line
                        .spans
                        .first()
                        .map(|s| s.content.to_string())
                        .unwrap_or_default();
                    if first_span_str.starts_with('#') {
                        // It's a heading line — strip the # prefix, keep the original line style
                        let line_style = line.style;
                        lines.push(Line::from(""));

                        // Determine heading level for separator
                        let hash_count = first_span_str.chars().take_while(|c| *c == '#').count();

                        // Build the cleaned heading text
                        let mut heading_text = String::new();
                        for s in &line.spans {
                            let content = s.content.to_string();
                            if content.starts_with('#') {
                                heading_text.push_str(content.trim_start_matches('#').trim_start());
                            } else {
                                heading_text.push_str(&content);
                            }
                        }

                        // Apply heading style from our stylesheet
                        let heading_style = ArmadaiStyleSheet.heading(hash_count as u8)
                            .patch(line_style);
                        lines.push(Line::from(Span::styled(heading_text, heading_style)));

                        // Add separator after H1/H2/H3
                        if hash_count <= 3 {
                            lines.push(Line::from(Span::styled(
                                "─".repeat(50),
                                Style::default().fg(Color::DarkGray),
                            )));
                        }
                    } else {
                        lines.push(line);
                    }
                }
            }

            // Add blank line between messages
            lines.push(Line::from(""));
        }

        // Add loading indicator as last message
        if self.loading {
            let spinner = SPINNER_FRAMES[self.spinner_frame];
            let elapsed = self.loading_start.map(|s| s.elapsed().as_secs_f64()).unwrap_or(0.0);
            lines.push(Line::from(vec![Span::styled(
                format!("{spinner} Generating response… {elapsed:.0}s"),
                Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC),
            )]));
        }

        // Calculate scroll position
        let visible_height = area.height.saturating_sub(2) as usize; // minus borders
        let total_lines = lines.len();
        let max_scroll = if total_lines > visible_height {
            (total_lines - visible_height) as u16
        } else {
            0
        };
        let scroll = if self.manual_scroll {
            // User is manually scrolling — clamp to valid range
            self.scroll.min(max_scroll)
        } else {
            // Auto-scroll to bottom
            max_scroll
        };

        // Create paragraph with message content
        let messages_text = Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL))
            .wrap(Wrap { trim: true })
            .scroll((scroll, 0));

        frame.render_widget(messages_text, area);
    }

    fn render_statusbar(&self, frame: &mut Frame, area: Rect) {
        let status_text = if self.loading {
            let elapsed = self.loading_start.map(|s| s.elapsed().as_secs_f64()).unwrap_or(0.0);
            let spinner = SPINNER_FRAMES[self.spinner_frame];
            format!(
                "{} │ {} in │ {} out │ ${:.3} │ {spinner} thinking… {:.0}s",
                self.provider_name,
                self.tokens_in,
                self.tokens_out,
                self.cost,
                elapsed,
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
            .style(Style::default().fg(Color::DarkGray).bg(Color::Rgb(22, 27, 34)));

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
        if self.cursor >= self.input.chars().count() && !self.loading {
            input_spans.push(Span::styled(
                " ",
                Style::default().bg(Color::White).fg(Color::Black),
            ));
        }

        let input_line = Paragraph::new(Line::from(input_spans)).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray))
                .title(" Input ")
                .title_style(Style::default().fg(Color::Cyan)),
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
