//! Black-box regressions for the static tier placeholders (`latest`,
//! `latest:fast`, `latest:pro`, `latest:max`) on the `armadai run` path
//! (#376).
//!
//! Only `latest:auto` used to be resolved. The other four went to the
//! provider **verbatim, as a model name** — a 400 "model not found" against
//! a real API, or silently wrong routing on a permissive gateway — while
//! `armadai link` resolved them correctly, so the same string appeared to
//! work on one command and not the other.
//!
//! These are wire-level tests on purpose. The defect is not observable from
//! any of ArmadAI's own output: `agent_start` reports the agent's *declared*
//! model (`latest:pro`, by design, exactly as it does for `latest:auto`),
//! and the CLI provider ignores `request.model` entirely. The only place the
//! bug exists is in the bytes sent to the server, so the test reads the
//! bytes: a scripted HTTP server on `127.0.0.1:0` records the `model` field
//! of every request body the real `armadai` binary sends it, with
//! `ANTHROPIC_BASE_URL` pointed at it. No key, no network, no fake-provider
//! feature.
//!
//! Spawning the real binary is also what makes these *wiring* tests: the fix
//! spans one CLI loop (`--pipe`) and four event-sourced effect runners, each
//! with its own roster construction. A unit test per runner (there is one,
//! next to each runner) cannot show that every CLI entry point actually
//! reaches the fixed code.

#![cfg(feature = "providers-api")]

use armadai_core::model_resolution::{ModelTier, fallback_model_for_tier};
use assert_cmd::Command;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

// ── Scripted Anthropic-shaped server ─────────────────────────────────

/// A minimal HTTP/1.1 server that answers every `POST /v1/messages` with a
/// well-formed Anthropic completion and records the `model` field of the
/// request body.
///
/// The response echoes the model it was asked for, so a failure message
/// shows what the binary actually sent rather than just that it differed.
struct FakeApi {
    port: u16,
    models: Arc<Mutex<Vec<String>>>,
}

impl FakeApi {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let models: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&models);

        // Detached: the test process owns the whole listener's lifetime and
        // exits when the test binary does. Each connection is served inline
        // (the client is a single sequential run), then closed.
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut reader = BufReader::new(stream.try_clone().unwrap());

                // Headers, then exactly `Content-Length` body bytes.
                let mut len = 0usize;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
                        break;
                    }
                    if line == "\r\n" || line == "\n" {
                        break;
                    }
                    if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        len = v.trim().parse().unwrap_or(0);
                    }
                }
                let mut body = vec![0u8; len];
                if reader.read_exact(&mut body).is_err() {
                    continue;
                }

                let model = serde_json::from_slice::<serde_json::Value>(&body)
                    .ok()
                    .and_then(|v| v["model"].as_str().map(str::to_string))
                    .unwrap_or_else(|| "<unparseable>".to_string());
                sink.lock().unwrap().push(model.clone());

                let payload = serde_json::json!({
                    "content": [{"type": "text", "text": "ok"}],
                    "model": model,
                    "usage": {"input_tokens": 1, "output_tokens": 1},
                })
                .to_string();
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{payload}",
                    payload.len()
                );
                let _ = stream.flush();
            }
        });

        Self { port, models }
    }

    fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}/v1", self.port)
    }

    /// Every model string the binary asked for, in request order.
    fn models_seen(&self) -> Vec<String> {
        self.models.lock().unwrap().clone()
    }
}

// ── Project fixture ──────────────────────────────────────────────────

/// A sandbox holding the project, an isolated `ARMADAI_CONFIG_DIR` and an
/// isolated `XDG_DATA_HOME`.
///
/// Both redirections matter: without the config dir the agent-shadowing
/// check scans the developer's real global library **and**
/// `resolve_model_for_tier` reads their real `models-cache.json` (making the
/// expected model machine-dependent); without the data dir `record_run`
/// writes this test's runs into the developer's real SQLite database (#267).
struct Sandbox {
    _dir: tempfile::TempDir,
    root: PathBuf,
    config: PathBuf,
    data: PathBuf,
}

impl Sandbox {
    fn new(agents: &[(&str, &str)]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        std::fs::create_dir_all(root.join("agents")).unwrap();

        let list: String = agents
            .iter()
            .map(|(name, _)| format!("  - name: {name}\n"))
            .collect();
        std::fs::write(root.join("armadai.yaml"), format!("agents:\n{list}")).unwrap();
        for (name, model) in agents {
            write_api_agent(&root, name, model);
        }

        let config = dir.path().join("config");
        let data = dir.path().join("data");
        std::fs::create_dir_all(&config).unwrap();
        std::fs::create_dir_all(&data).unwrap();
        Self {
            _dir: dir,
            root,
            config,
            data,
        }
    }

    fn run(&self, api: &FakeApi, args: &[&str]) -> std::process::Output {
        let mut cmd = Command::cargo_bin("armadai").unwrap();
        cmd.current_dir(&self.root)
            .env("ARMADAI_CONFIG_DIR", &self.config)
            .env("XDG_DATA_HOME", &self.data)
            .env("ANTHROPIC_BASE_URL", api.base_url())
            .env("ANTHROPIC_API_KEY", "sk-not-a-real-key")
            .env("NO_COLOR", "1")
            .args(args);
        cmd.output().unwrap()
    }
}

/// An agent on the HTTP Anthropic provider — the path that takes the model
/// string literally.
fn write_api_agent(root: &Path, name: &str, model: &str) {
    std::fs::write(
        root.join("agents").join(format!("{name}.md")),
        format!(
            "# {name}\n\n\
             ## Metadata\n\
             - provider: anthropic\n\
             - model: {model}\n\n\
             ## System Prompt\n\
             You are {name}.\n"
        ),
    )
    .unwrap();
}

/// The concrete id a tier resolves to with an empty models cache — which is
/// what the isolated `ARMADAI_CONFIG_DIR` guarantees. Derived from the same
/// table the production code falls back to, rather than hardcoded here, so
/// bumping a default model does not turn this into a red test about nothing.
fn expected(tier: ModelTier) -> String {
    fallback_model_for_tier("anthropic", tier).to_string()
}

fn assert_no_placeholder_reached_the_wire(seen: &[String]) {
    assert!(
        !seen.is_empty(),
        "the binary never called the provider — the test proves nothing"
    );
    for m in seen {
        assert!(
            !m.contains("latest"),
            "a `latest:*` placeholder reached the provider as a model name: {seen:?}"
        );
    }
}

// ── Tests ────────────────────────────────────────────────────────────

/// Single agent — the `direct` event-sourced runner.
#[test]
fn single_agent_resolves_a_static_tier_placeholder_before_calling_the_provider() {
    let api = FakeApi::start();
    let sb = Sandbox::new(&[("alpha", "latest:pro")]);

    let out = sb.run(&api, &["run", "alpha", "hello", "--json"]);
    assert!(out.status.success(), "run failed: {out:?}");

    let seen = api.models_seen();
    assert_no_placeholder_reached_the_wire(&seen);
    assert_eq!(seen, vec![expected(ModelTier::Pro)]);
}

/// `--pipe` — the sequential loop, the one path that is not event-sourced.
/// Two links on two different tiers, so a fix that resolved a single shared
/// model for the whole chain would show up here.
#[test]
fn a_pipe_chain_resolves_each_links_own_tier() {
    let api = FakeApi::start();
    let sb = Sandbox::new(&[("alpha", "latest:pro"), ("beta", "latest:fast")]);

    let out = sb.run(&api, &["run", "alpha", "hello", "--pipe", "beta", "--json"]);
    assert!(out.status.success(), "run failed: {out:?}");

    let seen = api.models_seen();
    assert_no_placeholder_reached_the_wire(&seen);
    assert_eq!(
        seen,
        vec![expected(ModelTier::Pro), expected(ModelTier::Fast)]
    );
}

/// `--orchestrate` — the orchestrated roster (built by `run_orchestrated`,
/// dispatched through the `ring` effect runner). `ring` circulates then
/// votes, so each agent is called several times; the assertion is on the
/// distinct set, not the count.
#[test]
fn an_orchestrated_roster_resolves_every_members_tier() {
    let api = FakeApi::start();
    let sb = Sandbox::new(&[("alpha", "latest:pro"), ("beta", "latest:max")]);

    let out = sb.run(
        &api,
        &[
            "run",
            "alpha",
            "hello",
            "--pipe",
            "beta",
            "--orchestrate",
            "ring",
            "--json",
        ],
    );
    assert!(out.status.success(), "run failed: {out:?}");

    let seen = api.models_seen();
    assert_no_placeholder_reached_the_wire(&seen);
    let mut distinct: Vec<String> = seen.clone();
    distinct.sort();
    distinct.dedup();
    let mut want = vec![expected(ModelTier::Pro), expected(ModelTier::Max)];
    want.sort();
    assert_eq!(distinct, want);
}

/// `--resume` rebuilds its own roster from the project on disk, so it is a
/// third construction site and needs its own proof.
///
/// The resumable run is created the cheap way: an agent whose `command:`
/// does not exist makes the `direct` effect runner propagate the error, and
/// the process dies leaving the log in `Running`. The agent is then rewritten
/// onto the API provider with a tier placeholder, which is what `--resume`
/// reloads.
#[cfg(feature = "storage")]
#[test]
fn a_resumed_run_resolves_the_reloaded_rosters_tier() {
    let api = FakeApi::start();
    let sb = Sandbox::new(&[("alpha", "latest:pro")]);

    std::fs::write(
        sb.root.join("agents/alpha.md"),
        "# alpha\n\n## Metadata\n- provider: cli\n- command: /nonexistent-armadai-command\n\n\
         ## System Prompt\nYou are alpha.\n",
    )
    .unwrap();

    let out = sb.run(&api, &["run", "alpha", "hello", "--json"]);
    assert!(
        !out.status.success(),
        "the setup run was supposed to fail, leaving a resumable run: {out:?}"
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let run_id = stdout
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .find(|v| v["t"] == "run_start")
        .and_then(|v| v["run_id"].as_str().map(str::to_string))
        .expect("no run_start event to take a run id from");
    assert!(
        api.models_seen().is_empty(),
        "the setup run must not have reached the API at all"
    );

    write_api_agent(&sb.root, "alpha", "latest:max");
    let out = sb.run(&api, &["run", "--resume", &run_id, "--json", "--no-tui"]);
    assert!(out.status.success(), "resume failed: {out:?}");

    let seen = api.models_seen();
    assert_no_placeholder_reached_the_wire(&seen);
    assert_eq!(seen, vec![expected(ModelTier::Max)]);
}
