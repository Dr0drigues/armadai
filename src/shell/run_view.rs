#![cfg(feature = "tui")]

use std::io;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::Rect,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use crate::core::events::{EventSink, RunEvent};
use crate::core::orchestration::OrchestrationPattern;
use crate::shell::workroom::Workroom;
use crate::theme;

/// An `EventSink` that forwards a clone of every `RunEvent` into a channel,
/// so a TUI render loop can drain and project them onto a `Workroom`.
pub struct WorkroomSink {
    tx: UnboundedSender<RunEvent>,
}

impl WorkroomSink {
    pub fn new() -> (Self, UnboundedReceiver<RunEvent>) {
        let (tx, rx) = unbounded_channel();
        (Self { tx }, rx)
    }
}

impl EventSink for WorkroomSink {
    fn emit(&self, ev: &RunEvent) {
        // Receiver gone (TUI exited) → drop silently; the run still completes.
        let _ = self.tx.send(ev.clone());
    }
}

/// Restore the terminal to normal state. Called on exit and on panic — mirrors
/// `src/shell/app.rs::restore_terminal`. Operates on `io::stdout()` directly
/// (rather than through a `Terminal`/backend handle) so it can also run from
/// the panic hook, where no `Terminal` is reachable.
fn restore_terminal() {
    if let Err(e) = disable_raw_mode() {
        tracing::warn!("Failed to disable raw mode: {:?}", e);
    }
    if let Err(e) = execute!(io::stdout(), LeaveAlternateScreen, crossterm::cursor::Show) {
        tracing::warn!("Failed to restore terminal state: {:?}", e);
    }
    // Restore tracing output now that the alternate screen is gone — safe to
    // interleave with stdout again. Must run after the terminal is restored
    // (not before) so a log emitted between the two doesn't land mid-frame.
    crate::logging::restore();
}

/// Run an orchestration (`run`) while showing a live Workroom TUI fed by its
/// event stream. Restores the terminal on exit (including on error or panic),
/// and returns `(run_id, content)` for the caller to print *after* the
/// terminal has been restored: `run_id` is `Some` once a `RunStart` has been
/// observed (independent of whether the run produced a final answer), so a
/// caller can still surface it — for a later `--resume`/`--replay` — even on
/// an early abort. The alternate screen clears everything on exit, so this is
/// the only way the id survives in scrollback for the TUI path (OH1 Lot 6).
pub async fn run_orchestration_tui<F>(
    run: impl FnOnce(Arc<dyn EventSink>) -> F,
    config_yaml: Option<String>,
    explicit_pattern: Option<OrchestrationPattern>,
) -> anyhow::Result<(Option<String>, Option<String>)>
where
    F: std::future::Future<Output = anyhow::Result<()>> + Send + 'static,
{
    let (sink, mut rx) = WorkroomSink::new();
    let sink: Arc<dyn EventSink> = Arc::new(sink);

    // Seed roles from the orchestration config if available (RunStart carries
    // no roles); otherwise the flotte stays flat.
    let mut workroom = Workroom::new();
    if let Some(cfg) = config_yaml {
        workroom.init_from_config(&cfg);
    }
    // An explicit `--orchestrate <pattern>` flag overrides whatever
    // `init_from_config` inferred (or its `Hierarchical` default) — the
    // project config often has no `orchestration:` block at all for a
    // one-off explicit run, which would otherwise render the wrong layout.
    if let Some(pattern) = explicit_pattern {
        workroom.set_pattern(pattern);
    }
    workroom.set_visible(true);
    // Fullscreen dedicated run view (unlike the shell's narrow sidebar): show
    // the rich pattern layout (ring/tree/blackboard) by default rather than
    // requiring Ctrl+W to focus first. Ctrl+W still toggles it off/on below.
    workroom.set_focused(true);

    // Install a panic hook that restores the terminal before the default
    // handler runs (mirrors `src/shell/app.rs`). Without this, a panic while
    // raw mode + the alternate screen are active leaves the user's terminal
    // unusable (no echo, stuck on the alternate buffer) after the process
    // exits, since the sequential restore code below would never run.
    let default_panic = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        default_panic(info);
    }));

    // Launch the orchestration in the background.
    let handle = tokio::spawn(run(sink));

    // Silence tracing (e.g. the provider factory's `INFO … using CLI …` logs
    // emitted while the background orchestration builds providers) before we
    // ever touch the terminal: once the alternate screen is entered, stderr
    // writes from other tasks interleave with the TUI's own draws, corrupting
    // the rendered frame (observed as a cut-off top border on first render).
    // Restored in `restore_terminal()` above once the alternate screen is
    // torn down.
    crate::logging::suppress();

    // Enter alternate screen (mirrors src/shell/app.rs). Each step is
    // unwound individually on failure rather than bare-`?`-ing through: if
    // `EnterAlternateScreen` fails after raw mode was already enabled, or
    // `Terminal::new` fails after the alternate screen was already entered,
    // a bare `?` would return with the terminal left half-initialized.
    if let Err(e) = enable_raw_mode() {
        handle.abort();
        return Err(e.into());
    }
    let mut stdout = io::stdout();
    if let Err(e) = execute!(stdout, EnterAlternateScreen) {
        disable_raw_mode().ok();
        handle.abort();
        return Err(e.into());
    }
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = match Terminal::new(backend) {
        Ok(t) => t,
        Err(e) => {
            restore_terminal();
            handle.abort();
            return Err(e.into());
        }
    };

    let render_result = run_loop(&mut terminal, &mut workroom, &mut rx, handle).await;

    // Captured before `restore_terminal()` merely for clarity — the workroom
    // itself is untouched by the terminal teardown; read here so it's beside
    // the `render_result` it's paired with below.
    let run_id = workroom.run_id().map(str::to_string);

    // Always restore the terminal, even if the loop errored.
    restore_terminal();

    render_result.map(|content| (run_id, content))
}

/// Minimum sensible Workroom panel width (columns) — narrower than this and
/// the rich pattern layouts (ring/tree/blackboard) start wrapping awkwardly.
const WORKROOM_WIDTH: u16 = 72;
const WORKROOM_WIDTH_MIN: u16 = 50;

/// Roughly two lines per agent (arrow connectors between them in the
/// ring/tree layouts) plus room for the footer hint block.
const WORKROOM_HEIGHT_PER_AGENT: u16 = 2;
const WORKROOM_HEIGHT_BASE: u16 = 8;

/// Compute a `width` x `height` `Rect` centered within `area`, clamped so it
/// never exceeds the terminal's own bounds (a terminal smaller than the
/// requested size just gets the whole area).
fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let w = width.min(area.width);
    let h = height.min(area.height);
    let x = area.x + (area.width - w) / 2;
    let y = area.y + (area.height - h) / 2;
    Rect::new(x, y, w, h)
}

/// Render the agent detail popup (drill-down on `Enter`) as a centered
/// overlay on top of the Workroom, mirroring `src/shell/tui.rs::render_popup`
/// (Clear + markdown Paragraph + themed border) but sized ~70% of the
/// terminal and titled " Detail ".
fn render_detail_popup(frame: &mut Frame, area: Rect, markdown: &str) {
    let popup_width = (area.width as f32 * 0.70) as u16;
    let popup_height = (area.height as f32 * 0.70) as u16;
    let popup_area = centered_rect(popup_width, popup_height, area);

    frame.render_widget(Clear, popup_area);

    let mut lines: Vec<Line> = crate::shell::md_render::render_markdown(markdown);
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Enter/Esc to close",
        theme::muted(),
    )));

    let popup = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(theme::border_style())
                .title(" Detail ")
                .title_style(theme::heading()),
        )
        .wrap(Wrap { trim: false });

    frame.render_widget(popup, popup_area);
}

/// Side-effect-free decision for a single keypress in `run_loop`, extracted
/// so the quit/abort model can be unit tested without a real terminal or
/// orchestration. `focused`/`finished`/`detail_open` mirror the loop's local
/// state at the time the key was read.
///
/// Fix for #274: `q` (like Ctrl+C) must abort a running run **regardless of
/// focus** — previously `q` only aborted when `!focused`, so pressing
/// Ctrl+W to focus the Workroom silently disabled the `q` abort, leaving
/// Ctrl+C as the only way out (and the CLI subprocess would then orphan
/// unless `providers::cli` also sets `kill_on_drop`, see that module).
/// Once `finished`, `q`/Esc keep their unrelated meaning: dismiss the held
/// final frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyAction {
    /// Abort the running orchestration (`handle.abort()`) and exit the loop.
    Abort,
    /// Exit the loop without aborting (only valid once `finished`).
    Close,
    ToggleFocus,
    SelectPrev,
    SelectNext,
    OpenDetail,
    CloseDetail,
    None,
}

fn key_action(
    code: KeyCode,
    modifiers: KeyModifiers,
    focused: bool,
    finished: bool,
    detail_open: bool,
) -> KeyAction {
    // Ctrl+C aborts anytime, even with the detail popup open.
    if code == KeyCode::Char('c') && modifiers.contains(KeyModifiers::CONTROL) {
        return KeyAction::Abort;
    }

    if detail_open {
        return if matches!(code, KeyCode::Enter | KeyCode::Esc) {
            KeyAction::CloseDetail
        } else {
            KeyAction::None
        };
    }

    match code {
        KeyCode::Char('w') if modifiers.contains(KeyModifiers::CONTROL) => KeyAction::ToggleFocus,
        KeyCode::Up | KeyCode::Char('k') if focused => KeyAction::SelectPrev,
        KeyCode::Down | KeyCode::Char('j') if focused => KeyAction::SelectNext,
        KeyCode::Enter if focused => KeyAction::OpenDetail,
        KeyCode::Char('q') | KeyCode::Esc if finished => KeyAction::Close,
        // `q` aborts a running run regardless of focus (#274) — deliberately
        // not gated on `!focused` anymore.
        KeyCode::Char('q') => KeyAction::Abort,
        _ => KeyAction::None,
    }
}

/// Drain events, redraw, and poll input until the orchestration finishes and
/// the user dismisses the final frame (or aborts early). Returns the final
/// `RunEvent::Result` content (if any) for the caller to print once the
/// terminal is restored.
async fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    workroom: &mut Workroom,
    rx: &mut UnboundedReceiver<RunEvent>,
    handle: tokio::task::JoinHandle<anyhow::Result<()>>,
) -> anyhow::Result<Option<String>> {
    let mut final_content: Option<String> = None;
    // Flipped once the orchestration has finished and the channel is fully
    // drained. From that point the loop no longer auto-exits: it keeps
    // rendering the final Workroom frame (with a dismissal hint) and waits
    // for the user to press q/Esc — otherwise fast providers make the
    // workroom flash and disappear before it can be seen.
    let mut finished = false;
    // Markdown for the drill-down detail popup (Enter on a selected agent),
    // built from `Workroom::selected_detail_markdown()`. `Some` while the
    // popup overlay is open.
    let mut detail: Option<String> = None;
    loop {
        // Drain all pending events.
        while let Ok(ev) = rx.try_recv() {
            if let RunEvent::Result { content, .. } = &ev {
                final_content = Some(content.clone());
            }
            workroom.on_run_event_at(&ev, Instant::now());
        }

        if !finished && handle.is_finished() && rx.is_empty() {
            finished = true;
            workroom.set_completed(true);
        }

        workroom.tick();
        terminal.draw(|f| {
            let area = f.area();
            // Clear the whole frame first so nothing from a previous
            // (larger) draw lingers around the centered panel.
            f.render_widget(Clear, area);

            let width = WORKROOM_WIDTH
                .min(area.width)
                .max(WORKROOM_WIDTH_MIN.min(area.width));
            let needed_height = (workroom.agent_count() as u16)
                .saturating_mul(WORKROOM_HEIGHT_PER_AGENT)
                .saturating_add(WORKROOM_HEIGHT_BASE);
            let height = needed_height.min(area.height);
            let workroom_area = centered_rect(width, height, area);
            workroom.render(f, workroom_area);

            if let Some(md) = &detail {
                render_detail_popup(f, area, md);
            }
        })?;

        // Input: decision is delegated to the pure `key_action` helper (see
        // its doc comment for the quit/abort model) — this loop only
        // performs the resulting side effects.
        if event::poll(Duration::from_millis(80))?
            && let Event::Key(k) = event::read()?
            && k.kind == KeyEventKind::Press
        {
            match key_action(
                k.code,
                k.modifiers,
                workroom.is_focused(),
                finished,
                detail.is_some(),
            ) {
                KeyAction::Abort => {
                    handle.abort();
                    break;
                }
                KeyAction::Close => break,
                KeyAction::ToggleFocus => workroom.set_focused(!workroom.is_focused()),
                KeyAction::SelectPrev => workroom.select_prev(),
                KeyAction::SelectNext => workroom.select_next(),
                KeyAction::OpenDetail => detail = workroom.selected_detail_markdown(),
                KeyAction::CloseDetail => detail = None,
                KeyAction::None => {}
            }
        }
    }

    // Propagate the orchestration's result / return final content for the
    // caller to print after restoring the terminal.
    let outcome = handle.await;
    match outcome {
        Ok(Ok(())) => Ok(final_content),
        Ok(Err(e)) => Err(e),
        Err(join_err) if join_err.is_cancelled() => Ok(None), // Ctrl+C abort: nothing to print
        Err(join_err) => Err(anyhow::anyhow!(join_err)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::events::{EventSink, RunEvent};
    use crate::shell::workroom::Workroom;
    use std::time::Instant;

    // ── key_action: the #274 quit/abort decision table ──

    const NONE: KeyModifiers = KeyModifiers::NONE;
    const CTRL: KeyModifiers = KeyModifiers::CONTROL;

    #[test]
    fn q_aborts_a_running_run_when_unfocused() {
        assert_eq!(
            key_action(KeyCode::Char('q'), NONE, false, false, false),
            KeyAction::Abort
        );
    }

    #[test]
    fn q_aborts_a_running_run_when_focused() {
        // Regression for #274: previously `q` only aborted when unfocused,
        // so Ctrl+W (focus toggle) silently disabled the `q` abort and left
        // Ctrl+C as the only way to quit a live run.
        assert_eq!(
            key_action(KeyCode::Char('q'), NONE, true, false, false),
            KeyAction::Abort
        );
    }

    #[test]
    fn ctrl_c_aborts_regardless_of_focus_or_detail_popup() {
        for focused in [false, true] {
            for detail_open in [false, true] {
                assert_eq!(
                    key_action(KeyCode::Char('c'), CTRL, focused, false, detail_open),
                    KeyAction::Abort,
                    "focused={focused} detail_open={detail_open}"
                );
            }
        }
    }

    #[test]
    fn q_and_esc_dismiss_instead_of_abort_once_finished() {
        assert_eq!(
            key_action(KeyCode::Char('q'), NONE, true, true, false),
            KeyAction::Close
        );
        assert_eq!(
            key_action(KeyCode::Char('q'), NONE, false, true, false),
            KeyAction::Close
        );
        assert_eq!(
            key_action(KeyCode::Esc, NONE, true, true, false),
            KeyAction::Close
        );
    }

    #[test]
    fn esc_does_not_abort_a_running_run() {
        // Esc has no abort meaning while running (only q/Ctrl+C do); it's a
        // no-op here so it doesn't fall through to anything destructive.
        assert_eq!(
            key_action(KeyCode::Esc, NONE, false, false, false),
            KeyAction::None
        );
    }

    #[test]
    fn ctrl_w_toggles_focus_in_either_direction() {
        assert_eq!(
            key_action(KeyCode::Char('w'), CTRL, false, false, false),
            KeyAction::ToggleFocus
        );
        assert_eq!(
            key_action(KeyCode::Char('w'), CTRL, true, false, false),
            KeyAction::ToggleFocus
        );
    }

    #[test]
    fn jk_select_only_when_focused() {
        assert_eq!(
            key_action(KeyCode::Char('j'), NONE, true, false, false),
            KeyAction::SelectNext
        );
        assert_eq!(
            key_action(KeyCode::Char('k'), NONE, true, false, false),
            KeyAction::SelectPrev
        );
        assert_eq!(
            key_action(KeyCode::Down, NONE, true, false, false),
            KeyAction::SelectNext
        );
        assert_eq!(
            key_action(KeyCode::Up, NONE, true, false, false),
            KeyAction::SelectPrev
        );
        // Unfocused: j/k are not select shortcuts (and not the `q` abort key
        // either), so they're a no-op.
        assert_eq!(
            key_action(KeyCode::Char('j'), NONE, false, false, false),
            KeyAction::None
        );
        assert_eq!(
            key_action(KeyCode::Char('k'), NONE, false, false, false),
            KeyAction::None
        );
    }

    #[test]
    fn enter_opens_detail_only_when_focused_and_no_popup_open() {
        assert_eq!(
            key_action(KeyCode::Enter, NONE, true, false, false),
            KeyAction::OpenDetail
        );
        assert_eq!(
            key_action(KeyCode::Enter, NONE, false, false, false),
            KeyAction::None
        );
    }

    #[test]
    fn enter_or_esc_close_an_open_detail_popup() {
        assert_eq!(
            key_action(KeyCode::Enter, NONE, true, false, true),
            KeyAction::CloseDetail
        );
        assert_eq!(
            key_action(KeyCode::Esc, NONE, true, false, true),
            KeyAction::CloseDetail
        );
        // Any other key with the popup open is a no-op — it doesn't leak
        // through to select/abort while the popup is up.
        assert_eq!(
            key_action(KeyCode::Char('j'), NONE, true, false, true),
            KeyAction::None
        );
        assert_eq!(
            key_action(KeyCode::Char('q'), NONE, true, false, true),
            KeyAction::None
        );
    }

    #[test]
    fn sink_forwards_events_to_projection() {
        let (sink, mut rx) = WorkroomSink::new();
        sink.emit(&RunEvent::RunStart {
            run_id: "r1".into(),
            v: 1,
            agents: vec!["a".into(), "b".into()],
            prov: "f".into(),
            model: "m".into(),
            in_chars: 0,
        });
        sink.emit(&RunEvent::AgentStart {
            agent: "a".into(),
            prov: "f".into(),
            model: "m".into(),
        });
        sink.emit(&RunEvent::AgentEnd {
            agent: "a".into(),
            tin: 0,
            tout: 0,
            cost: 0.0,
            content: "hi".into(),
        });
        drop(sink);

        let mut wr = Workroom::new();
        let now = Instant::now();
        while let Ok(ev) = rx.try_recv() {
            wr.on_run_event_at(&ev, now);
        }
        let agents = wr.agents_for_test();
        assert_eq!(agents.len(), 2);
        assert_eq!(
            agents.iter().find(|a| a.name == "a").unwrap().state,
            crate::shell::workroom::AgentState::Done
        );
    }
}
