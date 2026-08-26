//! Shared HTTP retry/backoff for the API providers (anthropic, google).
//!
//! Lot 1 (`rate_limiter.rs`, one directory up) is a pure token-bucket
//! throttler: it smooths OUTGOING calls, proactively, before they hit the
//! wire. It contains no retry, no backoff, no 429/529 handling. This module
//! is the reactive half — what to do when the SERVER comes back with a
//! rate-limit or overload response.
//!
//! Both providers speak plain HTTP and hit overlapping — not identical —
//! failure modes under overload, all of which need the same retryable/
//! terminal distinction (a 400 or 401 is never worth retrying — doing so
//! only delays a clear error):
//!
//! - **429** (rate limited): both Anthropic (`rate_limit_error`) and
//!   Google/Gemini (`RESOURCE_EXHAUSTED`) emit this.
//! - **529**: Anthropic's `overloaded_error`. Not part of Gemini's
//!   documented error taxonomy, but harmless to keep in the shared set —
//!   Google will simply never produce it.
//! - **503**: Gemini's overload signal (`UNAVAILABLE`). Also RFC 7231's
//!   generic "Service Unavailable", which per spec MAY carry `Retry-After`
//!   and is meant to be transient — so treating it as retryable is the
//!   conventional reading, not a stretch. Anthropic doesn't document it,
//!   but the same "harmless if unused" reasoning applies.
//!
//! One shared, provider-agnostic set — `is_retryable` below — covers both
//! rather than dispatching per provider, and is implemented ONCE here and
//! used by every HTTP provider rather than grown independently in each —
//! two copies of one policy is the defect class this repo has fixed
//! repeatedly (see the issue write-up). Since #368 that includes the
//! OpenAI-compatible path (`api::openai_compatible`, behind both
//! `api::openai` and `proxy`), which reaches a far wider set of servers
//! (gateways, local runtimes) than the two first-party APIs — all the more
//! reason for one shared policy rather than a per-vendor one.
//!
//! Bounded by ATTEMPT COUNT, not elapsed time and not a caller-supplied
//! deadline: neither provider threads `agent.metadata.timeout` (that only
//! feeds `CliProvider`) or any `reqwest::Client` timeout into its request
//! path today, so there is no existing deadline for this loop to interact
//! with. A `Retry-After` value from the server is honored close to
//! verbatim — capping it too aggressively would defeat the point of
//! trusting the server's own knowledge of its quota window — but it IS
//! capped, at the dedicated, more generous `RetryPolicy::max_retry_after`
//! (default 60s, well above the guessed-backoff cap): an unbounded,
//! server-controlled sleep inside a client is a defect regardless of
//! whether the server is well-behaved — a misconfigured proxy or a buggy
//! upstream sending `Retry-After: 86400` must not park the caller for a
//! day. Only the HTTP delta-seconds form of the header is parsed (see
//! `parse_retry_after`); the header is only ever a hint, and every wait is
//! still bounded overall by `max_retries`, so a caller-side deadline added
//! later composes for free: wrap the whole `complete()`/`stream()` call in
//! `tokio::time::timeout(..)` from the outside; nothing in this loop needs
//! to change.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use reqwest::{RequestBuilder, Response, StatusCode, header::RETRY_AFTER};

/// Bounds and pacing for the retry loop. `Default` gives the production
/// values; tests build a `RetryPolicy` with tiny delays so the suite stays
/// fast and deterministic (never sleeping seconds for no reason).
#[derive(Debug, Clone, Copy)]
pub(crate) struct RetryPolicy {
    /// Retries allowed AFTER the first attempt. 3 -> up to 4 total tries.
    pub max_retries: u32,
    /// Base for the exponential backoff used when the server sends no
    /// `Retry-After` (`base * 2^(attempt-1)`, before jitter).
    pub base_delay: Duration,
    /// Ceiling for a GUESSED backoff wait (no `Retry-After` present).
    pub max_backoff: Duration,
    /// Ceiling for a server-declared `Retry-After`. Deliberately separate
    /// from, and more generous than, `max_backoff`: the server told us
    /// something real about its own quota window, which deserves more
    /// trust than our own guess — but not unlimited trust. 60s is long
    /// enough to respect a real rate-limit window while still bounding the
    /// worst case from a misconfigured proxy or a buggy/malicious upstream
    /// (e.g. `Retry-After: 86400`) to something sane.
    pub max_retry_after: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 3,
            base_delay: Duration::from_millis(500),
            max_backoff: Duration::from_secs(30),
            max_retry_after: Duration::from_secs(60),
        }
    }
}

/// One retry-or-give-up decision, and — when retrying — how long to wait.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RetryDecision {
    Retry(Duration),
    GiveUp,
}

/// Whether an HTTP status is worth retrying: 429 (rate limited, both
/// providers), 529 (Anthropic's "overloaded"), and 503 (Gemini's overload
/// signal, and RFC 7231's generic transient "Service Unavailable"). See the
/// module docs for which provider actually emits which. Everything else —
/// including other 4xx/5xx — is terminal. A 400 or 401 will never succeed
/// on replay; retrying it only turns a clear, fast error into a slow one.
pub(crate) fn is_retryable(status: StatusCode) -> bool {
    matches!(status.as_u16(), 429 | 503 | 529)
}

/// Parse a `Retry-After` value as a plain count of seconds — the only form
/// Anthropic/Google actually send. The HTTP-date form (`Retry-After: <date>`,
/// e.g. `Wed, 21 Oct 2026 07:28:00 GMT`) is legal per RFC 7231 but is not
/// produced by either provider today and is not parsed here — a value that
/// doesn't parse as an unsigned integer is treated as absent (falling back
/// to computed backoff) rather than guessed at. `send_with_retry` logs when
/// this happens so the fallback isn't silent.
pub(crate) fn parse_retry_after(value: &str) -> Option<Duration> {
    value.trim().parse::<u64>().ok().map(Duration::from_secs)
}

/// Decide whether the response to attempt number `attempt` (1-based: 1 is
/// the first try) should be retried, and after how long.
pub(crate) fn decide(
    policy: &RetryPolicy,
    status: StatusCode,
    retry_after: Option<Duration>,
    attempt: u32,
) -> RetryDecision {
    if !is_retryable(status) {
        return RetryDecision::GiveUp;
    }
    if attempt > policy.max_retries {
        return RetryDecision::GiveUp;
    }
    match retry_after {
        Some(delay) => RetryDecision::Retry(delay.min(policy.max_retry_after)),
        None => RetryDecision::Retry(backoff_with_jitter(policy, attempt)),
    }
}

fn backoff_with_jitter(policy: &RetryPolicy, attempt: u32) -> Duration {
    let exp = attempt.saturating_sub(1).min(16);
    let cap = policy
        .base_delay
        .saturating_mul(1u32 << exp)
        .min(policy.max_backoff);
    full_jitter(cap)
}

/// Uniform-in-`[0, cap]` "full jitter" (AWS's term for the classic
/// decorrelated-backoff trick), so concurrent callers backing off from the
/// same overload don't retry in lockstep. Hand-rolled xorshift64* seeded
/// from the wall clock and a process-wide counter — no `rand` dependency
/// needed for what is, deliberately, a dozen lines; this doesn't need to be
/// cryptographically strong, only unpredictable enough to decorrelate.
fn full_jitter(cap: Duration) -> Duration {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let mut x = nanos ^ n.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xD1B5_4A32_D192_ED03;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    let frac = (x >> 11) as f64 / (1u64 << 53) as f64; // uniform in [0, 1)
    cap.mul_f64(frac)
}

/// Send a request, retrying on 429/503/529 per `policy` and honoring
/// `Retry-After` (capped, see module docs) when the server sends one.
/// `build` is called once per
/// attempt (never `Fn`-cached) since a `RequestBuilder` is consumed by
/// `send()` and can't be replayed.
///
/// Returns the FIRST response that is either successful or not worth
/// retrying (terminal status, or the retry budget is exhausted) — success
/// or failure, callers keep their existing status-check / error-body
/// logic completely unchanged; this function only decides whether to try
/// again, never how to interpret the final response.
pub(crate) async fn send_with_retry<F>(
    policy: &RetryPolicy,
    mut build: F,
) -> reqwest::Result<Response>
where
    F: FnMut() -> RequestBuilder,
{
    let mut attempt: u32 = 1;
    loop {
        let response = build().send().await?;
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }

        let retry_after_raw = response
            .headers()
            .get(RETRY_AFTER)
            .and_then(|v| v.to_str().ok());
        let retry_after = retry_after_raw.and_then(parse_retry_after);

        if let Some(raw) = retry_after_raw
            && retry_after.is_none()
        {
            // Legal (HTTP-date) but unsupported form — see parse_retry_after.
            // Not silent: falling back to computed backoff without saying so
            // would hide that we ignored a hint the server actually gave us.
            tracing::debug!(
                raw_value = raw,
                "Retry-After header present but not in the supported delta-seconds form \
                 (HTTP-date is not parsed) — falling back to computed backoff",
            );
        }

        match decide(policy, status, retry_after, attempt) {
            RetryDecision::GiveUp => return Ok(response),
            RetryDecision::Retry(delay) => {
                tracing::warn!(
                    %status,
                    attempt,
                    max_retries = policy.max_retries,
                    delay_ms = delay.as_millis() as u64,
                    "provider HTTP error is retryable, backing off before retry",
                );
                tokio::time::sleep(delay).await;
                attempt += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- pure decision logic: no networking, no sleeping ---

    #[test]
    fn parse_retry_after_reads_plain_seconds_and_rejects_garbage() {
        // Mutation this catches: the parser accepting non-integer input
        // (e.g. switching to `f64` parsing, or not trimming whitespace).
        assert_eq!(parse_retry_after("7"), Some(Duration::from_secs(7)));
        assert_eq!(parse_retry_after("  3  "), Some(Duration::from_secs(3)));
        assert_eq!(parse_retry_after("0"), Some(Duration::from_secs(0)));
        assert_eq!(parse_retry_after("abc"), None);
        assert_eq!(parse_retry_after(""), None);
        assert_eq!(parse_retry_after("-1"), None);
        assert_eq!(parse_retry_after("1.5"), None);
    }

    #[test]
    fn parse_retry_after_does_not_parse_the_http_date_form() {
        // Documents a known, deliberate limitation (see module docs):
        // Retry-After's HTTP-date form is legal per RFC 7231 but not
        // parsed. Mutation this catches: silently starting to accept it
        // (which would need re-auditing the "falls back to backoff, with a
        // log line" behavior this test and `send_with_retry` both assume).
        assert_eq!(parse_retry_after("Wed, 21 Oct 2026 07:28:00 GMT"), None);
    }

    #[test]
    fn is_retryable_is_exactly_429_503_and_529() {
        // Mutation this catches: the retryable set drifting — e.g. dropping
        // 503 (Gemini's actual overload status) or 529 (Anthropic's,
        // easy to forget when generalizing) because they're
        // provider-specific, or "helpfully" widening to a generic 5xx.
        for code in [429, 503, 529] {
            assert!(
                is_retryable(StatusCode::from_u16(code).unwrap()),
                "{code} should be retryable"
            );
        }
        for code in [400, 401, 403, 404, 500, 502, 504] {
            assert!(
                !is_retryable(StatusCode::from_u16(code).unwrap()),
                "{code} must NOT be retryable"
            );
        }
    }

    #[test]
    fn retry_after_header_is_honored_up_to_the_cap() {
        // Mutation this catches: computing a backoff delay even when the
        // server gave an explicit `Retry-After`, or scaling that value
        // instead of using it as-is (for a value comfortably under the
        // cap, "as-is" and "capped" are indistinguishable — that's the
        // point of this test; the next one pins the cap itself).
        let policy = RetryPolicy {
            max_retries: 3,
            base_delay: Duration::from_millis(1),
            max_backoff: Duration::from_millis(1),
            max_retry_after: Duration::from_secs(60),
        };
        let decision = decide(
            &policy,
            StatusCode::TOO_MANY_REQUESTS,
            Some(Duration::from_secs(7)),
            1,
        );
        assert_eq!(decision, RetryDecision::Retry(Duration::from_secs(7)));
    }

    #[test]
    fn retry_after_header_above_the_cap_is_clamped() {
        // The defect this closes: a buggy server or an injecting proxy
        // sending `Retry-After: 86400` must not park the caller for a day.
        // Mutation this catches: dropping the `.min(max_retry_after)` (or
        // applying it to the wrong branch, e.g. the guessed backoff
        // instead of the header).
        let policy = RetryPolicy {
            max_retries: 3,
            base_delay: Duration::from_millis(1),
            max_backoff: Duration::from_millis(1),
            max_retry_after: Duration::from_secs(60),
        };
        let decision = decide(
            &policy,
            StatusCode::TOO_MANY_REQUESTS,
            Some(Duration::from_secs(86_400)),
            1,
        );
        assert_eq!(decision, RetryDecision::Retry(Duration::from_secs(60)));
    }

    #[test]
    fn no_header_backs_off_within_cap_and_jitters() {
        // Mutation this catches: backoff ignoring `max_backoff` (unbounded
        // growth), and backoff being deterministic (no jitter at all —
        // e.g. always returning the cap), which would make concurrent
        // retries lock-step.
        let policy = RetryPolicy {
            max_retries: 5,
            base_delay: Duration::from_millis(100),
            max_backoff: Duration::from_millis(100),
            ..RetryPolicy::default()
        };
        let mut seen = std::collections::HashSet::new();
        for _ in 0..30 {
            let RetryDecision::Retry(d) = decide(&policy, StatusCode::TOO_MANY_REQUESTS, None, 1)
            else {
                panic!("expected a retry decision");
            };
            assert!(
                d <= Duration::from_millis(100),
                "backoff exceeded the cap: {d:?}"
            );
            seen.insert(d);
        }
        assert!(
            seen.len() > 1,
            "30 samples produced no variation — jitter looks disabled"
        );
    }

    #[test]
    fn terminal_status_never_retries_regardless_of_attempt_or_header() {
        // Mutation this catches: the retryable check being skipped or
        // inverted, so a 400 with a (nonsensical but present) Retry-After
        // header would still be retried.
        let policy = RetryPolicy::default();
        let decision = decide(
            &policy,
            StatusCode::BAD_REQUEST,
            Some(Duration::from_secs(1)),
            1,
        );
        assert_eq!(decision, RetryDecision::GiveUp);
    }

    #[test]
    fn retry_budget_boundary_last_allowed_attempt_retries_next_gives_up() {
        // Mutation this catches: an off-by-one in the exhaustion check
        // (`>=` vs `>`), which either retries one time too many or gives up
        // one time too early. Pins the exact boundary.
        let policy = RetryPolicy {
            max_retries: 2,
            base_delay: Duration::from_millis(1),
            max_backoff: Duration::from_millis(1),
            ..RetryPolicy::default()
        };
        assert!(matches!(
            decide(&policy, StatusCode::TOO_MANY_REQUESTS, None, 2),
            RetryDecision::Retry(_)
        ));
        assert_eq!(
            decide(&policy, StatusCode::TOO_MANY_REQUESTS, None, 3),
            RetryDecision::GiveUp
        );
    }

    // --- end-to-end over real (local-only) HTTP: proves the wiring, not
    // just the decision table ---

    // The scripted local HTTP server used below now lives in
    // `super::test_server`, shared with `openai_compatible.rs` — see that
    // module for what it does and why mocking at the client level would not
    // do instead.
    use crate::api::test_server::ScriptedServer;

    fn tiny_policy() -> RetryPolicy {
        RetryPolicy {
            max_retries: 2,
            base_delay: Duration::from_millis(20),
            max_backoff: Duration::from_millis(20),
            ..RetryPolicy::default()
        }
    }

    #[tokio::test]
    async fn send_with_retry_honors_real_retry_after_header() {
        // Mutation this catches: the header extraction never being wired
        // into `send_with_retry` (wrong header name/case, or the
        // `.headers().get(...)` call being dropped) — if honoring the
        // header were broken, this would fall through to the tiny 20ms
        // backoff instead of the header's declared 1s, and the timing
        // assertion below would fail.
        let server = ScriptedServer::start(vec![
            (429, vec![("Retry-After", "1".to_string())], ""),
            (200, vec![], "ok"),
        ]);
        let client = reqwest::Client::new();
        let url = server.url();
        let start = std::time::Instant::now();
        let response = send_with_retry(&tiny_policy(), || client.get(&url))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(
            start.elapsed() >= Duration::from_millis(900),
            "should have waited ~1s per Retry-After, took {:?}",
            start.elapsed()
        );
        assert_eq!(server.request_count(), 2);
    }

    #[tokio::test]
    async fn send_with_retry_clamps_a_pathological_retry_after_header() {
        // The over-the-wire proof of the cap fix: a server (or an
        // injecting proxy) sending a huge Retry-After must not park the
        // caller anywhere near that long. Mutation this catches: the cap
        // not actually being wired into the real `send_with_retry` path
        // (e.g. only added to `decide` in isolation, or applied to the
        // wrong Duration) — this test uses a policy whose cap is tiny and
        // whose base_delay/max_backoff are LARGER, so honoring the
        // (huge) header uncapped OR falling back to the backoff cap would
        // both blow the timing assertion; only clamping to
        // `max_retry_after` fits inside it. Without the cap this doesn't
        // just fail the assertion — it actually sleeps for 999999s
        // (confirmed by hand: the test hung until killed), so the call is
        // wrapped in a `tokio::time::timeout` the same way as the
        // give-up-after-budget test, turning "no cap" into a fast failure.
        let policy = RetryPolicy {
            max_retries: 2,
            base_delay: Duration::from_millis(200),
            max_backoff: Duration::from_millis(200),
            max_retry_after: Duration::from_millis(20),
        };
        let server = ScriptedServer::start(vec![
            (429, vec![("Retry-After", "999999".to_string())], ""),
            (200, vec![], "ok"),
        ]);
        let client = reqwest::Client::new();
        let url = server.url();
        let start = std::time::Instant::now();
        let response = tokio::time::timeout(
            Duration::from_secs(5),
            send_with_retry(&policy, || client.get(&url)),
        )
        .await
        .expect("send_with_retry did not clamp the Retry-After header")
        .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(
            start.elapsed() < Duration::from_millis(150),
            "a Retry-After of 999999s must be clamped to the 20ms cap, took {:?}",
            start.elapsed()
        );
        assert_eq!(server.request_count(), 2);
    }

    #[tokio::test]
    async fn send_with_retry_falls_back_to_backoff_on_an_http_date_header() {
        // Wiring proof for the HTTP-date limitation: a legal but unparsed
        // `Retry-After` form must not crash or hang the request — it
        // should behave exactly like "no header", i.e. computed backoff.
        // Mutation this catches: `send_with_retry` panicking or stalling
        // on a header it can't parse instead of falling through cleanly.
        let server = ScriptedServer::start(vec![
            (
                429,
                vec![("Retry-After", "Wed, 21 Oct 2026 07:28:00 GMT".to_string())],
                "",
            ),
            (200, vec![], "ok"),
        ]);
        let client = reqwest::Client::new();
        let url = server.url();
        let response = send_with_retry(&tiny_policy(), || client.get(&url))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(server.request_count(), 2);
    }

    #[tokio::test]
    async fn send_with_retry_backs_off_and_succeeds_without_header() {
        // Mutation this catches: treating "no Retry-After header" as "don't
        // retry" (an easy slip once the header-based path exists) — if so,
        // the server would see only 1 request and the final status would
        // still be 429, not 200.
        let server = ScriptedServer::start(vec![(429, vec![], ""), (200, vec![], "ok")]);
        let client = reqwest::Client::new();
        let url = server.url();
        let response = send_with_retry(&tiny_policy(), || client.get(&url))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(server.request_count(), 2);
    }

    #[tokio::test]
    async fn send_with_retry_never_retries_a_400_over_the_wire() {
        // Mutation this catches: retrying on any non-2xx instead of just
        // 429/529 — would show up here as more than one request and a slow
        // return instead of an immediate one.
        let server = ScriptedServer::start(vec![(400, vec![], "bad request")]);
        let client = reqwest::Client::new();
        let url = server.url();
        let start = std::time::Instant::now();
        let response = send_with_retry(&tiny_policy(), || client.get(&url))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(server.request_count(), 1);
        assert!(start.elapsed() < Duration::from_millis(200));
    }

    #[tokio::test]
    async fn send_with_retry_gives_up_after_budget_and_stops_the_server_traffic() {
        // The one that matters most: a server that NEVER recovers must not
        // be retried forever. Mutation this catches: a missing/broken
        // exhaustion check turning this into an infinite loop, or a count
        // that drifts from the configured budget. An infinite loop would
        // otherwise just hang this test (and the CI job) instead of failing
        // it — confirmed by removing the exhaustion check by hand: the test
        // hung until killed rather than reporting a failure — so the call
        // is wrapped in a generous `tokio::time::timeout` that turns "loops
        // forever" into an explicit, fast assertion failure.
        let policy = RetryPolicy {
            max_retries: 2,
            base_delay: Duration::from_millis(5),
            max_backoff: Duration::from_millis(5),
            ..RetryPolicy::default()
        };
        let server = ScriptedServer::start(vec![(429, vec![], "still overloaded")]);
        let client = reqwest::Client::new();
        let url = server.url();
        let response = tokio::time::timeout(
            Duration::from_secs(5),
            send_with_retry(&policy, || client.get(&url)),
        )
        .await
        .expect("send_with_retry did not give up — it looped past the retry budget")
        .unwrap();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        // 1 initial attempt + 2 retries = 3, never more.
        assert_eq!(server.request_count(), 3);
    }
}
