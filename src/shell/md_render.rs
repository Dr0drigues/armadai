//! Custom Markdown renderer for the shell TUI.
//!
//! Uses `pulldown-cmark` to parse markdown and produces `ratatui` `Line`/`Span`
//! objects with proper styling. Replaces `tui-markdown` for full control over
//! rendering (tables, code blocks, headers, etc.).

#![cfg(feature = "tui")]

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use ratatui::{
    prelude::*,
    text::{Line, Span},
};

// ── Colors (GitHub dark theme) ──────────────────────────────────

const H1_COLOR: Color = Color::Rgb(88, 166, 255);
const H2_COLOR: Color = Color::Rgb(63, 185, 80);
const H3_COLOR: Color = Color::Rgb(210, 153, 34);
const H4_COLOR: Color = Color::Rgb(139, 148, 158);
const CODE_FG: Color = Color::Rgb(230, 237, 243);
const CODE_BG: Color = Color::Rgb(55, 62, 71);
const LINK_COLOR: Color = Color::Rgb(88, 166, 255);
const QUOTE_COLOR: Color = Color::Rgb(139, 148, 158);
const SEPARATOR_COLOR: Color = Color::DarkGray;
const TABLE_BORDER_COLOR: Color = Color::DarkGray;
const TABLE_HEADER_COLOR: Color = Color::Rgb(88, 166, 255);

// ── Public API ──────────────────────────────────────────────────

/// Render markdown text into ratatui Lines.
pub fn render_markdown(input: &str) -> Vec<Line<'static>> {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);

    let parser = Parser::new_ext(input, opts);
    let mut renderer = MdRenderer::new();
    renderer.process(parser);
    renderer.lines
}

// ── Renderer state machine ──────────────────────────────────────

struct MdRenderer {
    lines: Vec<Line<'static>>,
    /// Current line being built
    current_spans: Vec<Span<'static>>,
    /// Style stack (bold, italic, code, etc.)
    style_stack: Vec<Style>,
    /// Current base style
    base_style: Style,
    /// Inside a code block
    in_code_block: bool,
    /// Code block language
    code_lang: String,
    /// Code block accumulated content
    code_buffer: String,
    /// Inside a heading (level)
    heading_level: Option<u8>,
    /// Inside a blockquote
    in_blockquote: bool,
    /// List stack (None = unordered, Some(n) = ordered starting at n)
    list_stack: Vec<Option<u64>>,
    /// Current list item index
    list_item_index: u64,
    /// Inside a table
    in_table: bool,
    /// Table rows: each row is a Vec of cell contents
    table_rows: Vec<Vec<String>>,
    /// Current table row being built
    current_row: Vec<String>,
    /// Current table cell being built
    current_cell: String,
    /// Is current row a header row
    table_header_row: bool,
}

impl MdRenderer {
    fn new() -> Self {
        Self {
            lines: Vec::new(),
            current_spans: Vec::new(),
            style_stack: Vec::new(),
            base_style: Style::default(),
            in_code_block: false,
            code_lang: String::new(),
            code_buffer: String::new(),
            heading_level: None,
            in_blockquote: false,
            list_stack: Vec::new(),
            list_item_index: 0,
            in_table: false,
            table_rows: Vec::new(),
            current_row: Vec::new(),
            current_cell: String::new(),
            table_header_row: false,
        }
    }

    fn process(&mut self, parser: Parser) {
        for event in parser {
            match event {
                Event::Start(tag) => self.start_tag(tag),
                Event::End(tag) => self.end_tag(tag),
                Event::Text(text) => self.text(&text),
                Event::Code(code) => self.inline_code(&code),
                Event::SoftBreak | Event::HardBreak => self.line_break(),
                Event::Rule => self.rule(),
                _ => {}
            }
        }
        // Flush any remaining spans
        self.flush_line();
    }

    // ── Tag handlers ────────────────────────────────────────────

    fn start_tag(&mut self, tag: Tag) {
        match tag {
            Tag::Heading { level, .. } => {
                self.flush_line();
                self.lines.push(Line::from(""));
                let lvl = match level {
                    pulldown_cmark::HeadingLevel::H1 => 1,
                    pulldown_cmark::HeadingLevel::H2 => 2,
                    pulldown_cmark::HeadingLevel::H3 => 3,
                    pulldown_cmark::HeadingLevel::H4 => 4,
                    pulldown_cmark::HeadingLevel::H5 => 5,
                    pulldown_cmark::HeadingLevel::H6 => 6,
                };
                self.heading_level = Some(lvl);
                self.base_style = heading_style(lvl);
            }
            Tag::Paragraph => {
                // Start a new paragraph (blank line before if not first)
                if !self.lines.is_empty() && !self.in_table {
                    self.flush_line();
                }
            }
            Tag::CodeBlock(kind) => {
                self.flush_line();
                self.in_code_block = true;
                self.code_buffer.clear();
                self.code_lang = match kind {
                    pulldown_cmark::CodeBlockKind::Fenced(lang) => lang.to_string(),
                    _ => String::new(),
                };
            }
            Tag::BlockQuote(_) => {
                self.in_blockquote = true;
            }
            Tag::List(start) => {
                self.flush_line();
                self.list_stack.push(start);
                self.list_item_index = start.unwrap_or(1);
            }
            Tag::Item => {
                self.flush_line();
                let indent = "  ".repeat(self.list_stack.len().saturating_sub(1));
                let bullet = if self.list_stack.last() == Some(&None) {
                    format!("{indent}- ")
                } else {
                    let idx = self.list_item_index;
                    self.list_item_index += 1;
                    format!("{indent}{idx}. ")
                };
                self.current_spans
                    .push(Span::styled(bullet, Style::default().fg(SEPARATOR_COLOR)));
            }
            Tag::Emphasis => {
                self.style_stack.push(self.base_style);
                self.base_style = self.base_style.add_modifier(Modifier::ITALIC);
            }
            Tag::Strong => {
                self.style_stack.push(self.base_style);
                self.base_style = self.base_style.add_modifier(Modifier::BOLD);
            }
            Tag::Strikethrough => {
                self.style_stack.push(self.base_style);
                self.base_style = self.base_style.add_modifier(Modifier::CROSSED_OUT);
            }
            Tag::Link { dest_url, .. } => {
                self.style_stack.push(self.base_style);
                self.base_style = Style::default().fg(LINK_COLOR).underlined();
                let _ = dest_url; // we show the text, not the URL
            }
            Tag::Table(_) => {
                self.flush_line();
                self.in_table = true;
                self.table_rows.clear();
            }
            Tag::TableHead => {
                self.table_header_row = true;
                self.current_row.clear();
            }
            Tag::TableRow => {
                self.current_row.clear();
            }
            Tag::TableCell => {
                self.current_cell.clear();
            }
            _ => {}
        }
    }

    fn end_tag(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Heading(_) => {
                self.flush_line_with_style(self.base_style);
                if let Some(level) = self.heading_level
                    && level <= 3
                {
                    self.lines.push(Line::from(Span::styled(
                        "─".repeat(50),
                        Style::default().fg(SEPARATOR_COLOR),
                    )));
                }
                self.heading_level = None;
                self.base_style = Style::default();
            }
            TagEnd::Paragraph => {
                self.flush_line();
            }
            TagEnd::CodeBlock => {
                self.render_code_block();
                self.in_code_block = false;
            }
            TagEnd::BlockQuote(_) => {
                self.in_blockquote = false;
            }
            TagEnd::List(_) => {
                self.list_stack.pop();
                if self.list_stack.is_empty() {
                    self.flush_line();
                }
            }
            TagEnd::Item => {
                self.flush_line();
            }
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough | TagEnd::Link => {
                if let Some(prev) = self.style_stack.pop() {
                    self.base_style = prev;
                }
            }
            TagEnd::Table => {
                self.render_table();
                self.in_table = false;
            }
            TagEnd::TableHead => {
                self.table_rows.push(self.current_row.clone());
                self.table_header_row = false;
            }
            TagEnd::TableRow => {
                self.table_rows.push(self.current_row.clone());
            }
            TagEnd::TableCell => {
                self.current_row.push(self.current_cell.clone());
            }
            _ => {}
        }
    }

    // ── Content handlers ────────────────────────────────────────

    fn text(&mut self, text: &str) {
        if self.in_code_block {
            self.code_buffer.push_str(text);
            return;
        }
        if self.in_table {
            self.current_cell.push_str(text);
            return;
        }

        let style = if self.in_blockquote {
            Style::default().fg(QUOTE_COLOR).italic()
        } else {
            self.base_style
        };

        let prefix = if self.in_blockquote { "│ " } else { "" };

        for (i, line) in text.split('\n').enumerate() {
            if i > 0 {
                self.flush_line();
            }
            if !line.is_empty() {
                let display = if !prefix.is_empty() && self.current_spans.is_empty() {
                    format!("{prefix}{line}")
                } else {
                    line.to_string()
                };
                self.current_spans.push(Span::styled(display, style));
            }
        }
    }

    fn inline_code(&mut self, code: &str) {
        if self.in_table {
            self.current_cell.push_str(code);
            return;
        }
        self.current_spans.push(Span::styled(
            format!(" {code} "),
            Style::default().fg(CODE_FG).bg(CODE_BG),
        ));
    }

    fn line_break(&mut self) {
        self.flush_line();
    }

    fn rule(&mut self) {
        self.flush_line();
        self.lines.push(Line::from(Span::styled(
            "───────────────────────────────────────────────────",
            Style::default().fg(SEPARATOR_COLOR),
        )));
    }

    // ── Flush helpers ───────────────────────────────────────────

    fn flush_line(&mut self) {
        if !self.current_spans.is_empty() {
            let spans = std::mem::take(&mut self.current_spans);
            self.lines.push(Line::from(spans));
        }
    }

    fn flush_line_with_style(&mut self, style: Style) {
        if !self.current_spans.is_empty() {
            let spans = std::mem::take(&mut self.current_spans);
            self.lines.push(Line::from(spans).style(style));
        }
    }

    // ── Code block rendering ────────────────────────────────────

    fn render_code_block(&mut self) {
        let lang_label = if self.code_lang.is_empty() {
            String::new()
        } else {
            format!(" {} ", self.code_lang)
        };

        // Top border
        self.lines.push(Line::from(Span::styled(
            format!(
                "┌{lang_label}{}┐",
                "─".repeat(48_usize.saturating_sub(lang_label.len()))
            ),
            Style::default().fg(SEPARATOR_COLOR),
        )));

        // Code lines
        let code_style = Style::default().fg(CODE_FG).bg(CODE_BG);
        for line in self.code_buffer.lines() {
            self.lines.push(Line::from(vec![
                Span::styled("│ ", Style::default().fg(SEPARATOR_COLOR)),
                Span::styled(line.to_string(), code_style),
            ]));
        }

        // Bottom border
        self.lines.push(Line::from(Span::styled(
            format!("└{}┘", "─".repeat(48)),
            Style::default().fg(SEPARATOR_COLOR),
        )));
    }

    // ── Table rendering ─────────────────────────────────────────

    fn render_table(&mut self) {
        if self.table_rows.is_empty() {
            return;
        }

        // Calculate column widths
        let col_count = self.table_rows.iter().map(|r| r.len()).max().unwrap_or(0);
        let mut col_widths = vec![0usize; col_count];

        for row in &self.table_rows {
            for (i, cell) in row.iter().enumerate() {
                if i < col_count {
                    col_widths[i] = col_widths[i].max(cell.len());
                }
            }
        }

        // Render each row
        for (row_idx, row) in self.table_rows.iter().enumerate() {
            let is_header = row_idx == 0;
            let mut spans: Vec<Span<'static>> = Vec::new();

            spans.push(Span::styled("│", Style::default().fg(TABLE_BORDER_COLOR)));

            for (i, cell) in row.iter().enumerate() {
                let width = col_widths.get(i).copied().unwrap_or(10);
                let padded = format!(" {:<width$} ", cell, width = width);

                let style = if is_header {
                    Style::default().fg(TABLE_HEADER_COLOR).bold()
                } else {
                    Style::default()
                };
                spans.push(Span::styled(padded, style));
                spans.push(Span::styled("│", Style::default().fg(TABLE_BORDER_COLOR)));
            }

            self.lines.push(Line::from(spans));

            // Separator after header
            if is_header {
                let mut sep_spans: Vec<Span<'static>> = Vec::new();
                sep_spans.push(Span::styled("├", Style::default().fg(TABLE_BORDER_COLOR)));
                for (i, &w) in col_widths.iter().enumerate() {
                    sep_spans.push(Span::styled(
                        "─".repeat(w + 2),
                        Style::default().fg(TABLE_BORDER_COLOR),
                    ));
                    if i < col_count - 1 {
                        sep_spans.push(Span::styled("┼", Style::default().fg(TABLE_BORDER_COLOR)));
                    }
                }
                sep_spans.push(Span::styled("┤", Style::default().fg(TABLE_BORDER_COLOR)));
                self.lines.push(Line::from(sep_spans));
            }
        }
    }
}

// ── Style helpers ───────────────────────────────────────────────

fn heading_style(level: u8) -> Style {
    match level {
        1 => Style::new().fg(H1_COLOR).bold().underlined(),
        2 => Style::new().fg(H2_COLOR).bold(),
        3 => Style::new().fg(H3_COLOR).bold(),
        _ => Style::new().fg(H4_COLOR).italic(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_plain_text() {
        let lines = render_markdown("Hello world");
        assert!(!lines.is_empty());
    }

    #[test]
    fn test_render_header() {
        let lines = render_markdown("# Title\n\nContent");
        // Should have: blank, title, separator, content
        assert!(lines.len() >= 3);
    }

    #[test]
    fn test_render_code_block() {
        let lines = render_markdown("```rust\nfn main() {}\n```");
        let text: String = lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.to_string()))
            .collect();
        assert!(text.contains("fn main()"));
    }

    #[test]
    fn test_render_table() {
        let md = "| A | B |\n|---|---|\n| 1 | 2 |\n| 3 | 4 |";
        let lines = render_markdown(md);
        let text: String = lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.to_string()))
            .collect();
        assert!(text.contains("A"));
        assert!(text.contains("1"));
    }

    #[test]
    fn test_render_inline_code() {
        let lines = render_markdown("Use `cargo build` here");
        let text: String = lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.to_string()))
            .collect();
        assert!(text.contains("cargo build"));
    }

    #[test]
    fn test_render_bold_italic() {
        let lines = render_markdown("**bold** and *italic*");
        assert!(!lines.is_empty());
    }

    #[test]
    fn test_render_list() {
        let lines = render_markdown("- item 1\n- item 2\n- item 3");
        assert!(lines.len() >= 3);
    }

    #[test]
    fn test_render_rule() {
        let lines = render_markdown("above\n\n---\n\nbelow");
        let has_rule = lines
            .iter()
            .any(|l| l.spans.iter().any(|s| s.content.contains("───")));
        assert!(has_rule);
    }
}
