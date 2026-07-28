use crate::core::provider::{
    CompletionRequest, CompletionResponse, Provider, ProviderMetadata, TokenStream,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

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

/// Token-bucket rate limiter for provider calls.
pub struct RateLimiter {
    state: Mutex<BucketState>,
}

struct BucketState {
    tokens: f64,
    max_tokens: f64,
    refill_rate: f64, // tokens per second
    last_refill: Instant,
}

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

    /// Wait until a token is available, then consume it.
    pub async fn acquire(&self) {
        loop {
            let wait = {
                // If the mutex is poisoned, we can't recover, so we panic with a clear message.
                // This should never happen in practice unless there's a panic inside the lock.
                let mut state = self.state.lock().expect("rate limiter mutex poisoned");

                // Unlimited: no throttle configured. Short-circuit before any
                // wait-duration math so `deficit / refill_rate` (which would be
                // `inf`/`NaN` when `refill_rate <= 0.0`) is never computed.
                if state.refill_rate <= 0.0 || state.max_tokens <= 0.0 {
                    return;
                }

                let now = Instant::now();
                let elapsed = now.duration_since(state.last_refill).as_secs_f64();
                state.tokens = (state.tokens + elapsed * state.refill_rate).min(state.max_tokens);
                state.last_refill = now;

                if state.tokens >= 1.0 {
                    state.tokens -= 1.0;
                    None
                } else {
                    let deficit = 1.0 - state.tokens;
                    Some(Duration::from_secs_f64(deficit / state.refill_rate))
                }
            };

            match wait {
                None => return,
                Some(duration) => tokio::time::sleep(duration).await,
            }
        }
    }
}

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
        Self {
            inner,
            provider_limiter,
            agent_limiter,
        }
    }

    async fn throttle(&self) {
        if let Some(l) = &self.provider_limiter {
            tracing::debug!("rate-limit: throttling provider call (provider limiter)");
            l.acquire().await;
        }
        if let Some(l) = &self.agent_limiter {
            tracing::debug!("rate-limit: throttling provider call (agent limiter)");
            l.acquire().await;
        }
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

static PROVIDER_LIMITERS: OnceLock<Mutex<HashMap<String, Arc<RateLimiter>>>> = OnceLock::new();

/// Return (memoized, process-global) the shared limiter for `key`, creating it
/// from `rate` on first use. `None` when no rate is configured for `key`.
/// First-write-wins per key: if `key` is already registered, its existing
/// rate is kept and this call's `rate` is ignored (even if it differs).
pub fn shared_provider_limiter(key: &str, rate: Option<Rate>) -> Option<Arc<RateLimiter>> {
    let rate = rate?;
    let map = PROVIDER_LIMITERS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = map.lock().expect("provider limiter registry poisoned");
    Some(
        guard
            .entry(key.to_string())
            .or_insert_with(|| Arc::new(RateLimiter::new(rate)))
            .clone(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[tokio::test]
    async fn acquire_unlimited_never_waits_and_never_panics() {
        // per_sec 0 == unlimited; the OLD code panicked here (from_secs_f64(inf)).
        let limiter = RateLimiter::new(Rate {
            per_sec: 0.0,
            burst: 0.0,
        });
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

    use crate::core::provider::{
        ChatMessage, CompletionRequest, CompletionResponse, Provider, ProviderMetadata, TokenStream,
    };
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CountingProvider {
        calls: Arc<AtomicUsize>,
    }
    #[async_trait::async_trait]
    impl Provider for CountingProvider {
        async fn complete(&self, _req: CompletionRequest) -> anyhow::Result<CompletionResponse> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(CompletionResponse {
                content: "ok".into(),
                model: "m".into(),
                tokens_in: 0,
                tokens_out: 0,
                cost: 0.0,
            })
        }
        async fn stream(&self, _req: CompletionRequest) -> anyhow::Result<TokenStream> {
            anyhow::bail!("unused")
        }
        fn metadata(&self) -> ProviderMetadata {
            ProviderMetadata {
                name: "counting".into(),
                models: vec![],
                supports_streaming: false,
            }
        }
    }
    fn req() -> CompletionRequest {
        CompletionRequest {
            model: "m".into(),
            system_prompt: String::new(),
            messages: vec![ChatMessage {
                role: "user".into(),
                content: "hi".into(),
            }],
            temperature: 0.0,
            max_tokens: None,
        }
    }

    #[tokio::test]
    async fn no_limiters_never_blocks() {
        let calls = Arc::new(AtomicUsize::new(0));
        let p = RateLimitedProvider::new(
            Arc::new(CountingProvider {
                calls: calls.clone(),
            }),
            None,
            None,
        );
        let start = Instant::now();
        for _ in 0..50 {
            p.complete(req()).await.unwrap();
        }
        assert_eq!(calls.load(Ordering::SeqCst), 50);
        assert!(start.elapsed() < Duration::from_millis(200));
    }

    #[tokio::test]
    async fn shared_provider_limiter_throttles_across_decorators() {
        // Two decorators sharing ONE provider limiter (burst 2, 1/sec refill).
        let shared = Arc::new(RateLimiter::new(Rate {
            per_sec: 1.0,
            burst: 2.0,
        }));
        let calls = Arc::new(AtomicUsize::new(0));
        let a = RateLimitedProvider::new(
            Arc::new(CountingProvider {
                calls: calls.clone(),
            }),
            Some(shared.clone()),
            None,
        );
        let b = RateLimitedProvider::new(
            Arc::new(CountingProvider {
                calls: calls.clone(),
            }),
            Some(shared.clone()),
            None,
        );
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
        let agent = Arc::new(RateLimiter::new(Rate {
            per_sec: 1.0,
            burst: 1.0,
        })); // 1 then wait
        let calls = Arc::new(AtomicUsize::new(0));
        let p = RateLimitedProvider::new(
            Arc::new(CountingProvider {
                calls: calls.clone(),
            }),
            Some(provider),
            Some(agent),
        );
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
}
