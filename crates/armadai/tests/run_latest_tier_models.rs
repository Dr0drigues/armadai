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
//! any of ArmadAI's own output, and that is a measurement rather than a
//! design claim: `agent_start` reports the agent's *declared* model
//! (`latest:pro`), and for a static tier there is no `Route`/`ModelRouted`
//! event carrying the resolved one either — unlike `latest:auto`, whose
//! resolved tier the stream does carry. The stderr summary that does print a
//! concrete id (`[name] model=…`) lives only in `run_single_agent`, i.e. on
//! `--pipe` and nowhere else. So on three of the four run paths nothing
//! ArmadAI emits names the model that was billed (see `es::bridge`'s
//! `execution_event_to_run_events` doc for the open gap).
//!
//! The only place the bug exists is therefore in the bytes sent to the
//! server, so the test reads the bytes: a scripted HTTP server on
//! `127.0.0.1:0` records the `model` field of every request body the real
//! `armadai` binary sends it, with `ANTHROPIC_BASE_URL` pointed at it. No
//! key, no network, no fake-provider feature.
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
        Self::rejecting(&[])
    }

    /// Same server, except every request naming a model in `rejected` is
    /// answered with the 404 an API returns for a model it does not serve —
    /// the trigger for the `model_fallback` retry path.
    fn rejecting(rejected: &[&str]) -> Self {
        let rejected: Vec<String> = rejected.iter().map(|s| (*s).to_string()).collect();
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

                let (status, payload) = if rejected.contains(&model) {
                    (
                        "404 Not Found",
                        serde_json::json!({
                            "type": "error",
                            "error": {"type": "not_found_error",
                                      "message": format!("model: {model} not found")},
                        })
                        .to_string(),
                    )
                } else {
                    (
                        "200 OK",
                        serde_json::json!({
                            "content": [{"type": "text", "text": "ok"}],
                            "model": model,
                            "usage": {"input_tokens": 1, "output_tokens": 1},
                        })
                        .to_string(),
                    )
                };
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\n\
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

    /// Plant a models.dev cache in the isolated config dir, so tier
    /// resolution reads a catalog this test controls instead of falling
    /// through to the hardcoded table. `entries` is the JSON array for one
    /// provider, written verbatim.
    fn seed_models_cache(&self, provider: &str, entries: &str) {
        std::fs::write(
            self.config.join("models-cache.json"),
            format!(r#"{{"providers":{{"{provider}":{entries}}}}}"#),
        )
        .unwrap();
    }

    /// Same, pointed at a Gemini-shaped server instead, and with a PATH that
    /// holds no `gemini` binary — so `provider: gemini` takes the API
    /// fallback (`create_unified_provider`) rather than relaying a CLI that
    /// would ignore the model entirely. `/usr/bin:/bin` still carries
    /// `which`, which is what the availability probe shells out to.
    fn run_gemini(&self, api: &FakeGoogleApi, args: &[&str]) -> std::process::Output {
        let mut cmd = Command::cargo_bin("armadai").unwrap();
        cmd.current_dir(&self.root)
            .env("ARMADAI_CONFIG_DIR", &self.config)
            .env("XDG_DATA_HOME", &self.data)
            .env("PATH", "/usr/bin:/bin")
            .env("GOOGLE_BASE_URL", api.base_url())
            .env("GOOGLE_API_KEY", "not-a-real-key")
            .env("NO_COLOR", "1")
            .args(args);
        cmd.output().unwrap()
    }
}

// ── Scripted Gemini-shaped server ────────────────────────────────────

/// Google names the model in the **URL**
/// (`/v1beta/models/<model>:generateContent`), not the body — which is how
/// the F1 defect was legible at a glance: a Claude id in a Google path.
struct FakeGoogleApi {
    port: u16,
    paths: Arc<Mutex<Vec<String>>>,
}

impl FakeGoogleApi {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let paths: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&paths);

        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut reader = BufReader::new(stream.try_clone().unwrap());

                let mut request_line = String::new();
                if reader.read_line(&mut request_line).unwrap_or(0) == 0 {
                    continue;
                }
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
                let path = request_line
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or("<no path>")
                    .to_string();
                sink.lock().unwrap().push(path);

                let payload = serde_json::json!({
                    "candidates": [{"content": {"parts": [{"text": "ok"}]}}],
                    "usageMetadata": {"promptTokenCount": 1, "candidatesTokenCount": 1},
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

        Self { port, paths }
    }

    fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}/v1beta", self.port)
    }

    /// Every request path the binary asked for, in request order.
    fn paths_seen(&self) -> Vec<String> {
        self.paths.lock().unwrap().clone()
    }
}

/// An agent on the HTTP Anthropic provider — the path that takes the model
/// string literally.
fn write_api_agent(root: &Path, name: &str, model: &str) {
    write_agent(
        root,
        name,
        &format!("- provider: anthropic\n- model: {model}\n"),
    );
}

/// An agent with arbitrary `## Metadata` lines.
fn write_agent(root: &Path, name: &str, metadata: &str) {
    std::fs::write(
        root.join("agents").join(format!("{name}.md")),
        format!(
            "# {name}\n\n\
             ## Metadata\n\
             {metadata}\n\
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

// ── The vendor a tier resolves against (#398 review, F1) ─────────────

/// An agent's `provider:` is a *tool* name; the model catalog is keyed by
/// *vendor*. Handing the tool name to the catalog missed it and fell through
/// to a table whose catch-all answers with Anthropic, so this exact fixture
/// — `provider: gemini`, `model: latest:pro`, no `gemini` binary on PATH —
/// sent `POST /v1beta/models/claude-sonnet-4-5-20250929:generateContent` to
/// Google.
///
/// The assertion is on the URL because that is where the answer is
/// unambiguous: a Claude id inside a `generativelanguage` path cannot be
/// read as anything but the wrong vendor.
#[test]
fn a_tools_tier_resolves_against_its_own_vendor_not_anthropic() {
    let api = FakeGoogleApi::start();
    let sb = Sandbox::new(&[("alpha", "unused")]);
    write_agent(
        &sb.root,
        "alpha",
        "- provider: gemini\n- model: latest:pro\n",
    );

    let out = sb.run_gemini(&api, &["run", "alpha", "hello", "--json"]);
    assert!(out.status.success(), "run failed: {out:?}");

    let seen = api.paths_seen();
    assert!(
        !seen.is_empty(),
        "the binary never called the provider — the test proves nothing"
    );
    let want = fallback_model_for_tier("google", ModelTier::Pro);
    for path in &seen {
        assert!(
            path.contains(want),
            "expected the Google catalog's Pro model ({want}) in the request path, got: {seen:?}"
        );
        assert!(
            !path.contains("claude"),
            "an Anthropic model id reached a Google endpoint: {seen:?}"
        );
    }
}

/// `latest:auto` had the same defect one line above the one #376 fixed: the
/// router names a tier, and the tier was resolved against the raw tool name
/// too. Measured before the fix, this fixture sent
/// `.../models/claude-haiku-4-5-20251001:generateContent` to Google.
#[test]
fn a_routed_tier_resolves_against_its_own_vendor_too() {
    let api = FakeGoogleApi::start();
    let sb = Sandbox::new(&[("alpha", "unused")]);
    write_agent(
        &sb.root,
        "alpha",
        "- provider: gemini\n- model: latest:auto\n",
    );

    let out = sb.run_gemini(&api, &["run", "alpha", "hello", "--json"]);
    assert!(out.status.success(), "run failed: {out:?}");

    let seen = api.paths_seen();
    assert!(
        !seen.is_empty(),
        "the binary never called the provider — the test proves nothing"
    );
    for path in &seen {
        assert!(
            path.contains("gemini-"),
            "expected a Google model id in the request path, got: {seen:?}"
        );
        assert!(
            !path.contains("claude"),
            "an Anthropic model id reached a Google endpoint: {seen:?}"
        );
    }
}

// ── model_fallback entries get the same resolution (#398 review, F4) ──

/// A `model_fallback:` entry is a model string like any other, so the retry
/// that uses it resolves its tier too. Nothing covered this: deleting the
/// resolution at that site left the whole suite green.
///
/// The server rejects the primary model with the 404 an API returns for a
/// model it does not serve, which is what arms the fallback loop; the
/// fallback is a placeholder, and the assertion is that the *second* request
/// named a concrete id and not `latest:fast`.
#[test]
fn a_model_fallback_entry_resolves_its_tier_before_the_retry() {
    let api = FakeApi::rejecting(&["definitely-not-a-model"]);
    let sb = Sandbox::new(&[("alpha", "unused")]);
    write_agent(
        &sb.root,
        "alpha",
        "- provider: anthropic\n\
         - model: definitely-not-a-model\n\
         - model_fallback: [latest:fast]\n",
    );

    let out = sb.run(
        &api,
        &["run", "alpha", "hello", "--pipe", "alpha", "--json"],
    );
    assert!(out.status.success(), "run failed: {out:?}");

    let seen = api.models_seen();
    assert_no_placeholder_reached_the_wire(&seen);
    assert_eq!(
        seen,
        vec![
            "definitely-not-a-model".to_string(),
            expected(ModelTier::Fast),
            "definitely-not-a-model".to_string(),
            expected(ModelTier::Fast),
        ],
        "each link should try its declared model, then its resolved fallback"
    );
}

// ── Which placeholder is routed per call (#398 review, F5) ───────────

/// `run_single_agent`'s guard is `raw_model == "latest:auto"`, exactly: a
/// static tier is resolved from the string alone and must NOT be handed to
/// the router.
///
/// This replaces a unit test that compared two string literals to each other
/// (`assert_ne!("latest:pro", "latest:auto")`) — true at compile time, and
/// exercising no line of production code. It had also become half wrong:
/// `latest:pro` *is* resolved now, just not routed.
///
/// The observable difference is the `route` event, so that is what is
/// asserted — together with both models reaching the wire concrete, so a
/// guard widened to `starts_with("latest")` cannot pass by resolving
/// everything through the router instead.
#[test]
fn only_latest_auto_is_routed_per_call() {
    let api = FakeApi::start();
    let sb = Sandbox::new(&[("alpha", "latest:pro"), ("beta", "latest:auto")]);

    let out = sb.run(&api, &["run", "alpha", "hello", "--pipe", "beta", "--json"]);
    assert!(out.status.success(), "run failed: {out:?}");

    let routed: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|v| v["t"] == "route")
        .map(|v| v["agent"].as_str().unwrap_or("").to_string())
        .collect();
    assert_eq!(
        routed,
        vec!["beta".to_string()],
        "only the `latest:auto` link should be routed per call; stdout:\n{}",
        String::from_utf8_lossy(&out.stdout)
    );

    assert_no_placeholder_reached_the_wire(&api.models_seen());
}

// -- The tier read from the catalog, at the wire (issue #404) ---------

/// With a models.dev cache present, the id that reaches the server is the
/// one the catalog names for the tier -- and "the catalog's newest" is read
/// numerically, not off the alphabet.
///
/// The fixture is the case that separates the two: generation `10` is above
/// generation `4.6`, while the *string* `"claude-sonnet-10"` is below
/// `"claude-sonnet-4-6"`. The old `candidates.iter().max()` therefore sent
/// `claude-sonnet-4-6`. It also prices the newer model *higher*, so
/// "cheapest wins" alone answers `claude-sonnet-4-6` too: the assertion is
/// satisfied only by ordering on the generation first.
///
/// Wire-level rather than unit-level for the reason this whole file exists:
/// on three of the four run paths nothing ArmadAI prints names the model it
/// billed, so the bytes are the only witness.
#[test]
fn the_model_on_the_wire_is_the_catalogs_newest_read_as_a_number() {
    let api = FakeApi::start();
    let sb = Sandbox::new(&[("alpha", "latest:pro")]);
    sb.seed_models_cache(
        "anthropic",
        r#"[{"id":"claude-sonnet-4-6","cost":{"input":2.0,"output":10.0}},
            {"id":"claude-sonnet-10","cost":{"input":3.0,"output":15.0}}]"#,
    );

    let out = sb.run(&api, &["run", "alpha", "hello", "--json"]);
    assert!(out.status.success(), "run failed: {out:?}");

    let seen = api.models_seen();
    assert_no_placeholder_reached_the_wire(&seen);
    assert_eq!(seen, vec!["claude-sonnet-10".to_string()]);
}
