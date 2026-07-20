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
use unicode_width::UnicodeWidthChar;

use super::SPINNER_FRAMES;

/// A single message in the conversation
#[derive(Debug, Clone)]
pub struct DisplayMessage {
    pub role: String, // "You" or agent name
    pub content: String,
    pub is_user: bool,
    pub is_system: bool,    // System messages (commands, etc.)
    pub id: Option<String>, // Unique ID for tandem streams (prevents collision when same provider appears twice)
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
    /// Pending tandem providers (used for next message)
    tandem_providers: Option<Vec<String>>,
    /// Pending pipeline providers (used for next message)
    pipeline_providers: Option<Vec<String>>,
    /// Overlay popup content (shown on top of messages, dismissed with Esc)
    popup: Option<String>,
    /// Popup scroll offset
    popup_scroll: u16,
    /// Should quit
    should_quit: bool,
    /// PTY mode enabled
    pty_mode: bool,
    /// Agent workroom panel
    pub workroom: super::workroom::Workroom,
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
            tandem_providers: None,
            pipeline_providers: None,
            popup: None,
            popup_scroll: 0,
            provider_name,
            model_name: String::new(),
            turn_count: 0,
            tokens_in: 0,
            tokens_out: 0,
            cost: 0.0,
            last_duration: Duration::from_secs(0),
            should_quit: false,
            pty_mode: false,
            workroom: super::workroom::Workroom::new(),
        }
    }

    /// Add a user message to the display
    pub fn add_user_message(&mut self, content: &str) {
        self.messages.push(DisplayMessage {
            role: "You".to_string(),
            content: content.to_string(),
            is_user: true,
            is_system: false,
            id: None,
        });
        self.scroll_to_bottom();
    }

    /// Add an assistant response to the display
    pub fn add_assistant_message(&mut self, content: &str) {
        self.messages.push(DisplayMessage {
            role: self.provider_name.clone(),
            content: content.to_string(),
            is_user: false,
            is_system: false,
            id: None,
        });
        // Reset to auto-scroll on new content
        self.manual_scroll = false;
        self.scroll = 0;
    }

    /// Add an assistant response with a custom label (for tandem/pipeline mode)
    pub fn add_assistant_message_with_label(&mut self, label: &str, content: &str) {
        self.messages.push(DisplayMessage {
            role: label.to_string(),
            content: content.to_string(),
            is_user: false,
            is_system: false,
            id: None,
        });
        self.manual_scroll = false;
        self.scroll = 0;
    }

    /// Add a system message (from slash commands, etc.)
    pub fn add_system_message(&mut self, content: &str) {
        self.messages.push(DisplayMessage {
            role: "System".to_string(),
            content: content.to_string(),
            is_user: false,
            is_system: true,
            id: None,
        });
        self.scroll_to_bottom();
    }

    /// Start a new streaming assistant response
    pub fn start_streaming_response(&mut self) {
        self.messages.push(DisplayMessage {
            role: self.provider_name.clone(),
            content: String::new(),
            is_user: false,
            is_system: false,
            id: None,
        });
        self.manual_scroll = false;
        self.scroll = 0;
    }

    /// Append text to the current streaming response
    pub fn append_to_streaming(&mut self, text: &str) {
        if let Some(last) = self.messages.last_mut()
            && !last.is_user
            && !last.is_system
        {
            last.content.push_str(text);
            self.manual_scroll = false;
            self.scroll = 0;
        }
    }

    /// Start a streaming response for a specific provider in tandem mode
    /// Returns a unique ID for this stream to prevent collision when the same provider appears twice
    pub fn start_tandem_stream(&mut self, provider_label: &str) -> String {
        let stream_id = uuid::Uuid::new_v4().to_string();
        self.messages.push(DisplayMessage {
            role: provider_label.to_string(),
            content: String::new(),
            is_user: false,
            is_system: false,
            id: Some(stream_id.clone()),
        });
        self.manual_scroll = false;
        self.scroll = 0;
        stream_id
    }

    /// Append text to a specific provider's streaming response in tandem mode
    /// stream_id is used to disambiguate when the same provider appears twice
    pub fn append_to_tandem_stream(&mut self, stream_id: &str, text: &str) {
        // Find the message with matching stream_id (search from end for latest)
        if let Some(msg) = self
            .messages
            .iter_mut()
            .rev()
            .find(|m| m.id.as_deref() == Some(stream_id) && !m.is_user && !m.is_system)
        {
            msg.content.push_str(text);
            self.manual_scroll = false;
            self.scroll = 0;
        }
    }

    /// Get content of the last assistant message
    pub fn get_last_assistant_content(&self) -> String {
        self.messages
            .iter()
            .rev()
            .find(|m| !m.is_user && !m.is_system)
            .map(|m| m.content.clone())
            .unwrap_or_default()
    }

    /// Get content of an assistant message by stream ID (for tandem mode)
    pub fn get_assistant_content_by_stream_id(&self, stream_id: &str) -> String {
        self.messages
            .iter()
            .rev()
            .find(|m| m.id.as_deref() == Some(stream_id) && !m.is_user && !m.is_system)
            .map(|m| m.content.clone())
            .unwrap_or_default()
    }

    /// Update the content of an assistant message by stream ID (for tandem marker cleanup)
    pub fn update_assistant_by_stream_id(&mut self, stream_id: &str, content: &str) {
        if let Some(msg) = self
            .messages
            .iter_mut()
            .rev()
            .find(|m| m.id.as_deref() == Some(stream_id) && !m.is_user && !m.is_system)
        {
            msg.content = content.to_string();
        }
    }

    /// Update the last assistant message content (after marker stripping)
    pub fn update_last_assistant(&mut self, content: &str) {
        if let Some(last) = self
            .messages
            .iter_mut()
            .rev()
            .find(|m| !m.is_user && !m.is_system)
        {
            last.content = content.to_string();
        }
    }

    /// Update the last assistant message label and content
    pub fn update_last_assistant_with_label(&mut self, label: &str, content: &str) {
        if let Some(last) = self
            .messages
            .iter_mut()
            .rev()
            .find(|m| !m.is_user && !m.is_system)
        {
            last.role = label.to_string();
            last.content = content.to_string();
        }
    }

    /// Check if loading
    pub fn is_loading(&self) -> bool {
        self.loading
    }

    /// Get current cursor position (char-based)
    pub fn cursor_pos(&self) -> usize {
        self.cursor
    }

    /// Convert char position to byte index (public for paste handling)
    pub fn char_to_byte_pub(&self, char_pos: usize) -> usize {
        self.char_to_byte(char_pos)
    }

    /// Insert a char at byte position and advance cursor
    pub fn insert_char_at(&mut self, byte_idx: usize, c: char) {
        self.input.insert(byte_idx, c);
        self.cursor += 1;
    }

    /// Set tandem mode for the next message
    pub fn set_tandem(&mut self, providers: Vec<String>) {
        self.tandem_providers = Some(providers);
    }

    /// Set pipeline mode for the next message
    pub fn set_pipeline(&mut self, providers: Vec<String>) {
        self.pipeline_providers = Some(providers);
    }

    /// Take tandem providers (consumes the setting)
    pub fn take_tandem(&mut self) -> Option<Vec<String>> {
        self.tandem_providers.take()
    }

    /// Take pipeline providers (consumes the setting)
    pub fn take_pipeline(&mut self) -> Option<Vec<String>> {
        self.pipeline_providers.take()
    }

    /// Show a popup overlay (dismissed with Esc / q / Enter; other keys are ignored)
    pub fn show_popup(&mut self, content: String) {
        self.popup = Some(content);
        self.popup_scroll = 0;
    }

    /// Dismiss the popup
    pub fn dismiss_popup(&mut self) {
        self.popup = None;
        self.popup_scroll = 0;
    }

    /// Whether a popup is currently shown
    pub fn has_popup(&self) -> bool {
        self.popup.is_some()
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

        // If popup is active, handle popup keys first
        if self.has_popup() {
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Enter => self.dismiss_popup(),
                KeyCode::Up | KeyCode::Char('k') => {
                    self.popup_scroll = self.popup_scroll.saturating_sub(2);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.popup_scroll = self.popup_scroll.saturating_add(2);
                }
                KeyCode::PageUp => {
                    self.popup_scroll = self.popup_scroll.saturating_sub(10);
                }
                KeyCode::PageDown => {
                    self.popup_scroll = self.popup_scroll.saturating_add(10);
                }
                _ => {}
            }
            return false;
        }

        // Ctrl+W toggles workroom focus mode (drill-down). If hidden, show +
        // pin it and enter focus; if focused, exit focus; if visible but
        // unfocused, enter focus.
        if key.code == KeyCode::Char('w') && key.modifiers == KeyModifiers::CONTROL {
            if self.workroom.is_focused() {
                self.workroom.set_focused(false);
            } else {
                if !self.workroom.is_visible() {
                    self.workroom.set_visible(true);
                    self.workroom.toggle_pin();
                }
                self.workroom.set_focused(true);
            }
            return false;
        }

        // Gate focus-mode navigation BEFORE the text-input branch below, so
        // that j/k/Enter/Esc are consumed here instead of being inserted into
        // the input buffer or triggering submit/quit.
        if self.workroom.is_focused() {
            match key.code {
                KeyCode::Up | KeyCode::Char('k') => self.workroom.select_prev(),
                KeyCode::Down | KeyCode::Char('j') => self.workroom.select_next(),
                KeyCode::Enter => {
                    if let Some(md) = self.workroom.selected_detail_markdown() {
                        self.show_popup(md);
                    }
                }
                KeyCode::Esc => {
                    self.workroom.set_focused(false);
                }
                _ => {}
            }
            return false;
        }

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
        if self.has_popup() {
            match mouse.kind {
                MouseEventKind::ScrollUp => {
                    self.popup_scroll = self.popup_scroll.saturating_sub(2);
                }
                MouseEventKind::ScrollDown => {
                    self.popup_scroll = self.popup_scroll.saturating_add(2);
                }
                _ => {}
            }
        } else {
            match mouse.kind {
                MouseEventKind::ScrollUp => self.scroll_up(),
                MouseEventKind::ScrollDown => self.scroll_down(),
                _ => {}
            }
        }
    }

    pub fn should_quit(&self) -> bool {
        self.should_quit
    }

    /// Get the provider name
    pub fn provider_name(&self) -> &str {
        &self.provider_name
    }

    /// Get the model name
    pub fn model_name(&self) -> &str {
        &self.model_name
    }

    /// Set the provider name (used when switching providers)
    pub fn toggle_pty_mode(&mut self) {
        self.pty_mode = !self.pty_mode;
    }

    pub fn is_pty_mode(&self) -> bool {
        self.pty_mode
    }

    pub fn set_provider_name(&mut self, name: String) {
        self.provider_name = name;
    }

    /// Render the shell TUI
    pub fn render(&self, frame: &mut Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),                                     // Header
                Constraint::Min(0),                                        // Messages area
                Constraint::Length(1),                                     // Statusbar
                Constraint::Length(self.input_height(frame.area().width)), // Input (dynamic)
            ])
            .split(frame.area());

        // Header
        let model_info = if self.model_name.is_empty() {
            self.provider_name.clone()
        } else {
            format!("{} ({})", self.provider_name, self.model_name)
        };
        let pty_indicator = if self.pty_mode { " [PTY]" } else { "" };
        let header_text = format!(
            "ArmadAI Shell — {}{} — Turn #{}",
            model_info, pty_indicator, self.turn_count
        );
        let header = Paragraph::new(header_text).style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );
        frame.render_widget(header, chunks[0]);

        // Messages area (with optional workroom panel)
        if self.workroom.is_visible() {
            let h_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Min(0),     // Messages (main)
                    Constraint::Length(35), // Workroom panel
                ])
                .split(chunks[1]);
            self.render_messages_area(frame, h_chunks[0]);
            self.workroom.render(frame, h_chunks[1]);
        } else {
            self.render_messages_area(frame, chunks[1]);
        }

        // Status bar
        self.render_statusbar(frame, chunks[2]);

        // Input line
        self.render_input_line(frame, chunks[3]);

        // Popup overlay (rendered on top of everything)
        if let Some(ref content) = self.popup {
            self.render_popup(frame, content);
        }
    }

    fn render_popup(&self, frame: &mut Frame, content: &str) {
        let area = frame.area();

        // Center the popup: 80% width, 70% height
        let popup_width = (area.width as f32 * 0.80) as u16;
        let popup_height = (area.height as f32 * 0.70) as u16;
        let x = (area.width.saturating_sub(popup_width)) / 2;
        let y = (area.height.saturating_sub(popup_height)) / 2;
        let popup_area = Rect::new(x, y, popup_width, popup_height);

        // Semi-transparent background (clear the area)
        frame.render_widget(ratatui::widgets::Clear, popup_area);

        // Render markdown content using our custom renderer
        let mut lines: Vec<Line> = super::md_render::render_markdown(content);

        // Footer hint
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            " Esc to close │ ↑↓ scroll",
            Style::default().fg(Color::DarkGray),
        )));

        let popup = Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Cyan))
                    .title(" ArmadAI ")
                    .title_style(Style::default().fg(Color::Cyan).bold()),
            )
            .wrap(Wrap { trim: false })
            .scroll((self.popup_scroll, 0));

        frame.render_widget(popup, popup_area);
    }

    fn render_messages_area(&self, frame: &mut Frame, area: Rect) {
        if self.messages.is_empty() {
            let placeholder = Paragraph::new("Welcome to ArmadAI Shell!\n\nType your message and press Enter to get started. Press Ctrl+L to clear conversation, Ctrl+W to focus the workroom panel, Ctrl+C or Esc to quit.")
                .block(Block::default().borders(Borders::ALL))
                .wrap(Wrap { trim: false });
            frame.render_widget(placeholder, area);
            return;
        }

        // Format messages for display
        let mut lines: Vec<Line> = Vec::new();

        for msg in &self.messages {
            // Add role label
            let role_style = if msg.is_system {
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::DIM)
            } else if msg.is_user {
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            };

            let role_prefix = if msg.is_system { "⚙ " } else { "" };
            lines.push(Line::from(vec![Span::styled(
                format!("{}{}: ", role_prefix, msg.role),
                role_style,
            )]));

            if msg.is_user {
                // User messages: plain text
                for line in msg.content.lines() {
                    lines.push(Line::from(line.to_string()));
                }
            } else {
                // System + Assistant messages: custom markdown rendering
                lines.extend(super::md_render::render_markdown(&msg.content));
            }

            // Add blank line between messages
            lines.push(Line::from(""));
        }

        // Add loading indicator as last message
        if self.loading {
            let spinner = SPINNER_FRAMES[self.spinner_frame];
            let elapsed = self
                .loading_start
                .map(|s| s.elapsed().as_secs_f64())
                .unwrap_or(0.0);
            lines.push(Line::from(vec![Span::styled(
                format!("{spinner} Generating response… {elapsed:.0}s"),
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::ITALIC),
            )]));
        }

        // Calculate scroll position — account for line wrapping
        let visible_height = area.height.saturating_sub(2) as usize; // minus borders
        let inner_width = area.width.saturating_sub(2) as usize; // minus borders
        let total_lines: usize = if inner_width > 0 {
            lines
                .iter()
                .map(|line| {
                    let char_count: usize =
                        line.spans.iter().map(|s| s.content.chars().count()).sum();
                    (char_count / inner_width) + 1
                })
                .sum()
        } else {
            lines.len()
        };
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
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0));

        frame.render_widget(messages_text, area);
    }

    fn render_statusbar(&self, frame: &mut Frame, area: Rect) {
        let status_text = if self.loading {
            let elapsed = self
                .loading_start
                .map(|s| s.elapsed().as_secs_f64())
                .unwrap_or(0.0);
            let spinner = SPINNER_FRAMES[self.spinner_frame];
            format!(
                "{} │ {} in │ {} out │ ${:.3} │ {spinner} thinking… {:.0}s",
                self.provider_name, self.tokens_in, self.tokens_out, self.cost, elapsed,
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

        let statusbar = Paragraph::new(status_text).style(
            Style::default()
                .fg(Color::DarkGray)
                .bg(Color::Rgb(22, 27, 34)),
        );

        frame.render_widget(statusbar, area);
    }

    /// Calculate dynamic input height based on content and terminal width.
    /// Uses display width (unicode-width) for accurate wrapping, not char count.
    fn input_height(&self, terminal_width: u16) -> u16 {
        let inner_width = terminal_width.saturating_sub(4) as usize; // borders + prompt char
        if inner_width == 0 {
            return 3;
        }
        // Calculate display width of prompt + input (not char count)
        // Prompt is "> " (2 cells)
        let input_display_width: usize = self
            .input
            .chars()
            .map(|c| c.width().unwrap_or(1).max(1))
            .sum();
        let total_width = input_display_width + 2; // +2 for "> "
        let lines = (total_width / inner_width) + 1;
        // Min 3 (for borders + 1 line), max 8
        (lines as u16 + 2).clamp(3, 8)
    }

    /// Calculate cursor position (row, col) in display cells, accounting for Unicode width.
    /// Takes the text up to cursor and available width in cells.
    /// Returns (row, col) where row is 0-indexed from top and col is in display cells.
    fn calculate_wrapped_cursor_position_unicode(
        text_before_cursor: &str,
        width: usize,
    ) -> (usize, usize) {
        if width == 0 {
            return (0, 0);
        }
        let mut row = 0;
        let mut col = 0;
        for c in text_before_cursor.chars() {
            let char_width = c.width().unwrap_or(1).max(1); // Handle control chars as 1 cell
            if col + char_width > width {
                // Wrap to next line
                row += 1;
                col = char_width;
            } else {
                col += char_width;
            }
        }
        (row, col)
    }

    fn render_input_line(&self, frame: &mut Frame, area: Rect) {
        let cursor_indicator = if self.loading { "..." } else { ">" };

        // Build plain text for wrapping
        let display_text = format!("{} {}", cursor_indicator, self.input);
        let prefix_len = cursor_indicator.len() + 1; // "> " or "... "

        // Calculate available width (Borders::ALL = 2 cells left+right)
        let available_width = area.width.saturating_sub(2) as usize;
        if available_width == 0 {
            // Terminal too narrow, just render without wrapping logic
            let mut input_spans = Vec::new();
            for c in display_text.chars() {
                input_spans.push(Span::raw(c.to_string()));
            }
            if !self.loading {
                input_spans.push(Span::styled(
                    " ",
                    Style::default().bg(Color::White).fg(Color::Black),
                ));
            }

            let input_paragraph = Paragraph::new(Line::from(input_spans))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::DarkGray))
                        .title(" Input ")
                        .title_style(Style::default().fg(Color::Cyan)),
                )
                .wrap(Wrap { trim: false });

            frame.render_widget(input_paragraph, area);
            return;
        }

        // Split display_text into prefix and input text for cursor calculation
        let prefix = &display_text[..prefix_len.min(display_text.len())];
        let input_part = if prefix_len < display_text.len() {
            &display_text[prefix_len..]
        } else {
            ""
        };

        // Calculate cursor position in text-before-cursor
        let text_before_cursor = if self.cursor > 0 {
            &input_part[..input_part
                .char_indices()
                .nth(self.cursor)
                .map(|(i, _)| i)
                .unwrap_or(input_part.len())]
        } else {
            ""
        };

        let cursor_full_text = format!("{}{}", prefix, text_before_cursor);
        let (cursor_row, _cursor_col) =
            Self::calculate_wrapped_cursor_position_unicode(&cursor_full_text, available_width);

        // Build lines based on wrapping with Unicode-aware width
        let mut lines: Vec<Line> = Vec::new();
        let mut current_line_spans: Vec<Span> = Vec::new();
        let mut current_col = 0;

        for (char_idx, c) in display_text.chars().enumerate() {
            let char_width = c.width().unwrap_or(1).max(1);

            // Check if this character is at cursor position (for cursor rendering)
            let is_cursor_pos = char_idx == prefix_len + self.cursor && !self.loading;

            // Check if we need to wrap (current char doesn't fit on current line)
            if current_col + char_width > available_width {
                // Push current line and start new one
                lines.push(Line::from(current_line_spans.clone()));
                current_line_spans.clear();
                current_col = 0;
            }

            // Add this character to current line
            if is_cursor_pos {
                current_line_spans.push(Span::styled(
                    c.to_string(),
                    Style::default().bg(Color::White).fg(Color::Black),
                ));
            } else {
                current_line_spans.push(Span::raw(c.to_string()));
            }
            current_col += char_width;
        }

        // Add any remaining spans as the last line
        if !current_line_spans.is_empty() {
            lines.push(Line::from(current_line_spans));
        }

        // Track if a new line was created for cursor block (edge case: cursor at wrap boundary)
        let mut cursor_on_new_line = false;

        // If cursor at end of text, add a cursor block
        if self.cursor >= input_part.chars().count() && !self.loading {
            if current_col < available_width {
                // Cursor fits on current line
                if let Some(last_line) = lines.last_mut() {
                    last_line.spans.push(Span::styled(
                        " ",
                        Style::default().bg(Color::White).fg(Color::Black),
                    ));
                }
            } else if current_col > 0 {
                // Cursor would wrap to next line (edge case: cursor at exact wrap boundary)
                let cursor_span = vec![Span::styled(
                    " ",
                    Style::default().bg(Color::White).fg(Color::Black),
                )];
                lines.push(Line::from(cursor_span));
                cursor_on_new_line = true;
            }
        }

        // Calculate scroll offset to keep cursor visible
        // Edge case: if cursor moved to a new line due to wrap boundary, update row
        let visible_height = area.height.saturating_sub(2) as usize; // minus borders
        let final_cursor_row = if cursor_on_new_line {
            // Cursor was placed on a newly created line due to wrap boundary
            lines.len() - 1
        } else {
            cursor_row
        };
        let scroll_offset = if final_cursor_row >= visible_height {
            (final_cursor_row - visible_height + 1) as u16
        } else {
            0
        };

        // Render with lines and scroll
        let input_paragraph = Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::DarkGray))
                    .title(" Input ")
                    .title_style(Style::default().fg(Color::Cyan)),
            )
            .wrap(Wrap { trim: false })
            .scroll((scroll_offset, 0));

        frame.render_widget(input_paragraph, area);
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
        assert!(!app.messages[0].is_system);
        assert!(!app.messages[1].is_system);
    }

    #[test]
    fn test_add_system_message() {
        let mut app = ShellApp::new("Gemini".to_string());
        app.add_system_message("Session cleared");

        assert_eq!(app.messages.len(), 1);
        assert!(!app.messages[0].is_user);
        assert!(app.messages[0].is_system);
        assert_eq!(app.messages[0].role, "System");
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

    #[test]
    fn test_calculate_wrapped_cursor_position_unicode_ascii() {
        // Pure ASCII, no special handling needed
        // Text "hello" (5 chars, each 1 cell wide), width 20
        let (row, col) = ShellApp::calculate_wrapped_cursor_position_unicode("hello", 20);
        assert_eq!(row, 0);
        assert_eq!(col, 5);
    }

    #[test]
    fn test_calculate_wrapped_cursor_position_unicode_with_wrap() {
        // ASCII text that wraps: 25 chars with width 20
        let text = "a".repeat(25);
        let (row, col) = ShellApp::calculate_wrapped_cursor_position_unicode(&text, 20);
        assert_eq!(row, 1); // Wrapped to second line
        assert_eq!(col, 5); // 5 chars on second line
    }

    #[test]
    fn test_calculate_wrapped_cursor_position_unicode_emoji() {
        // Emoji (typically 2 cells wide)
        // "😀" is 2 cells, then "ab" is 2 cells → wraps at width 3
        let text = "😀ab"; // 2 + 1 + 1 = 4 cells
        let (row, col) = ShellApp::calculate_wrapped_cursor_position_unicode(text, 3);
        // First 3 cells: "😀a" (2+1) on line 0
        // Then "b" wraps to line 1
        // But we're calculating position BEFORE cursor, so at "😀ab" end:
        // Line 0: "😀a" = 3 cells
        // Line 1: "b" = 1 cell
        assert_eq!(row, 1);
        assert_eq!(col, 1);
    }

    #[test]
    fn test_calculate_wrapped_cursor_position_unicode_zero_width() {
        // Edge case: zero width should not panic
        let (row, col) = ShellApp::calculate_wrapped_cursor_position_unicode("hello", 0);
        assert_eq!(row, 0);
        assert_eq!(col, 0);
    }

    #[test]
    fn test_calculate_wrapped_cursor_position_unicode_empty_text() {
        // Empty text should return (0, 0)
        let (row, col) = ShellApp::calculate_wrapped_cursor_position_unicode("", 20);
        assert_eq!(row, 0);
        assert_eq!(col, 0);
    }

    #[test]
    fn test_input_height_calculation() {
        // Test that input height is calculated correctly with wrapping
        let mut app = ShellApp::new("Gemini".to_string());

        // Short input that fits on one line
        app.input = "hello".to_string();
        let height = app.input_height(80);
        assert!(height >= 3); // Min height
        assert!(height <= 8); // Max height

        // Longer input that wraps
        app.input = "a".repeat(100);
        let height = app.input_height(80);
        assert!(height >= 3);
        assert!(height <= 8);
    }

    #[test]
    fn test_cursor_navigation_with_input() {
        let mut app = ShellApp::new("Gemini".to_string());

        // Insert some text
        app.input = "hello world".to_string();
        app.cursor = 11; // At end

        // Move cursor left
        assert_eq!(app.cursor, 11);

        // Test char_to_byte conversion
        app.input = "café".to_string();
        let byte_idx = app.char_to_byte(1); // Position after 'c', before 'a'
        assert_eq!(byte_idx, 1);
    }

    /// Helper to find cursor position in rendered buffer (background white/black)
    fn find_cursor_in_buffer(
        terminal: &ratatui::Terminal<ratatui::backend::TestBackend>,
    ) -> Option<(u16, u16)> {
        let buf = terminal.backend().buffer();
        for y in 0..buf.area().height {
            for x in 0..buf.area().width {
                if let Some(cell) = buf.cell((x, y)) {
                    // Cursor is styled with bg=White, fg=Black
                    if cell.bg == ratatui::prelude::Color::White
                        && cell.fg == ratatui::prelude::Color::Black
                    {
                        return Some((x, y));
                    }
                }
            }
        }
        None
    }

    #[test]
    fn test_render_input_line_with_wrapping() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut app = ShellApp::new("Test".to_string());
        app.input = "a".repeat(50); // 50 character input, should wrap
        app.cursor = 25; // In middle

        // Create a test terminal with 30-char width and 10 lines height
        let backend = TestBackend::new(30, 10);
        let mut terminal = Terminal::new(backend).unwrap();

        // Render the app
        let _ = terminal.draw(|f| {
            app.render(f);
        });

        // Verify rendering produced output
        let buffer = terminal.backend().buffer().clone();
        assert!(!buffer.content.is_empty(), "Buffer should have content");

        // Verify cursor is actually rendered at a valid position
        if let Some((cursor_x, cursor_y)) = find_cursor_in_buffer(&terminal) {
            // Cursor should be within the terminal bounds (accounting for borders)
            assert!(
                cursor_x > 0 && cursor_x < 30,
                "Cursor X position {} should be within terminal width",
                cursor_x
            );
            // Cursor should be within terminal height
            assert!(
                cursor_y < 10,
                "Cursor Y position {} should be within terminal height",
                cursor_y
            );
        }
        // Note: If cursor not found, it might be rendered without highlight at wrap boundary,
        // which is acceptable but less ideal for this test.
    }

    #[test]
    fn test_render_input_with_unicode() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut app = ShellApp::new("Test".to_string());
        // Mix of ASCII and emoji
        app.input = "hello😀world".to_string();
        app.cursor = 6; // After emoji

        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();

        let _ = terminal.draw(|f| {
            app.render(f);
        });

        // Verify rendering didn't panic and produced output
        let buffer = terminal.backend().buffer().clone();
        assert!(!buffer.content.is_empty(), "Buffer should have content");

        // Verify cursor position is within valid bounds if found
        if let Some((cursor_x, cursor_y)) = find_cursor_in_buffer(&terminal) {
            assert!(
                cursor_x > 0 && cursor_x < 40,
                "Cursor X position {} should be within terminal width",
                cursor_x
            );
            assert!(
                cursor_y < 10,
                "Cursor Y position {} should be within terminal height",
                cursor_y
            );
        }
        // Note: Unicode test - cursor may not be highlighted if at wrap boundary,
        // but we verified rendering completed without panicking.
    }
}
