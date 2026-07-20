//! Shared search-bar widget for list views (Agents, Prompts, Skills, Starters,
//! History, Models, Orchestration).
//!
//! Renders the one-line `/ query█` filter indicator pinned to the bottom of a
//! list panel. Extracted from the identical `render_search_bar` copy-pasted
//! across every list view so appearance stays in sync by construction.

use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    widgets::{Block, Paragraph},
};

/// Render the search/filter bar as the last line of `list_area`.
pub fn search_bar(frame: &mut Frame, query: &str, list_area: Rect) {
    let search_area = Rect {
        x: list_area.x,
        y: list_area.bottom() - 1,
        width: list_area.width,
        height: 1,
    };

    let query_display = format!("/ {query}\u{2588}");
    let search = Paragraph::new(query_display)
        .style(Style::default().fg(Color::Yellow))
        .block(Block::default());
    frame.render_widget(search, search_area);
}
