# Rate-limiting Lot 1 (proactive throttling) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make ArmadAI's exposed-but-inert provider rate-limiting real: a shared, durable per-process throttle applied to every provider call site (event-sourced engines included), driven by the two existing knobs (`config.rate_limits` per provider + agent frontmatter `rate_limit`), with the `/hour` panic fixed.

**Architecture:** A `RateLimitedProvider` decorator (implements the `Provider` trait, wraps the real provider) installed in `factory.rs::create_provider`, so all call sites inherit throttling via the trait. A process-global registry shares one limiter per provider key across a run; an optional per-agent limiter (from frontmatter) tightens it. The token bucket is reworked to a `Rate { per_sec, burst }` representation to kill the integer-division panic.

**Tech Stack:** Rust edition 2024, `src/providers/` (rate_limiter, factory, traits), `tokio`, `async_trait`.

## Global Constraints

- Spec: `docs/superpowers/specs/2026-07-27-rate-limiting-design.md`. Milestone item **#265** (P1). Scope = **Lot 1 (A) only**; 429/529/Retry-After/backoff = **Lot 2, OUT of scope**.
- The `Provider` trait (`src/providers/traits.rs`) is **unchanged**; the decorator implements it.
- No rate-limit section in `armadai.yaml`/`ProjectConfig`.
- **Absence of a cap → unlimited** (no limiter, never blocks). A refill ≤ 0 or "unlimited" construction must make `acquire()` return immediately — **never** `Duration::from_secs_f64(inf)` (the current panic).
- Throttling is **silent** + `tracing::debug!` when a wait actually happened. **No** `RunEvent`/Workroom signal (Lot 2).
- `config.rate_limits` values are **requests per MINUTE** (`u32`; defaults anthropic:50, openai:60, google:60, proxy:100).
- The `RateLimiter`/`Rate`/`RateLimitedProvider`/registry are **NOT feature-gated** (must compile in all 3 CI modes); only the concrete API providers are gated `providers-api`.
- Gate every task: `cargo fmt --all` + clippy 3 modes (`tui` / `tui,providers-api` / `tui,web,storage`) `-D warnings` + `cargo test --no-default-features --features tui` + `cargo test --no-default-features --features tui,storage`.
- rust-analyzer unreliable (ABI/inactive/stale) — verify at the compiler. Conventional Commits single type; trailer `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`. One PR (Lot 1) + independent review + Dimitri validation.

---

## File Structure

- `src/providers/rate_limiter.rs` — `Rate` struct + reworked `RateLimiter` (Task 1) + `RateLimitedProvider` decorator + process-global registry (Task 2).
- `src/providers/factory.rs` — wrap the constructed provider (Task 3).
- `src/providers/mod.rs` — export the new items if needed (Task 2/3).
- `src/cli/run.rs` — remove the two dead manual limiter blocks (Task 3).

---

## Task 1: Rework `RateLimiter` around a `Rate { per_sec, burst }` (kill the panic)

**Files:**
- Modify: `src/providers/rate_limiter.rs`

**Interfaces:**
- Produces:
  - `pub struct Rate { pub per_sec: f64, pub burst: f64 }`
  - `impl Rate { pub fn from_per_minute(per_minute: f64) -> Rate; pub fn parse(s: &str) -> Option<Rate>; }`
  - `impl RateLimiter { pub fn new(rate: Rate) -> Self; pub async fn acquire(&self); }`
  - Semantics: `per_sec <= 0.0` or `burst <= 0.0` ⇒ "unlimited" ⇒ `acquire()` returns immediately.

- [ ] **Step 1: Write failing tests for `Rate::parse` (precise, no truncation/panic)**

Replace the existing `parse_rate_formats` test and add cases. In `#[cfg(test)] mod tests`:

```rust
#[test]
fn rate_parse_is_precise_no_truncation() {
    // per-minute canonical form
    let r = Rate::parse("10/min").unwrap();
    assert!((r.per_sec - 10.0 / 60.0).abs() < 1e-9);
    assert_eq!(r.burst, 10.0);

    // per-second
    let r = Rate::parse("1/sec").unwrap();
    assert!((r.per_sec - 1.0).abs() < 1e-9);
    assert_eq!(r.burst, 1.0);

    // per-hour: the OLD bug truncated 30/hour -> 0. Now precise.
    let r = Rate::parse("30/hour").unwrap();
    assert!((r.per_sec - 30.0 / 3600.0).abs() < 1e-9);
    assert_eq!(r.burst, 30.0);

    // sub-1/window still yields burst >= 1 (so a single request can pass)
    let r = Rate::parse("1/hour").unwrap();
    assert!((r.per_sec - 1.0 / 3600.0).abs() < 1e-12);
    assert_eq!(r.burst, 1.0);

    // aliases
    assert!(Rate::parse("5/m").is_some());
    assert!(Rate::parse("2/second").is_some());
    assert!(Rate::parse("100/hr").is_some());

    // invalid
    assert!(Rate::parse("invalid").is_none());
    assert!(Rate::parse("10/decade").is_none());
    assert!(Rate::parse("abc/min").is_none());
}
```

- [ ] **Step 2: Run — expect FAIL (Rate not defined)**

Run: `cargo test --no-default-features --features tui rate_parse_is_precise -- --nocapture`
Expected: FAIL (`cannot find type Rate`).

- [ ] **Step 3: Implement `Rate` + `parse`**

Add near the top of `src/providers/rate_limiter.rs`:

```rust
/// A parsed rate: sustained refill (`per_sec`) plus bucket capacity (`burst`).
/// `burst` is the window count, floored at 1.0 so a single request can always
/// pass before throttling kicks in (a capacity < 1 would deadlock acquire).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rate {
    pub per_sec: f64,
    pub burst: f64,
}

impl Rate {
    /// Build a rate from a requests-per-minute value (used by `config.rate_limits`).
    pub fn from_per_minute(per_minute: f64) -> Rate {
        Rate {
            per_sec: per_minute / 60.0,
            burst: per_minute.max(1.0),
        }
    }

    /// Parse "N/sec", "N/min", "N/hour" (with aliases). Precise — no integer
    /// truncation. Returns `None` on malformed input.
    pub fn parse(s: &str) -> Option<Rate> {
        let (count_str, unit) = s.split_once('/')?;
        let count: f64 = count_str.trim().parse::<u32>().ok()? as f64;
        let per_sec = match unit.trim() {
            "s" | "sec" | "second" => count,
            "m" | "min" | "minute" => count / 60.0,
            "h" | "hr" | "hour" => count / 3600.0,
            _ => return None,
        };
        Some(Rate {
            per_sec,
            burst: count.max(1.0),
        })
    }
}
```

- [ ] **Step 4: Run — expect PASS**

Run: `cargo test --no-default-features --features tui rate_parse_is_precise`
Expected: PASS.

- [ ] **Step 5: Rework `RateLimiter` to take a `Rate` + unlimited guard**

Replace `RateLimiter::new` and the `BucketState`/`acquire` refill so it uses `Rate`, and guard the unlimited case. New `new`:

```rust
impl RateLimiter {
    /// Create a limiter from a `Rate`. A non-positive `per_sec`/`burst` means
    /// "unlimited": `acquire()` never waits.
    pub fn new(rate: Rate) -> Self {
        Self {
            state: Mutex::new(BucketState {
                tokens: rate.burst,
                max_tokens: rate.burst,
                refill_rate: rate.per_sec,
                last_refill: Instant::now(),
            }),
        }
    }
}
```

In `acquire`, add the guard at the top of the locked block (before computing any wait):

```rust
        // Unlimited: no throttle configured.
        if state.refill_rate <= 0.0 || state.max_tokens <= 0.0 {
            return; // (restructure: see note)
        }
```

Note for the implementer: `acquire` currently computes `wait` inside a block then matches outside the lock. Restructure so the unlimited check short-circuits the whole `loop` (e.g. check `refill_rate <= 0.0 || max_tokens <= 0.0` once and `return`), guaranteeing `Duration::from_secs_f64(deficit / refill_rate)` is only reached when `refill_rate > 0.0` — so the `inf`/`NaN` panic is impossible. Keep the existing bucket math otherwise.

- [ ] **Step 6: Update the existing limiter tests to the new API + add unlimited/panic-guard + throttle tests**

Rewrite the `acquire_*` tests to use `Rate` and add coverage:

```rust
#[tokio::test]
async fn acquire_unlimited_never_waits_and_never_panics() {
    // per_sec 0 == unlimited; the OLD code panicked here (from_secs_f64(inf)).
    let limiter = RateLimiter::new(Rate { per_sec: 0.0, burst: 0.0 });
    let start = Instant::now();
    for _ in 0..1000 {
        limiter.acquire().await;
    }
    assert!(start.elapsed() < Duration::from_millis(200));
}

#[tokio::test]
async fn acquire_within_burst_is_immediate() {
    let limiter = RateLimiter::new(Rate::from_per_minute(60.0)); // burst 60
    let start = Instant::now();
    limiter.acquire().await;
    limiter.acquire().await;
    assert!(start.elapsed() < Duration::from_millis(100));
}

#[tokio::test]
async fn acquire_waits_when_burst_exhausted() {
    let limiter = RateLimiter::new(Rate::from_per_minute(60.0)); // 1/sec, burst 60
    for _ in 0..60 {
        limiter.acquire().await;
    }
    let start = Instant::now();
    limiter.acquire().await;
    // Refill is 1/sec, so the 61st waits ~1s. Generous margin (non-flaky).
    assert!(start.elapsed() >= Duration::from_millis(800));
}

#[tokio::test]
async fn low_hourly_rate_does_not_panic_and_passes_first_call() {
    // 30/hour: OLD code -> new(0) -> panic on first acquire. Now: burst 30 -> passes.
    let limiter = RateLimiter::new(Rate::parse("30/hour").unwrap());
    let start = Instant::now();
    limiter.acquire().await; // must not panic, must return promptly (burst available)
    assert!(start.elapsed() < Duration::from_millis(100));
}
```

- [ ] **Step 7: Run all limiter tests + gate**

Run: `cargo test --no-default-features --features tui rate_limiter` then the full gate (fmt + clippy 3 modes + test 2 modes).
Expected: PASS / clean. (No other file references `RateLimiter::new(u32)`/`parse_rate` yet except `cli/run.rs:906/1294` — those are removed in Task 3; if the crate doesn't compile because of them, this task may leave them; **verify**: Task 1 must keep the crate compiling. If `cli/run.rs` still calls the old `parse_rate`/`new(u32)`, update those two blocks minimally to the new API in THIS task OR remove them here — prefer removing them here since Task 3 also removes them; pick one and keep the build green. Recommended: remove the two blocks in Task 1's Step 5 commit so the crate compiles, and Task 3 only adds the factory wiring.)

- [ ] **Step 8: Commit**

```bash
git add src/providers/rate_limiter.rs src/cli/run.rs
git commit -m "refactor(providers): Rate{per_sec,burst} limiter, fix /hour panic (rate-limit Lot 1)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: `RateLimitedProvider` decorator + process-global registry

**Files:**
- Modify: `src/providers/rate_limiter.rs` (add decorator + registry)
- Modify: `src/providers/mod.rs` (export `RateLimitedProvider`, `provider_limiter` if needed by `factory.rs`)

**Interfaces:**
- Consumes: `Rate`, `RateLimiter` (Task 1); `Provider`/`CompletionRequest`/`CompletionResponse`/`TokenStream`/`ProviderMetadata` (`traits.rs`).
- Produces:
  - `pub struct RateLimitedProvider` wrapping `inner: Arc<dyn Provider>`, `provider_limiter: Option<Arc<RateLimiter>>`, `agent_limiter: Option<Arc<RateLimiter>>`.
  - `pub fn new(inner: Arc<dyn Provider>, provider_limiter: Option<Arc<RateLimiter>>, agent_limiter: Option<Arc<RateLimiter>>) -> RateLimitedProvider`.
  - `pub fn shared_provider_limiter(key: &str, rate: Option<Rate>) -> Option<Arc<RateLimiter>>` — process-global memoized limiter per provider key; `None` when `rate` is `None`.

- [ ] **Step 1: Write failing test — a fake Provider counting calls, shared limiter throttles across two decorators**

Add to the tests module (uses `async_trait`, already a dep):

```rust
use crate::providers::traits::{
    ChatMessage, CompletionRequest, CompletionResponse, Provider, ProviderMetadata, TokenStream,
};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

struct CountingProvider {
    calls: Arc<AtomicUsize>,
}
#[async_trait::async_trait]
impl Provider for CountingProvider {
    async fn complete(&self, _req: CompletionRequest) -> anyhow::Result<CompletionResponse> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(CompletionResponse { content: "ok".into(), model: "m".into(), tokens_in: 0, tokens_out: 0, cost: 0.0 })
    }
    async fn stream(&self, _req: CompletionRequest) -> anyhow::Result<TokenStream> {
        anyhow::bail!("unused")
    }
    fn metadata(&self) -> ProviderMetadata {
        ProviderMetadata { name: "counting".into(), models: vec![], supports_streaming: false }
    }
}
fn req() -> CompletionRequest {
    CompletionRequest { model: "m".into(), system_prompt: String::new(), messages: vec![ChatMessage{role:"user".into(),content:"hi".into()}], temperature: 0.0, max_tokens: None }
}

#[tokio::test]
async fn no_limiters_never_blocks() {
    let calls = Arc::new(AtomicUsize::new(0));
    let p = RateLimitedProvider::new(Arc::new(CountingProvider{calls: calls.clone()}), None, None);
    let start = Instant::now();
    for _ in 0..50 { p.complete(req()).await.unwrap(); }
    assert_eq!(calls.load(Ordering::SeqCst), 50);
    assert!(start.elapsed() < Duration::from_millis(200));
}

#[tokio::test]
async fn shared_provider_limiter_throttles_across_decorators() {
    // Two decorators sharing ONE provider limiter (burst 2, 1/sec refill).
    let shared = Arc::new(RateLimiter::new(Rate { per_sec: 1.0, burst: 2.0 }));
    let calls = Arc::new(AtomicUsize::new(0));
    let a = RateLimitedProvider::new(Arc::new(CountingProvider{calls: calls.clone()}), Some(shared.clone()), None);
    let b = RateLimitedProvider::new(Arc::new(CountingProvider{calls: calls.clone()}), Some(shared.clone()), None);
    // 2 immediate (burst), then the 3rd (via the OTHER decorator) must wait ~1s.
    a.complete(req()).await.unwrap();
    a.complete(req()).await.unwrap();
    let start = Instant::now();
    b.complete(req()).await.unwrap();
    assert!(start.elapsed() >= Duration::from_millis(800));
    assert_eq!(calls.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn agent_limiter_tightens_and_both_must_pass() {
    // Loose provider limiter, tight per-agent limiter -> agent limiter governs.
    let provider = Arc::new(RateLimiter::new(Rate::from_per_minute(600.0))); // basically loose
    let agent = Arc::new(RateLimiter::new(Rate { per_sec: 1.0, burst: 1.0 })); // 1 then wait
    let calls = Arc::new(AtomicUsize::new(0));
    let p = RateLimitedProvider::new(Arc::new(CountingProvider{calls: calls.clone()}), Some(provider), Some(agent));
    p.complete(req()).await.unwrap();
    let start = Instant::now();
    p.complete(req()).await.unwrap();
    assert!(start.elapsed() >= Duration::from_millis(800));
}

#[test]
fn shared_registry_memoizes_by_key() {
    let a = shared_provider_limiter("anthropic", Some(Rate::from_per_minute(50.0))).unwrap();
    let b = shared_provider_limiter("anthropic", Some(Rate::from_per_minute(50.0))).unwrap();
    assert!(Arc::ptr_eq(&a, &b)); // same key -> same Arc
    assert!(shared_provider_limiter("nokey", None).is_none());
}
```

- [ ] **Step 2: Run — expect FAIL (`RateLimitedProvider` / `shared_provider_limiter` undefined)**

Run: `cargo test --no-default-features --features tui rate_limiter -- --nocapture`
Expected: FAIL (unresolved names).

- [ ] **Step 3: Implement the decorator**

```rust
use std::sync::Arc;
use crate::providers::traits::{CompletionRequest, CompletionResponse, Provider, ProviderMetadata, TokenStream};

/// Wraps a `Provider` and awaits up to two limiters (shared-per-provider,
/// then per-agent) before delegating. The tighter one effectively governs.
pub struct RateLimitedProvider {
    inner: Arc<dyn Provider>,
    provider_limiter: Option<Arc<RateLimiter>>,
    agent_limiter: Option<Arc<RateLimiter>>,
}

impl RateLimitedProvider {
    pub fn new(
        inner: Arc<dyn Provider>,
        provider_limiter: Option<Arc<RateLimiter>>,
        agent_limiter: Option<Arc<RateLimiter>>,
    ) -> Self {
        Self { inner, provider_limiter, agent_limiter }
    }

    async fn throttle(&self) {
        if let Some(l) = &self.provider_limiter { l.acquire().await; }
        if let Some(l) = &self.agent_limiter { l.acquire().await; }
    }
}

#[async_trait::async_trait]
impl Provider for RateLimitedProvider {
    async fn complete(&self, request: CompletionRequest) -> anyhow::Result<CompletionResponse> {
        self.throttle().await;
        self.inner.complete(request).await
    }
    async fn stream(&self, request: CompletionRequest) -> anyhow::Result<TokenStream> {
        self.throttle().await;
        self.inner.stream(request).await
    }
    fn metadata(&self) -> ProviderMetadata {
        self.inner.metadata()
    }
}
```

Note: `acquire()` already `tracing::debug!`-style logging is out of scope in the limiter; add a `tracing::debug!("rate-limit: throttling provider call")` inside `throttle` guarded so it only logs when a limiter exists (the wait itself is inside `acquire`). Keep it minimal.

- [ ] **Step 4: Implement the process-global registry**

```rust
use std::collections::HashMap;
use std::sync::{Mutex as StdMutex, OnceLock};

static PROVIDER_LIMITERS: OnceLock<StdMutex<HashMap<String, Arc<RateLimiter>>>> = OnceLock::new();

/// Return (memoized, process-global) the shared limiter for `key`, creating it
/// from `rate` on first use. `None` when no rate is configured for `key`.
pub fn shared_provider_limiter(key: &str, rate: Option<Rate>) -> Option<Arc<RateLimiter>> {
    let rate = rate?;
    let map = PROVIDER_LIMITERS.get_or_init(|| StdMutex::new(HashMap::new()));
    let mut guard = map.lock().expect("provider limiter registry poisoned");
    Some(guard.entry(key.to_string()).or_insert_with(|| Arc::new(RateLimiter::new(rate))).clone())
}
```

- [ ] **Step 5: Export from `mod.rs`**

In `src/providers/mod.rs`, ensure `rate_limiter` items are reachable by `factory.rs`: `pub use rate_limiter::{Rate, RateLimiter, RateLimitedProvider, shared_provider_limiter};` (adjust to the module's existing export style).

- [ ] **Step 6: Run tests + gate**

Run: `cargo test --no-default-features --features tui rate_limiter` then full gate. Confirm the decorator compiles in `tui` (no `providers-api`) — it must, since it only depends on the `Provider` trait (always compiled). Expected: PASS / clean in all 3 clippy modes.

- [ ] **Step 7: Commit**

```bash
git add src/providers/rate_limiter.rs src/providers/mod.rs
git commit -m "feat(providers): RateLimitedProvider decorator + shared per-process limiter registry (rate-limit Lot 1)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: Wire the decorator into `create_provider` + retire the dead blocks

**Files:**
- Modify: `src/providers/factory.rs` (`create_provider` + a `rate_limit_key` helper)
- Modify: `src/cli/run.rs` (remove the two dead manual limiter blocks — only if Task 1 didn't already)

**Interfaces:**
- Consumes: `RateLimitedProvider`, `shared_provider_limiter`, `Rate`, `RateLimiter` (Task 1/2); `load_user_config()` (`crate::core::config`), `Agent`/`AgentMetadata`.
- Produces: `create_provider` returns a `RateLimitedProvider`-wrapped `Box<dyn Provider>` (same signature `-> anyhow::Result<Box<dyn Provider>>`).

- [ ] **Step 1: Write the failing test — `create_provider` wraps with the right limiters**

Because `create_provider` needs a real-ish `Agent`, test the `rate_limit_key` mapping + the wrapping decision at the helper level (avoid needing a live API). Add to `factory.rs` tests:

```rust
#[test]
fn rate_limit_key_maps_providers() {
    assert_eq!(rate_limit_key("anthropic"), Some("anthropic".to_string()));
    assert_eq!(rate_limit_key("openai"), Some("openai".to_string()));
    assert_eq!(rate_limit_key("google"), Some("google".to_string()));
    assert_eq!(rate_limit_key("proxy"), Some("proxy".to_string()));
    // unified names map to their API backend key
    assert_eq!(rate_limit_key("claude"), Some("anthropic".to_string()));
    assert_eq!(rate_limit_key("gemini"), Some("google".to_string()));
    assert_eq!(rate_limit_key("gpt"), Some("openai".to_string()));
    // pure CLI: no per-provider quota key
    assert_eq!(rate_limit_key("cli"), None);
}
```

- [ ] **Step 2: Run — expect FAIL (`rate_limit_key` undefined)**

Run: `cargo test --no-default-features --features tui,providers-api rate_limit_key`
Expected: FAIL.

- [ ] **Step 3: Implement `rate_limit_key` + the wrapping in `create_provider`**

Add the helper:

```rust
/// Map an agent's `provider` string to the `config.rate_limits` key, or `None`
/// for providers with no per-account API quota (pure CLI).
fn rate_limit_key(provider: &str) -> Option<String> {
    match provider {
        "anthropic" | "openai" | "google" | "proxy" => Some(provider.to_string()),
        "claude" => Some("anthropic".to_string()),
        "gemini" => Some("google".to_string()),
        "gpt" => Some("openai".to_string()),
        _ => None, // "cli", unknown, or unified-resolving-to-cli
    }
}
```

Refactor `create_provider` so the existing branches build the inner provider, then wrap once before returning:

```rust
pub fn create_provider(agent: &Agent) -> anyhow::Result<Box<dyn Provider>> {
    let provider = agent.metadata.provider.as_str();
    let inner: Box<dyn Provider> = match provider {
        "cli" => create_cli_provider(agent)?,
        "anthropic" | "openai" | "google" | "proxy" => create_api_provider(provider, agent)?,
        _ => {
            if let Some(tool) = find_tool(provider) {
                create_unified_provider(provider, tool, agent)?
            } else {
                anyhow::bail!("Unknown provider: '{provider}'. Known providers: cli, anthropic, openai, google, claude, gemini, gpt, aider");
            }
        }
    };
    Ok(wrap_rate_limited(agent, inner))
}

/// Wrap `inner` with the shared per-provider limiter (from `config.rate_limits`)
/// and the optional per-agent limiter (from frontmatter `rate_limit`).
fn wrap_rate_limited(agent: &Agent, inner: Box<dyn Provider>) -> Box<dyn Provider> {
    use super::rate_limiter::{Rate, RateLimiter, RateLimitedProvider, shared_provider_limiter};
    let inner: std::sync::Arc<dyn Provider> = std::sync::Arc::from(inner);

    let provider_limiter = rate_limit_key(agent.metadata.provider.as_str()).and_then(|key| {
        let rate = crate::core::config::load_user_config()
            .rate_limits
            .get(&key)
            .map(|&per_min| Rate::from_per_minute(per_min as f64));
        shared_provider_limiter(&key, rate)
    });

    let agent_limiter = agent
        .metadata
        .rate_limit
        .as_deref()
        .and_then(Rate::parse)
        .map(|r| std::sync::Arc::new(RateLimiter::new(r)));

    if provider_limiter.is_none() && agent_limiter.is_none() {
        // No throttle configured: return the inner unchanged (no overhead).
        return unarc(inner);
    }
    Box::new(RateLimitedProvider::new(inner, provider_limiter, agent_limiter))
}
```

Note for the implementer: `Box<dyn Provider>` → `Arc<dyn Provider>` via `Arc::from(box)` works. The `unarc` helper is only needed if you want to return the bare inner when no limiter applies; simpler is to always wrap (the decorator with two `None` limiters is a zero-cost pass-through — `throttle()` awaits nothing). **Prefer always wrapping** (drop the `unarc` branch): it's simpler and the overhead is nil. Adjust the code to always return `Box::new(RateLimitedProvider::new(inner, provider_limiter, agent_limiter))`.

- [ ] **Step 4: Run the mapping test — expect PASS**

Run: `cargo test --no-default-features --features tui,providers-api rate_limit_key`
Expected: PASS.

- [ ] **Step 5: Remove the dead manual limiter blocks (if still present)**

If Task 1 did not already remove them, delete these now-redundant blocks:
- `src/cli/run.rs` in `run_single_agent` (~lines 906-912):
  ```rust
  // 3. Apply rate limiting if configured
  if let Some(ref rate_str) = agent.metadata.rate_limit
      && let Some(rpm) = RateLimiter::parse_rate(rate_str) { let limiter = RateLimiter::new(rpm); limiter.acquire().await; }
  ```
- `src/cli/run.rs` in `run_single_agent_es` (~lines 1294-1299):
  ```rust
  // 3. Rate limiting (step 3).
  if let Some(ref rate_str) = agent.metadata.rate_limit
      && let Some(rpm) = RateLimiter::parse_rate(rate_str) { RateLimiter::new(rpm).acquire().await; }
  ```
Remove the now-unused `use` of `RateLimiter` in `run.rs` if it becomes dead. The throttle now lives in the decorator, applied to ALL paths (including the 4 ES engines that call `provider.complete()` directly), because they all obtain their provider from `create_provider`.

- [ ] **Step 6: Verify the ES path obtains its provider from `create_provider`**

Confirm (read-only) that `src/core/orchestration/es/{direct,blackboard,ring,hierarchical}.rs` (and their `cli/run.rs` dispatchers) build providers via `create_provider`/the factory — so the decorator covers them. If any path constructs a provider by other means, note it in the report (it would need the same wrapping). Do not change behavior beyond wrapping.

- [ ] **Step 7: Full gate**

Run: fmt + clippy 3 modes + test 2 modes. Expected: clean / all pass. Confirm no remaining reference to the old `RateLimiter::parse_rate`/`RateLimiter::new(u32)` API.

- [ ] **Step 8: Commit**

```bash
git add src/providers/factory.rs src/cli/run.rs
git commit -m "feat(providers): wire rate-limit decorator into the factory, retire dead run.rs blocks (rate-limit Lot 1)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Self-Review

**Spec coverage:**
- Decorator at factory covering all call sites incl. ES → Task 3 (+ Step 6 verifies ES obtains provider via factory). ✅
- Two-layer limiter (shared provider-key + per-agent) → Task 2 decorator + Task 3 wiring. ✅
- Shared per-process registry keyed by provider → Task 2 `shared_provider_limiter`. ✅
- `Rate{per_sec,burst}` f64, `/hour` precise, no-panic guard → Task 1. ✅
- Revive `config.rate_limits` (req/min → Rate) + frontmatter `rate_limit` → Task 3 `wrap_rate_limited`. ✅
- Remove dead `run.rs` blocks → Task 1 Step 7 note or Task 3 Step 5. ✅
- Provider trait unchanged; no armadai.yaml section; unlimited when no cap; silent + tracing::debug → Global Constraints + Task 2/3. ✅
- Not feature-gated (compiles all 3 modes) → Task 2 Step 6 verifies. ✅
- 429/529 OUT of scope → not in any task. ✅

**Placeholder scan:** No TBD/TODO. Two implementer decisions are bounded and resolved inline: (a) where to remove the dead `run.rs` blocks (Task 1 recommended, so the crate stays compiling; Task 3 Step 5 is the fallback) — pick Task 1; (b) always-wrap vs conditional (resolved: always wrap, drop `unarc`).

**Type consistency:** `Rate { per_sec, burst }`, `RateLimiter::new(Rate)`, `Rate::parse`/`Rate::from_per_minute`, `RateLimitedProvider::new(Arc<dyn Provider>, Option<Arc<RateLimiter>>, Option<Arc<RateLimiter>>)`, `shared_provider_limiter(&str, Option<Rate>) -> Option<Arc<RateLimiter>>`, `rate_limit_key(&str) -> Option<String>` — consistent across Tasks 1→3. `config.rate_limits: HashMap<String,u32>` (req/min) → `Rate::from_per_minute(u32 as f64)`.

**Resolved ambiguity:** The crate must stay compiling after Task 1 (it changes `RateLimiter`'s public API, breaking the two `run.rs` call sites). Therefore Task 1 Step 7 removes those two blocks in the same commit; Task 3 Step 5 becomes a no-op verification. This is called out in both tasks.
