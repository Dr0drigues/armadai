use async_trait::async_trait;
use std::time::{Duration, Instant};
use tokio::process::Command;

use armadai_core::provider::*;

/// Absolute backstop on top of the per-line inactivity timeout (#270 review
/// F4): inactivity is the right thing to bound (see `CliProvider::complete`),
/// but it cannot be the *only* bound — a subprocess that keeps producing
/// some output forever (a heartbeat that never converges) would otherwise
/// never be killed. This must never trip on any legitimate call, including
/// a single `claude -p` invocation that itself drives many native
/// Task-tool delegations in one process (a whole hierarchical session can
/// be ONE `CliProvider::complete()` call) — so it is deliberately large: 2
/// hours comfortably exceeds every turn duration observed on this project
/// (single agentic turns ~500-600s) with room for several chained turns.
/// It is a safety net, not a tuned control: if genuinely multi-hour
/// sessions become routine, this should become its own configurable field
/// instead of a larger constant.
///
/// `pub` so `armadai/src/cli/run.rs` can assert against it at compile time
/// (see the `const _: () = assert!(...)` below and its sibling in
/// `run.rs`) — nothing enforced the relationship between the three
/// timeout constants spread across two crates (this one,
/// `factory::DEFAULT_TIMEOUT_SECS`, `run.rs::ORCHESTRATED_DEFAULT_TIMEOUT_SECS`)
/// otherwise: if an inactivity default ever grew past this ceiling, every
/// timeout in the product would be misreported as "absolute ceiling".
pub const ABSOLUTE_CEILING_SECS: u64 = 2 * 60 * 60;

// This ceiling must stay strictly above the non-orchestrated inactivity
// default, or every direct-run timeout would misreport as "absolute
// ceiling" instead of "inactivity" (see `next_step_timeout`'s
// `ceiling_bound` flag). The orchestrated default (600s, in a different
// crate) is asserted against separately in `armadai/src/cli/run.rs`.
const _: () = assert!(
    ABSOLUTE_CEILING_SECS > crate::factory::DEFAULT_TIMEOUT_SECS,
    "ABSOLUTE_CEILING_SECS must stay above factory::DEFAULT_TIMEOUT_SECS"
);

/// Decide how long the next single read may wait, given how long the call
/// has run so far (`elapsed`), the per-line inactivity ceiling
/// (`inactivity_timeout`), and the absolute backstop (`absolute_ceiling`).
///
/// Returns `None` once `absolute_ceiling` itself has already been reached
/// (the caller must stop, regardless of activity). Otherwise returns the
/// duration the next read may wait, and whether THIS step is bounded by
/// the absolute ceiling rather than by genuine inactivity — the caller
/// uses that flag to report an accurate reason if the step times out.
///
/// Pure and sync so the scheduling math is unit-testable without any real
/// waiting (mirrors this codebase's existing `orchestrated_agent_timeout_secs`
/// pattern of keeping timeout arithmetic in a plain, fast-testable fn).
fn next_step_timeout(
    elapsed: Duration,
    inactivity_timeout: Duration,
    absolute_ceiling: Duration,
) -> Option<(Duration, bool)> {
    if elapsed >= absolute_ceiling {
        return None;
    }
    let remaining_to_ceiling = absolute_ceiling - elapsed;
    let ceiling_bound = remaining_to_ceiling < inactivity_timeout;
    Some((inactivity_timeout.min(remaining_to_ceiling), ceiling_bound))
}

/// Generic CLI provider that spawns any configured command.
pub struct CliProvider {
    pub command: String,
    pub args: Vec<String>,
    pub timeout_secs: u64,
    /// Absolute backstop, in seconds. Always `ABSOLUTE_CEILING_SECS` in
    /// production (`new()` sets it; there is no public way to change it) —
    /// this only exists as a field, instead of `complete`/`stream` reading
    /// the module constant directly, so a test can shrink it via
    /// `with_absolute_ceiling_secs` and prove the ceiling is genuinely
    /// consulted end-to-end (not just correct in isolation as pure math —
    /// see `next_step_timeout`'s own unit tests, and the review that found
    /// deleting the ceiling check from both loops left the suite green).
    absolute_ceiling_secs: u64,
}

impl CliProvider {
    pub fn new(command: String, args: Vec<String>, timeout_secs: u64) -> Self {
        Self {
            command,
            args,
            timeout_secs,
            absolute_ceiling_secs: ABSOLUTE_CEILING_SECS,
        }
    }

    /// Test-only: override the absolute ceiling so a test can observe it
    /// actually firing without waiting 2 real hours. Not exposed outside
    /// `#[cfg(test)]` — production code always uses `ABSOLUTE_CEILING_SECS`.
    #[cfg(test)]
    fn with_absolute_ceiling_secs(mut self, secs: u64) -> Self {
        self.absolute_ceiling_secs = secs;
        self
    }

    /// Compose the input string sent to the CLI command from the request's
    /// system prompt and last user message. The system prompt is the only
    /// channel carrying agent persona and delegation instructions (see
    /// `traits::CompletionRequest`); CLI providers have no separate channel
    /// for it, so it is prefixed onto the task, clearly delimited.
    ///
    /// When `system_prompt` is empty, the message is returned unchanged
    /// (no prefix, no tags) to preserve existing behavior exactly.
    fn compose_input(&self, request: &CompletionRequest) -> String {
        let msg = request
            .messages
            .last()
            .map(|m| m.content.as_str())
            .unwrap_or("");

        if request.system_prompt.is_empty() {
            msg.to_string()
        } else {
            format!("<system>\n{}\n</system>\n\n{}", request.system_prompt, msg)
        }
    }

    fn build_command(&self, input: &str) -> Command {
        let mut cmd = Command::new(&self.command);
        for arg in &self.args {
            cmd.arg(arg);
        }
        cmd.arg(input);
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        // Ensure the child is SIGKILLed if the `Child`/future is dropped
        // without being awaited to completion — e.g. when a caller aborts
        // the `tokio::spawn`'d task driving `complete()`/`stream()` (see
        // `shell::run_view::run_loop`'s Ctrl+C/`q` handling). Without this,
        // an aborted run leaves the CLI subprocess (e.g. `claude`) running
        // and orphaned, still writing to the inherited stdout/stderr after
        // the TUI has torn down the alternate screen — looking like "the
        // run keeps going" even though ArmadAI itself has exited (#274).
        cmd.kill_on_drop(true);
        cmd
    }

    fn parse_json_stdout(&self, raw: &str) -> CompletionResponse {
        use crate::json_runner::{StreamEvent, parse_stream_event};

        let mut content = String::new();
        let mut result = None;
        for line in raw.lines() {
            match parse_stream_event(&self.command, line) {
                StreamEvent::Delta(t) | StreamEvent::Message(t) => content.push_str(&t),
                StreamEvent::Result(resp) => result = Some(resp),
                _ => {}
            }
        }

        match result {
            Some(resp) => CompletionResponse {
                content,
                model: resp.model.unwrap_or_else(|| self.command.clone()),
                tokens_in: resp.tokens_in.unwrap_or(0) as u32,
                tokens_out: resp.tokens_out.unwrap_or(0) as u32,
                cost: resp.cost_usd.unwrap_or(0.0),
            },
            None => CompletionResponse {
                content: raw.to_string(),
                model: self.command.clone(),
                tokens_in: 0,
                tokens_out: 0,
                cost: 0.0,
            },
        }
    }
}

#[async_trait]
impl Provider for CliProvider {
    async fn complete(&self, request: CompletionRequest) -> anyhow::Result<CompletionResponse> {
        let input = self.compose_input(&request);

        // Run with `self.args` verbatim (the factory already selected the right
        // args: canonical stream-json args for a default JSON-capable CLI, or
        // the agent's explicit args). Then parse stdout opportunistically:
        // `parse_json_stdout` extracts content + cost/tokens from JSONL events
        // when present, and falls back to raw stdout (zeroed metrics) otherwise.
        let mut child = self.build_command(&input).spawn()?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("Failed to capture stdout"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| anyhow::anyhow!("Failed to capture stderr"))?;

        // Drain stderr concurrently. Reading stdout to EOF *before* `wait()`ing
        // (below) is required to avoid the classic pipe-deadlock (a chatty
        // child blocks on a full stdout pipe while we're not reading it), and
        // draining stderr on its own task means a chatty stderr can't cause
        // the same deadlock while we wait on stdout lines. Byte-oriented
        // (`read_until`, not `.lines()`): a single non-UTF-8 byte on stderr
        // must not abort the drain, it must lossily substitute like the rest
        // of this method (#270 review F2).
        let mut stderr_task = tokio::spawn(async move {
            let mut reader = tokio::io::BufReader::new(stderr);
            let mut out = Vec::new();
            let mut chunk = Vec::new();
            loop {
                chunk.clear();
                match tokio::io::AsyncBufReadExt::read_until(&mut reader, b'\n', &mut chunk).await {
                    Ok(0) | Err(_) => break,
                    Ok(_) => out.extend_from_slice(&chunk),
                }
            }
            String::from_utf8_lossy(&out).into_owned()
        });

        let mut reader = tokio::io::BufReader::new(stdout);
        let timeout = Duration::from_secs(self.timeout_secs);
        let absolute_ceiling = Duration::from_secs(self.absolute_ceiling_secs);
        let start = Instant::now();

        // Inactivity timeout (#270): rearmed on every line read from stdout,
        // rather than a single deadline covering the whole call. Any line —
        // a delegation (`tool_use`) event, a token delta, an init/result
        // event, even a line that fails to parse as JSON — counts as
        // activity: it proves the subprocess is still producing observable
        // output, which is the only signal available here that it isn't
        // hung (this works uniformly for JSON-streaming CLIs and plain-text
        // ones like `aider`, without coupling to `json_runner`'s
        // per-provider event parsing). A subprocess that goes fully silent
        // for `timeout_secs` is killed; one that keeps streaming survives
        // past what used to be a hard wall-clock ceiling, up to the
        // `ABSOLUTE_CEILING_SECS` backstop.
        //
        // Reads are byte-oriented (`read_until`, not `.lines()`), so a
        // single non-UTF-8 byte lossily substitutes (matching the pre-#270
        // `String::from_utf8_lossy(&output.stdout)` behavior) instead of
        // aborting the whole call — `provider: cli` is documented for
        // arbitrary scripts, and a stray byte from one must not be fatal.
        //
        // Design choice (#270 review round 2, item 4): every timeout below
        // in THIS loop discards `raw` and errors, even though some lines
        // may already have been collected — unlike the post-EOF
        // `child.wait()` timeout further down, which returns `raw` as a
        // success. The axis that decides it is not "was this a timeout",
        // it's "did the subprocess ever tell us it was done": stdout
        // reaching EOF is the subprocess's own unambiguous completion
        // signal, so returning `raw` there is trusting the subprocess, not
        // guessing. A timeout firing HERE means we gave up before EOF —
        // `raw` is provably an interrupted, unfinished response (most
        // often missing the terminal `result` event `parse_json_stdout`
        // looks for). Returning that as `Ok` would make the timeout
        // decorative in a new way: instead of visibly failing, it would
        // quietly hand the caller truncated content indistinguishable from
        // a real answer. Erroring here — while still killing the process
        // and freeing resources exactly like a successful path — is what
        // keeps a hang or a runaway heartbeat visible to the caller.
        let mut raw = String::new();
        let mut line_buf: Vec<u8> = Vec::new();
        loop {
            let Some((step_timeout, ceiling_bound)) =
                next_step_timeout(start.elapsed(), timeout, absolute_ceiling)
            else {
                let _ = child.start_kill();
                stderr_task.abort();
                anyhow::bail!(
                    "CLI command exceeded the absolute {}s ceiling despite ongoing activity",
                    absolute_ceiling.as_secs()
                );
            };

            line_buf.clear();
            let read = tokio::io::AsyncBufReadExt::read_until(&mut reader, b'\n', &mut line_buf);
            match tokio::time::timeout(step_timeout, read).await {
                Ok(Ok(0)) => break, // stdout closed: subprocess done writing
                Ok(Ok(_)) => raw.push_str(&String::from_utf8_lossy(&line_buf)),
                Ok(Err(e)) => {
                    let _ = child.start_kill();
                    stderr_task.abort();
                    return Err(e.into());
                }
                Err(_) if ceiling_bound => {
                    let _ = child.start_kill();
                    stderr_task.abort();
                    anyhow::bail!(
                        "CLI command exceeded the absolute {}s ceiling despite ongoing activity",
                        absolute_ceiling.as_secs()
                    );
                }
                Err(_) => {
                    let _ = child.start_kill();
                    stderr_task.abort();
                    anyhow::bail!(
                        "CLI command timed out after {}s of inactivity (no output)",
                        self.timeout_secs
                    );
                }
            }
        }

        // Bound the post-EOF awaits too (#270 review F1): a process — or an
        // orphaned descendant that inherited a pipe fd (e.g. an MCP server
        // `claude -p` spawned) — that doesn't exit/close promptly once
        // stdout is fully read must not hang the whole call. `child` is
        // dropped on every return path below; `kill_on_drop` (set in
        // `build_command`) then SIGKILLs it and tokio's own orphan queue
        // reaps it in the background, so no explicit wait-for-exit is
        // needed here to avoid leaking a zombie.
        let status = match tokio::time::timeout(timeout, child.wait()).await {
            Ok(result) => result?,
            Err(_) => {
                // stdout already reached a clean EOF, so `raw` holds a
                // complete response (the common real case: `claude -p`
                // itself finished and flushed its `result` event; a
                // spawned MCP server subprocess is what's still holding a
                // pipe open). Prefer that known-good content over hanging
                // or discarding it — we cannot learn anything more here
                // without blocking indefinitely on a descendant we don't
                // control (a surviving grandchild is a separate, pre-
                // existing concern, not this call's to solve). Abort the
                // stderr drain too rather than leaving it detached forever
                // reading a pipe that may never EOF.
                stderr_task.abort();
                return Ok(self.parse_json_stdout(&raw));
            }
        };

        if !status.success() {
            // Poll `&mut stderr_task` (not `stderr_task` by value): moving
            // the `JoinHandle` into `tokio::time::timeout` would, on
            // timeout, just DROP it — which detaches the task rather than
            // aborting it (a `JoinHandle`'s `Drop` does not cancel the
            // task), leaking it and its stderr fd if a descendant is still
            // holding the pipe open. Keeping our own handle lets us call
            // `.abort()` explicitly on that path.
            let stderr_buf = match tokio::time::timeout(timeout, &mut stderr_task).await {
                Ok(Ok(buf)) => buf,
                Ok(Err(_)) => String::new(), // drain task panicked/was cancelled
                Err(_) => {
                    stderr_task.abort();
                    String::new()
                }
            };
            anyhow::bail!("CLI command failed ({status}): {stderr_buf}");
        }

        // Successful exit: `stderr_buf` was never needed. Abort the drain
        // task explicitly rather than leaving it detached — the child
        // already exited, so in the common case this is a no-op (the task
        // already finished on its own when the child's stderr fd closed),
        // but it still frees the task+fd promptly if some descendant is
        // holding the pipe open.
        stderr_task.abort();
        Ok(self.parse_json_stdout(&raw))
    }

    async fn stream(&self, request: CompletionRequest) -> anyhow::Result<TokenStream> {
        let input = self.compose_input(&request);

        let mut child = self.build_command(&input).spawn()?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("Failed to capture stdout"))?;

        let timeout_secs = self.timeout_secs;
        let absolute_ceiling_secs = self.absolute_ceiling_secs;
        let (tx, rx) = tokio::sync::mpsc::channel(64);

        tokio::spawn(async move {
            let mut reader = tokio::io::BufReader::new(stdout);
            let timeout = Duration::from_secs(timeout_secs);
            let absolute_ceiling = Duration::from_secs(absolute_ceiling_secs);
            let start = Instant::now();
            let mut line_buf: Vec<u8> = Vec::new();

            // Mirrors `complete()`: inactivity timeout rearmed on every
            // line (byte-oriented `read_until`, lossy on non-UTF-8, not
            // `.lines()` — #270 review F1/F2/F4), bounded overall by
            // `ABSOLUTE_CEILING_SECS` so a child that never goes silent
            // still ends eventually. `child` is dropped on every path out
            // of this task; `kill_on_drop` (set in `build_command`) then
            // handles the kill+reap, so no unbounded `child.wait()` is
            // needed here to avoid a zombie.
            loop {
                let Some((step_timeout, ceiling_bound)) =
                    next_step_timeout(start.elapsed(), timeout, absolute_ceiling)
                else {
                    let _ = child.start_kill();
                    let _ = tx
                        .send(Err(anyhow::anyhow!(
                            "CLI command exceeded the absolute {}s ceiling despite ongoing activity",
                            absolute_ceiling.as_secs()
                        )))
                        .await;
                    return;
                };

                line_buf.clear();
                let read =
                    tokio::io::AsyncBufReadExt::read_until(&mut reader, b'\n', &mut line_buf);
                match tokio::time::timeout(step_timeout, read).await {
                    Ok(Ok(0)) => break, // stdout closed: child done writing
                    Ok(Ok(_)) => {
                        // `read_until` keeps the delimiter; strip it (and a
                        // preceding `\r`) so a line matches what `.lines()`
                        // used to yield — callers expect terminator-free
                        // lines (e.g. `cli_stream_echo`, `json_runner`'s
                        // per-line JSONL parsing).
                        if line_buf.last() == Some(&b'\n') {
                            line_buf.pop();
                            if line_buf.last() == Some(&b'\r') {
                                line_buf.pop();
                            }
                        }
                        let line = String::from_utf8_lossy(&line_buf).into_owned();
                        if tx.send(Ok(line)).await.is_err() {
                            // Receiver dropped: nobody is listening any
                            // more (e.g. the caller aborted). Return
                            // immediately rather than run the post-EOF
                            // wait below — `child` drops here and
                            // `kill_on_drop` handles cleanup (#274).
                            return;
                        }
                    }
                    Ok(Err(e)) => {
                        // Already reported via `tx`; nothing more useful to
                        // learn from also waiting for an exit status here.
                        let _ = tx
                            .send(Err(anyhow::anyhow!("Failed to read CLI output: {e}")))
                            .await;
                        return;
                    }
                    Err(_) => {
                        let _ = child.start_kill();
                        let message = if ceiling_bound {
                            format!(
                                "CLI command exceeded the absolute {}s ceiling despite ongoing activity",
                                absolute_ceiling.as_secs()
                            )
                        } else {
                            format!(
                                "CLI command timed out after {timeout_secs}s of inactivity (no output)"
                            )
                        };
                        if let Err(e) = tx.send(Err(anyhow::anyhow!(message))).await {
                            tracing::debug!(
                                "Failed to send timeout error (receiver dropped): {:?}",
                                e
                            );
                        }
                        return;
                    }
                }
            }

            // Reached only via the normal-EOF `break` above. Bound the
            // post-EOF wait like `complete()` does (review round 2, item
            // 2): closing stdout does not mean the child has exited, and
            // `kill_on_drop` SIGKILLs it the instant `child` drops — doing
            // that immediately here would kill a child that closed stdout
            // but is still doing legitimate post-EOF work (a clean exit,
            // flushing a log), where the pre-#270 code let it finish via
            // an unbounded `child.wait()`. This also closes item 3: with
            // no exit-status check at all, a non-zero exit after
            // successful output was silently reported as a clean stream
            // end — no error item was ever sent.
            match tokio::time::timeout(timeout, child.wait()).await {
                Ok(Ok(status)) if !status.success() => {
                    let _ = tx
                        .send(Err(anyhow::anyhow!("CLI command failed ({status})")))
                        .await;
                }
                Ok(Ok(_)) => {} // clean exit: every line was already forwarded
                Ok(Err(e)) => {
                    let _ = tx
                        .send(Err(anyhow::anyhow!("Failed to wait for CLI command: {e}")))
                        .await;
                }
                Err(_) => {
                    // Child closed stdout but did not exit within one more
                    // inactivity window (e.g. an orphaned descendant is
                    // still holding a pipe open, per `complete()`'s F1
                    // fix). We already forwarded every line; `child` drops
                    // here and `kill_on_drop` reaps it in the background
                    // rather than this task hanging on a descendant it
                    // doesn't control.
                }
            }
        });

        Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }

    fn metadata(&self) -> ProviderMetadata {
        ProviderMetadata {
            name: format!("cli:{}", self.command),
            models: vec![self.command.clone()],
            supports_streaming: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run `fut` while holding `ENV_MUTEX`, serialising the child-process spawn's
    /// read of `environ` against tests that mutate it via `std::env::set_var`
    /// (otherwise a data race — the reason `set_var` is `unsafe`). Holding the
    /// guard across `.await` is safe here: no other async task contends for
    /// `ENV_MUTEX`, so there is no deadlock risk.
    #[allow(clippy::await_holding_lock)]
    async fn with_env_lock<T>(fut: impl std::future::Future<Output = T>) -> T {
        let _guard = armadai_core::config::ENV_MUTEX.lock().unwrap();
        fut.await
    }

    fn echo_request(text: &str) -> CompletionRequest {
        CompletionRequest {
            model: "echo".to_string(),
            system_prompt: String::new(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: text.to_string(),
            }],
            temperature: 0.0,
            max_tokens: None,
        }
    }

    #[tokio::test]
    async fn cli_complete_echo() {
        let response = with_env_lock(async {
            let provider = CliProvider::new("echo".to_string(), vec![], 10);
            provider
                .complete(echo_request("hello world"))
                .await
                .unwrap()
        })
        .await;
        assert_eq!(response.content.trim(), "hello world");
        assert_eq!(response.tokens_in, 0);
        assert_eq!(response.cost, 0.0);
    }

    #[tokio::test]
    async fn cli_complete_with_args() {
        let response = with_env_lock(async {
            let provider = CliProvider::new("echo".to_string(), vec!["prefix".to_string()], 10);
            provider.complete(echo_request("test")).await.unwrap()
        })
        .await;
        assert_eq!(response.content.trim(), "prefix test");
    }

    #[tokio::test]
    async fn cli_stream_echo() {
        use tokio_stream::StreamExt;

        let output = with_env_lock(async {
            let provider = CliProvider::new("echo".to_string(), vec![], 10);
            let mut stream = provider.stream(echo_request("stream test")).await.unwrap();

            let mut output = Vec::new();
            while let Some(line) = stream.next().await {
                output.push(line.unwrap());
            }
            output
        })
        .await;
        assert_eq!(output, vec!["stream test"]);
    }

    #[tokio::test]
    async fn cli_complete_failure() {
        let result = with_env_lock(async {
            let provider = CliProvider::new("false".to_string(), vec![], 10);
            provider.complete(echo_request("")).await
        })
        .await;
        assert!(result.is_err());
    }

    // ── kill_on_drop (fix for #274: abort must not orphan the subprocess) ──

    /// Best-effort liveness check via `kill -0 <pid>` (POSIX; both the CI
    /// Linux runners and macOS dev machines have `kill`). A zombie still
    /// occupies the process table until its parent reaps it, so this
    /// correctly reports "alive" until the OS/tokio's orphan-queue reaper
    /// has actually collected it.
    fn process_alive(pid: i32) -> bool {
        std::process::Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    #[tokio::test]
    async fn dropping_child_kills_orphaned_process_via_kill_on_drop() {
        // Regression test for #274: the Workroom's `run_loop` aborts a
        // running orchestration by dropping the `tokio::spawn`'d task
        // future. Without `kill_on_drop(true)` on the `Command` built in
        // `build_command`, dropping the future (and its owned `Child`)
        // leaves the CLI subprocess (e.g. `claude`) running and orphaned —
        // it keeps writing to the inherited stdout/stderr after the TUI has
        // torn down, which looks like "the run keeps going" even though
        // ArmadAI has exited. This asserts the child is actually gone
        // shortly after the `Child` handle is dropped, unawaited.
        let provider = CliProvider::new("sh".to_string(), vec!["-c".to_string()], 10);
        let mut cmd = provider.build_command("sleep 30");
        let child = cmd.spawn().expect("failed to spawn `sh -c sleep 30`");
        let pid = child.id().expect("spawned child should have a pid") as i32;

        assert!(
            process_alive(pid),
            "sanity check: child {pid} should be running right after spawn"
        );

        // Simulate `handle.abort()`: drop the future (and the `Child` it
        // owns) without ever calling `.wait()`/`.output()` on it.
        drop(child);

        // `kill_on_drop`'s `Drop` impl sends SIGKILL synchronously, but the
        // process table entry is only cleared once tokio's orphan-queue
        // reaper collects it (driven by SIGCHLD) — poll briefly instead of
        // asserting instantaneously.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while std::time::Instant::now() < deadline && process_alive(pid) {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }

        assert!(
            !process_alive(pid),
            "child process {pid} should have been killed+reaped when the Child was \
             dropped (kill_on_drop); a still-running process here reproduces the #274 orphan"
        );
    }

    #[tokio::test]
    async fn cli_complete_timeout() {
        let result = with_env_lock(async {
            let provider = CliProvider::new("sleep".to_string(), vec![], 1);
            provider.complete(echo_request("30")).await
        })
        .await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("timed out"), "Error was: {err}");
    }

    // ── Inactivity timeout, rearmed per line (#270) ──
    //
    // The pre-fix implementation wrapped ONE `tokio::time::timeout` around
    // the entire subprocess call, so the ceiling measured total call
    // duration. That kills a hierarchical run mid-progress: each delegated
    // `claude -p` turn is itself long-running and agentic, and a
    // multi-delegation coordinator run legitimately exceeds any fixed
    // wall-clock ceiling while still making steady progress. The two tests
    // below pin the replacement contract: the ceiling must measure the gap
    // between successive lines of subprocess output, not the call's total
    // duration.

    /// A subprocess whose individual gaps between lines never exceed the
    /// ceiling must survive even once its TOTAL runtime exceeds it.
    ///
    /// Mutation this catches: reverting to a single `tokio::time::timeout`
    /// around the whole read loop (the pre-fix shape) instead of one
    /// rearmed on every line — that shape kills this exact case, since the
    /// process's total runtime (~1.25s over 5 ticks of 0.25s) exceeds the
    /// 1s ceiling even though no single gap does. It also catches a
    /// mutation that reads the whole stdout in one shot before checking
    /// the deadline (e.g. reverting to `cmd.output()`), which would
    /// observe no intermediate activity at all and time out the same way.
    ///
    /// The elapsed-time assertion is load-bearing, not decorative: without
    /// it, degenerating the script to `sleep 0` would still pass in ~10ms
    /// while proving nothing about surviving PAST the old ceiling — the
    /// exact premise in this test's name (#270 review F6). A 4x per-gap
    /// margin (1s ceiling / 0.25s ticks) tolerates scheduling jitter when
    /// the full suite runs many real subprocess-spawning tests in
    /// parallel, kept modest (not wider) so this test doesn't add to the
    /// affected binary's runtime (#270 review round 2: "prefer
    /// patched-down constants over real sleeps").
    #[tokio::test]
    async fn steady_stream_survives_past_the_old_static_ceiling() {
        let old_static_ceiling = std::time::Duration::from_secs(1);
        // `start` MUST be taken inside `with_env_lock`, after the global
        // `ENV_MUTEX` is actually held — not before. `with_env_lock` blocks
        // on that mutex, shared by every test in this module that spawns a
        // real subprocess; under a loaded parallel `cargo test`, queueing
        // for it can itself take seconds. Timing from outside the lock
        // measures queue-wait + real work, not the mechanism under test —
        // which is exactly why this assertion could pass on a no-sleep
        // script under load despite proving nothing (#270 review round 3).
        let (result, elapsed) = with_env_lock(async {
            let start = std::time::Instant::now();
            let provider = CliProvider::new("sh".to_string(), vec!["-c".to_string()], 1);
            let result = provider
                .complete(echo_request(
                    "for i in 1 2 3 4 5; do echo tick; sleep 0.25; done",
                ))
                .await;
            (result, start.elapsed())
        })
        .await;

        let response = result.expect("a steadily-ticking subprocess must not time out");
        assert_eq!(
            response.content.matches("tick").count(),
            5,
            "expected all 5 ticks to have been read before the process exited, got: {}",
            response.content
        );
        assert!(
            elapsed > old_static_ceiling,
            "test must actually run longer than the old 1s ceiling to prove the premise \
             (a degenerate sleep-0 script would pass this test without exercising the \
             reset-on-activity behavior at all): elapsed {elapsed:?}"
        );
    }

    /// A subprocess producing NOTHING on stdout must be killed once
    /// `timeout_secs` of silence elapses — it must die near that ceiling,
    /// not run toward its much longer natural completion (or hang forever
    /// if the mechanism is broken).
    ///
    /// Mutation this catches: disabling/removing the inactivity check, or
    /// rearming it on something other than actual stdout activity (e.g. an
    /// unconditional tick) — either would let this call run well past the
    /// 1s ceiling instead of erroring within a couple of seconds of it, so
    /// the UPPER elapsed-time bound below would fail. The LOWER bound
    /// catches the opposite mutation — e.g. a stray unit conversion turning
    /// the configured 1s into 1ms (`Duration::from_millis(self.timeout_secs)`
    /// instead of `from_secs`) — which the upper bound alone does not:
    /// dying near-instantly still satisfies "died before Ns" (#270 review
    /// F6).
    #[tokio::test]
    async fn silent_subprocess_dies_near_the_inactivity_ceiling_not_later() {
        // `start` inside `with_env_lock`, not outside — see the identical
        // note on `steady_stream_survives_past_the_old_static_ceiling`
        // (#270 review round 3: queue-wait for the shared `ENV_MUTEX` must
        // not be counted as part of the measured duration).
        let (result, elapsed) = with_env_lock(async {
            let start = std::time::Instant::now();
            let provider = CliProvider::new("sleep".to_string(), vec![], 1);
            let result = provider.complete(echo_request("60")).await;
            (result, start.elapsed())
        })
        .await;

        let err = result
            .expect_err("a silent subprocess must time out")
            .to_string();
        assert!(err.contains("timed out"), "error was: {err}");
        // Lower bound: must have actually waited close to the configured 1s
        // ceiling, not fired near-instantly (catches a unit-conversion-style
        // mutation that shrinks the effective ceiling by orders of
        // magnitude). Upper bound: tight now that `start` is measured
        // inside the lock (well under the 60s sleep, but no longer needs
        // to absorb queue-wait time from other tests contending for
        // `ENV_MUTEX`).
        assert!(
            elapsed >= std::time::Duration::from_millis(500),
            "should wait close to the configured 1s ceiling before dying, not fire near-instantly: {elapsed:?}"
        );
        assert!(
            elapsed < std::time::Duration::from_secs(5),
            "should die near the 1s inactivity ceiling, not run toward the 60s sleep: {elapsed:?}"
        );
    }

    // ── Post-EOF phase must be bounded too (#270 review F1) ──
    //
    // The read loop above only covers reading stdout up to EOF. Two MORE
    // awaits follow it: `child.wait()` (for the exit status) and, on
    // failure, joining the stderr-drain task. Both were originally
    // unbounded, so a subprocess (or an orphaned descendant holding an
    // inherited pipe fd — e.g. an MCP server `claude -p` spawns) that
    // reaches stdout EOF but doesn't exit/close promptly would hang
    // `complete()` forever, reproducing #274's symptom even though the read
    // loop itself timed out correctly. Both tests below are direct
    // reconstructions of the review's own repro shapes.

    /// The direct child keeps running (sleeping) itself after redirecting
    /// its OWN stdout away — stdout hits EOF almost immediately, but the
    /// process doesn't exit for a long time afterward.
    ///
    /// Mutation this catches: reverting `child.wait()` after the read loop
    /// to an unbounded `.await` (no `tokio::time::timeout` around it) —
    /// this call would then block for the full 30s sleep instead of
    /// returning within roughly the 1s inactivity ceiling.
    #[tokio::test]
    async fn complete_does_not_hang_when_stdout_closes_but_the_child_keeps_running() {
        // `start` inside `with_env_lock`, not outside (#270 review round 3
        // / round 4): the shared `ENV_MUTEX` is contended by every
        // subprocess-spawning test in this module, and queue-wait for it
        // must not be counted toward "did this call hang". The margin here
        // is coarse (correct ~1s vs a hang toward 30s) so the bound itself
        // doesn't need retuning — only the measurement point did.
        let (result, elapsed) = with_env_lock(async {
            let start = std::time::Instant::now();
            let provider = CliProvider::new("sh".to_string(), vec!["-c".to_string()], 1);
            let result = provider
                .complete(echo_request("echo hi; exec 1>/dev/null; sleep 30"))
                .await;
            (result, start.elapsed())
        })
        .await;

        assert!(
            elapsed < std::time::Duration::from_secs(10),
            "must not block toward the child's 30s post-EOF sleep: {elapsed:?}"
        );
        // Either outcome (a successful response built from the already-read
        // "hi", or a bounded timeout error) is acceptable — what matters is
        // that the call returns promptly. It must not hang.
        match result {
            Ok(response) => assert!(response.content.contains("hi")),
            Err(e) => {
                assert!(e.to_string().contains("timed out") || e.to_string().contains("ceiling"))
            }
        }
    }

    /// The direct child exits almost immediately after printing "hi", but
    /// backgrounds a detached grandchild that inherits the (unredirected)
    /// stderr pipe fd and keeps it open — so `stderr` never sees EOF, even
    /// though the process we spawned is long gone.
    ///
    /// Mutation this catches: reverting the success path to unconditionally
    /// join `stderr_task` (or joining it without a `tokio::time::timeout`)
    /// before returning — this call would then block for the full 30s the
    /// grandchild lives, instead of returning as soon as `child.wait()`
    /// resolves (near-instantly, since the direct child exits fast).
    #[tokio::test]
    async fn complete_does_not_hang_on_an_orphaned_descendant_holding_stderr_open() {
        // `start` inside `with_env_lock` — see the identical note on
        // `complete_does_not_hang_when_stdout_closes_but_the_child_keeps_running`.
        let (result, elapsed) = with_env_lock(async {
            let start = std::time::Instant::now();
            let provider = CliProvider::new("sh".to_string(), vec!["-c".to_string()], 1);
            let result = provider
                .complete(echo_request("(sleep 30 >/dev/null 0</dev/null &); echo hi"))
                .await;
            (result, start.elapsed())
        })
        .await;

        assert!(
            elapsed < std::time::Duration::from_secs(10),
            "must not block on the orphaned grandchild's 30s lifetime: {elapsed:?}"
        );
        let response = result.expect("the direct child exits successfully");
        assert!(response.content.contains("hi"));
    }

    // ── Lossy UTF-8 on stdout must not be fatal (#270 review F2) ──

    /// `provider: cli` is documented for arbitrary scripts; a single
    /// non-UTF-8 byte from one must lossily substitute (matching the
    /// pre-#270 `String::from_utf8_lossy(&output.stdout)` behavior), not
    /// abort the whole call.
    ///
    /// Mutation this catches: reverting the byte-oriented `read_until`
    /// reader back to `.lines()` (which requires valid UTF-8 and errors
    /// out on a bad byte) — this call would then fail instead of returning
    /// a response containing the lossy replacement character.
    #[tokio::test]
    async fn non_utf8_byte_on_stdout_is_lossily_substituted_not_fatal() {
        let result = with_env_lock(async {
            // printf writes a lone continuation byte (0x80), invalid on its
            // own in UTF-8, between two valid words.
            let provider = CliProvider::new("sh".to_string(), vec!["-c".to_string()], 10);
            provider
                .complete(echo_request(r"printf 'hello \200world\n'"))
                .await
        })
        .await;

        let response = result.expect("a stray non-UTF-8 byte must not fail the call");
        assert!(
            response.content.contains("hello") && response.content.contains("world"),
            "expected the surrounding valid text to survive: {:?}",
            response.content
        );
        assert!(
            response.content.contains('\u{FFFD}'),
            "expected the invalid byte to lossily substitute as U+FFFD: {:?}",
            response.content
        );
    }

    // ── Absolute ceiling scheduling math is pure/sync (#270 review F4) ──
    //
    // `next_step_timeout` is unit-tested directly, without any real
    // waiting, so the ceiling-vs-inactivity arithmetic is verified fast and
    // deterministically rather than only implicitly via a multi-hour test.

    #[test]
    fn next_step_timeout_uses_inactivity_timeout_when_far_from_the_ceiling() {
        let (step, ceiling_bound) = next_step_timeout(
            Duration::from_secs(0),
            Duration::from_secs(300),
            Duration::from_secs(7200),
        )
        .expect("well under the ceiling");
        assert_eq!(step, Duration::from_secs(300));
        assert!(!ceiling_bound);
    }

    #[test]
    fn next_step_timeout_shrinks_the_step_as_the_ceiling_approaches() {
        let (step, ceiling_bound) = next_step_timeout(
            Duration::from_secs(7100),
            Duration::from_secs(300),
            Duration::from_secs(7200),
        )
        .expect("still under the ceiling, by 100s");
        assert_eq!(step, Duration::from_secs(100));
        assert!(ceiling_bound);
    }

    #[test]
    fn next_step_timeout_is_none_once_the_ceiling_is_reached() {
        assert!(
            next_step_timeout(
                Duration::from_secs(7200),
                Duration::from_secs(300),
                Duration::from_secs(7200),
            )
            .is_none()
        );
        assert!(
            next_step_timeout(
                Duration::from_secs(7300),
                Duration::from_secs(300),
                Duration::from_secs(7200),
            )
            .is_none()
        );
    }

    // ── The ceiling must be genuinely CONSULTED, not just correct in
    // isolation (#270 review round 2, item 1) ──
    //
    // `next_step_timeout`'s own tests above prove the scheduling math is
    // right. They prove nothing about whether `complete()`/`stream()`
    // actually call it and act on the result — a review deleted the
    // ceiling check from BOTH loops and the suite stayed green (20/20);
    // only an incidental `unused variable: start` warning betrayed it.
    // `with_absolute_ceiling_secs` (test-only) patches the ceiling down so
    // this can observe it actually firing without waiting 2 real hours —
    // mirroring how the reviewer proved the mechanism works (patched to
    // 3s, watched a heartbeat-forever process die at 3.004s).

    /// Mutation this catches: replacing the `next_step_timeout(...)` call
    /// with an unconditional `Some((timeout, false))` (i.e. ignoring the
    /// ceiling entirely, ceiling check optimized away/deleted). The script
    /// never goes silent (a tick every 0.1s, far under the 30s inactivity
    /// timeout), so with the ceiling bypassed this call has nothing left
    /// to make it return at all — it hangs, caught by the generous outer
    /// test-level timeout instead of the whole suite.
    ///
    /// Asserts on the OUTCOME, not the clock (#270 review round 4): a
    /// prior version asserted `elapsed` fell in a tight window around the
    /// patched 1s ceiling (even after fixing round 3's separate `start`-
    /// outside-the-lock measurement bug). That is sensitive to a magnitude
    /// mutation (e.g. a 10x ceiling inflation) but still flaky under real
    /// load — subprocess spawn plus scheduling can push the observed time
    /// past a tight bound even when the mechanism is correct — and this
    /// sentinel is the ONLY guard on the ceiling's wiring, so a flaky one
    /// is worse than none: it gets silenced, then deleted, and the wiring
    /// goes back to being unguarded with a green suite. Measured: 1
    /// failure in 12 full-suite runs with the tight bound. The categorical
    /// check below — the error is specifically the absolute-ceiling one,
    /// not an inactivity timeout — reads no clock at all, so it is immune
    /// to scheduling load, while staying fully sensitive to the mutation
    /// this test exists to catch (deleting/bypassing the ceiling check
    /// either hangs, or would have to relabel the error as an inactivity
    /// timeout instead — the negative assertion below rejects that).
    /// Traded away, deliberately: sensitivity to the ceiling firing at the
    /// WRONG time (a 10x inflation still produces a "ceiling" error, just
    /// late) — closing that gap without a wall-clock assertion would
    /// reintroduce the exact flakiness this redesign removes; the pure
    /// `next_step_timeout` unit tests above remain the authority on the
    /// scheduling math itself.
    #[tokio::test]
    async fn absolute_ceiling_is_actually_consulted_by_complete() {
        let outcome = tokio::time::timeout(Duration::from_secs(15), async {
            with_env_lock(async {
                let provider = CliProvider::new("sh".to_string(), vec!["-c".to_string()], 30)
                    .with_absolute_ceiling_secs(1);
                provider
                    .complete(echo_request("while true; do echo tick; sleep 0.1; done"))
                    .await
            })
            .await
        })
        .await;

        // The 15s outer bound is generous on purpose: its job is only to
        // fail a genuine hang, not to measure the mechanism.
        let result = outcome.expect("must not hang past 15s if the ceiling wiring regresses");
        let err = result
            .expect_err("a heartbeat-forever process must still die at the absolute ceiling")
            .to_string();
        assert!(
            err.contains("ceiling"),
            "expected the absolute-ceiling error specifically: {err}"
        );
        assert!(
            !err.contains("inactivity"),
            "the error must not be an inactivity timeout — this script never goes \
             silent, so an inactivity error here means the ceiling check was \
             bypassed and the (far longer) inactivity timeout fired instead: {err}"
        );
    }

    /// Same sentinel, for `stream()`: its ceiling is threaded through
    /// separately (`absolute_ceiling_secs` captured into the spawned task
    /// before `complete()`'s logic ever runs), so pinning `complete()`
    /// alone proves nothing about `stream()` — confirmed by hand:
    /// multiplying `stream()`'s `absolute_ceiling` by 1000 left the whole
    /// suite green (#270 review round 3, item 1). See the sibling test
    /// above for why this asserts on the error kind, not elapsed time.
    #[tokio::test]
    async fn absolute_ceiling_is_actually_consulted_by_stream() {
        use tokio_stream::StreamExt;
        let outcome = tokio::time::timeout(Duration::from_secs(15), async {
            with_env_lock(async {
                let provider = CliProvider::new("sh".to_string(), vec!["-c".to_string()], 30)
                    .with_absolute_ceiling_secs(1);
                let mut stream = provider
                    .stream(echo_request("while true; do echo tick; sleep 0.1; done"))
                    .await
                    .unwrap();
                let mut items = Vec::new();
                while let Some(item) = stream.next().await {
                    items.push(item);
                }
                items
            })
            .await
        })
        .await;

        let items = outcome.expect("must not hang past 15s if the ceiling wiring regresses");
        let err = items
            .last()
            .expect("expected at least the ceiling error item")
            .as_ref()
            .expect_err("a heartbeat-forever process must still die at the absolute ceiling")
            .to_string();
        assert!(
            err.contains("ceiling"),
            "expected the absolute-ceiling error specifically: {err}"
        );
        assert!(
            !err.contains("inactivity"),
            "the error must not be an inactivity timeout — this script never goes \
             silent, so an inactivity error here means the ceiling check was \
             bypassed and the (far longer) inactivity timeout fired instead: {err}"
        );
    }

    /// `stream()`'s post-EOF `child.wait()` must itself be BOUNDED, not
    /// just correct about exit status once it returns — a script that
    /// closes stdout but keeps running for far longer than the inactivity
    /// timeout must not hang the spawned task forever (#270 review round
    /// 3, item 1: `stream_gives_the_child_time_to_finish_after_stdout_eof`
    /// and `stream_reports_a_non_zero_exit_after_successful_output` both
    /// use a SHORT post-EOF delay, so neither would notice if the
    /// `tokio::time::timeout` wrapping `child.wait()` were deleted and the
    /// wait became unbounded — this is the dedicated sentinel for that).
    ///
    /// Mutation this catches: removing the `tokio::time::timeout(...)`
    /// around `child.wait()` in `stream()`'s post-loop code (unbounded
    /// wait) — this would hang past the 30s the script actually sleeps,
    /// caught here by the outer 10s test-level guard instead of hanging
    /// the whole suite.
    #[tokio::test]
    async fn stream_post_eof_wait_is_bounded_not_unbounded() {
        use tokio_stream::StreamExt;
        // Both bounds sit well below the child's 30s sleep, which is what an
        // unbounded post-EOF wait would run toward — so either bound tripping
        // still proves the defect. They are generous because the timings are
        // measured against a real subprocess spawn under a loaded parallel
        // `cargo test`: at 10s/5s this test failed roughly 1 run in 12 on the
        // full suite while passing 6/6 in isolation (#270 review, #348).
        let outcome = tokio::time::timeout(Duration::from_secs(25), async {
            with_env_lock(async {
                let start = std::time::Instant::now();
                let provider = CliProvider::new("sh".to_string(), vec!["-c".to_string()], 1);
                let mut stream = provider
                    .stream(echo_request("echo hi; exec 1>/dev/null; sleep 30"))
                    .await
                    .unwrap();
                let mut items = Vec::new();
                while let Some(item) = stream.next().await {
                    items.push(item);
                }
                (items, start.elapsed())
            })
            .await
        })
        .await;

        let (items, elapsed) =
            outcome.expect("must not hang past 25s if the post-EOF wait becomes unbounded");
        assert!(
            items.iter().any(|i| i.as_ref().is_ok_and(|s| s == "hi")),
            "expected the line written before EOF to still arrive: {items:?}"
        );
        // The inner bound catches a *partial* regression — a wait that grew but
        // did not become infinite — which the outer timeout alone would miss.
        assert!(
            elapsed < Duration::from_secs(15),
            "must not block toward the child's 30s post-EOF sleep: {elapsed:?}"
        );
    }

    // ── `stream()`'s post-EOF phase (#270 review round 2, items 2 & 3) ──
    //
    // Closing stdout does not mean the child has exited. `stream()` used
    // to drop `child` (SIGKILLing it via `kill_on_drop`) the instant EOF
    // was seen, with no exit-status check at all — a non-zero exit after
    // successful output was silently reported as a clean stream end.

    /// Mutation this catches: reverting to no post-EOF exit-status check
    /// at all (the pre-fix shape) — the stream would end with only the
    /// `Ok("hi")` item and no error, silently reporting success for a
    /// command that actually failed.
    #[tokio::test]
    async fn stream_reports_a_non_zero_exit_after_successful_output() {
        use tokio_stream::StreamExt;
        let items = with_env_lock(async {
            let provider = CliProvider::new("sh".to_string(), vec!["-c".to_string()], 10);
            let mut stream = provider
                .stream(echo_request("echo hi; exit 42"))
                .await
                .unwrap();
            let mut items = Vec::new();
            while let Some(item) = stream.next().await {
                items.push(item);
            }
            items
        })
        .await;

        assert_eq!(
            items.len(),
            2,
            "expected the line then a failure item: {items:?}"
        );
        assert_eq!(items[0].as_ref().unwrap(), "hi");
        let err = items[1].as_ref().unwrap_err().to_string();
        assert!(
            err.contains("exit status: 42"),
            "expected the exit code to surface, error was: {err}"
        );
    }

    /// The child keeps running (past stdout EOF) just long enough to reach
    /// its OWN distinct exit code (7, chosen to be unmistakable) — an
    /// instant SIGKILL at EOF would never let it get there.
    ///
    /// Mutation this catches: dropping `child` right after the read loop
    /// instead of bounding a `child.wait()` first — the process would be
    /// SIGKILLed mid-sleep, before reaching `exit 7`, so this specific
    /// exit code would never surface.
    #[tokio::test]
    async fn stream_gives_the_child_time_to_finish_after_stdout_eof() {
        use tokio_stream::StreamExt;
        let items = with_env_lock(async {
            let provider = CliProvider::new("sh".to_string(), vec!["-c".to_string()], 10);
            let mut stream = provider
                .stream(echo_request("echo hi; exec 1>/dev/null; sleep 0.3; exit 7"))
                .await
                .unwrap();
            let mut items = Vec::new();
            while let Some(item) = stream.next().await {
                items.push(item);
            }
            items
        })
        .await;

        let last = items.last().expect("expected at least the failure item");
        let err = last
            .as_ref()
            .expect_err("expected an error for a non-zero exit")
            .to_string();
        assert!(
            err.contains("exit status: 7"),
            "expected the child's own post-EOF exit code (7) to survive to \
             the status check — an early kill would prevent it from ever \
             being reached: {err}"
        );
    }

    // ── JSON-mode stdout parsing (Part C: real cost/tokens from `claude`) ──

    #[test]
    fn parse_json_stdout_claude_extracts_cost_and_tokens() {
        let provider = CliProvider::new("claude".to_string(), vec![], 10);
        // A representative claude --output-format stream-json session: an
        // init event, one assistant turn (the visible answer), then the
        // terminal result event carrying usage/cost.
        let jsonl = concat!(
            r#"{"type":"system","subtype":"init","session_id":"s1","model":"claude-opus-4-6","agents":[]}"#,
            "\n",
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Hello there!"}]}}"#,
            "\n",
            r#"{"type":"result","subtype":"success","is_error":false,"duration_ms":1200,"num_turns":1,"result":"Hello there!","session_id":"s1","total_cost_usd":0.0123,"usage":{"input_tokens":42,"output_tokens":17},"modelUsage":{"claude-opus-4-6":{"outputTokens":17}}}"#,
        );

        let response = provider.parse_json_stdout(jsonl);

        assert_eq!(response.content, "Hello there!");
        assert_eq!(response.tokens_in, 42);
        assert_eq!(response.tokens_out, 17);
        assert!((response.cost - 0.0123).abs() < 1e-9);
        assert_eq!(response.model, "claude-opus-4-6");
    }

    #[test]
    fn parse_json_stdout_falls_back_to_raw_when_no_result_event() {
        // Partial/interrupted stream: no terminal "result" event at all.
        let provider = CliProvider::new("claude".to_string(), vec![], 10);
        let jsonl = r#"{"type":"system","subtype":"init","session_id":"s1","tools":[]}"#;

        let response = provider.parse_json_stdout(jsonl);

        // Falls back to the current behavior: raw stdout as content, zeroed metrics.
        assert_eq!(response.content, jsonl);
        assert_eq!(response.tokens_in, 0);
        assert_eq!(response.tokens_out, 0);
        assert_eq!(response.cost, 0.0);
    }

    // ── system_prompt forwarding (fix: CliProvider was dropping system_prompt) ──

    fn request_with_system(system_prompt: &str, text: &str) -> CompletionRequest {
        CompletionRequest {
            model: "echo".to_string(),
            system_prompt: system_prompt.to_string(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: text.to_string(),
            }],
            temperature: 0.0,
            max_tokens: None,
        }
    }

    #[test]
    fn compose_input_empty_system_prompt_returns_message_unchanged() {
        let provider = CliProvider::new("echo".to_string(), vec![], 10);
        let request = request_with_system("", "fais la tache");
        assert_eq!(provider.compose_input(&request), "fais la tache");
    }

    #[test]
    fn compose_input_non_empty_system_prompt_prefixes_message() {
        let provider = CliProvider::new("echo".to_string(), vec![], 10);
        let request =
            request_with_system("PERSONA-XYZ instructions de delegation", "fais la tache");
        let composed = provider.compose_input(&request);

        assert_eq!(
            composed,
            "<system>\nPERSONA-XYZ instructions de delegation\n</system>\n\nfais la tache"
        );
    }

    #[tokio::test]
    async fn non_empty_system_prompt_reaches_the_command() {
        let response = with_env_lock(async {
            let provider = CliProvider::new("echo".to_string(), vec![], 10);
            let request =
                request_with_system("PERSONA-XYZ instructions de delegation", "fais la tache");

            provider.complete(request).await.unwrap()
        })
        .await;

        assert!(
            response.content.contains("PERSONA-XYZ"),
            "expected system prompt marker in output, got: {}",
            response.content
        );
        assert!(
            response.content.contains("fais la tache"),
            "expected task text in output, got: {}",
            response.content
        );
        let system_pos = response.content.find("PERSONA-XYZ").unwrap();
        let task_pos = response.content.find("fais la tache").unwrap();
        assert!(
            system_pos < task_pos,
            "expected system prompt before task, got: {}",
            response.content
        );
    }

    #[tokio::test]
    async fn empty_system_prompt_changes_nothing() {
        let response = with_env_lock(async {
            let provider = CliProvider::new("echo".to_string(), vec![], 10);
            let request = request_with_system("", "hello world");

            provider.complete(request).await.unwrap()
        })
        .await;

        assert_eq!(response.content.trim(), "hello world");
    }
}
