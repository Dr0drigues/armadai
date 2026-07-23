#![cfg(feature = "tui")]

use std::io;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use crate::core::events::{EventSink, RunEvent};
use crate::core::orchestration::OrchestrationPattern;
use crate::shell::workroom::Workroom;

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
}

/// Run an orchestration (`run`) while showing a live Workroom TUI fed by its
/// event stream. Restores the terminal on exit (including on error or panic),
/// and returns the final answer (if the run produced one) for the caller to
/// print *after* the terminal has been restored.
pub async fn run_orchestration_tui<F>(
    run: impl FnOnce(Arc<dyn EventSink>) -> F,
    config_yaml: Option<String>,
    explicit_pattern: Option<OrchestrationPattern>,
) -> anyhow::Result<Option<String>>
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

    // Always restore the terminal, even if the loop errored.
    restore_terminal();

    render_result
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
        terminal.draw(|f| workroom.render(f, f.area()))?;

        // Input: Ctrl+W focus/drill-down (already implemented, and still
        // active during the post-completion hold), Ctrl+C aborts anytime,
        // q/Esc dismiss the held final frame once `finished`.
        if event::poll(Duration::from_millis(80))?
            && let Event::Key(k) = event::read()?
            && k.kind == KeyEventKind::Press
        {
            match k.code {
                KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                    handle.abort();
                    break;
                }
                KeyCode::Char('w') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                    workroom.set_focused(!workroom.is_focused());
                }
                KeyCode::Up | KeyCode::Char('k') if workroom.is_focused() => workroom.select_prev(),
                KeyCode::Down | KeyCode::Char('j') if workroom.is_focused() => {
                    workroom.select_next()
                }
                KeyCode::Char('q') | KeyCode::Esc if finished => break,
                KeyCode::Char('q') if !workroom.is_focused() => {
                    handle.abort();
                    break;
                }
                _ => {}
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

    #[test]
    fn sink_forwards_events_to_projection() {
        let (sink, mut rx) = WorkroomSink::new();
        sink.emit(&RunEvent::RunStart {
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
