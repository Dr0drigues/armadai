use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    widgets::{Block, Borders, Paragraph, Wrap},
};

use crate::tui::app::App;

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let status = if app.exec_running {
        " Execution [running...] "
    } else {
        " Execution "
    };

    if app.exec_output.is_empty() {
        let selected = app
            .agents
            .get(app.selected_agent)
            .map(|a| a.name.as_str())
            .unwrap_or("none");

        let msg = format!(
            "Select an agent in Dashboard (j/k), then press Enter to run.\n\n\
             Selected: {selected}\n\n\
             Streaming output will appear here."
        );
        let widget =
            Paragraph::new(msg).block(Block::default().borders(Borders::ALL).title(status));
        frame.render_widget(widget, area);
        return;
    }

    let text = app.exec_output.join("");
    let widget = Paragraph::new(text)
        .block(Block::default().borders(Borders::ALL).title(status))
        .style(Style::default().fg(Color::Green))
        .wrap(Wrap { trim: false });
    frame.render_widget(widget, area);
}
