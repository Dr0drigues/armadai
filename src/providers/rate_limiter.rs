use std::sync::Mutex;
use std::time::{Duration, Instant};

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
    /// Create a rate limiter allowing `max_per_minute` requests per minute.
    pub fn new(max_per_minute: u32) -> Self {
        let max = max_per_minute as f64;
        Self {
            state: Mutex::new(BucketState {
                tokens: max,
                max_tokens: max,
                refill_rate: max / 60.0,
                last_refill: Instant::now(),
            }),
        }
    }

    /// Wait until a token is available, then consume it.
    pub async fn acquire(&self) {
        loop {
            let wait = {
                let mut state = self.state.lock().unwrap();
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

    /// Parse a rate limit string like "10/min", "60/hour", "5/sec".
    /// Returns requests per minute.
    pub fn parse_rate(rate_str: &str) -> Option<u32> {
        let (count_str, unit) = rate_str.split_once('/')?;
        let count: u32 = count_str.trim().parse().ok()?;
        match unit.trim() {
            "s" | "sec" | "second" => Some(count * 60),
            "m" | "min" | "minute" => Some(count),
            "h" | "hr" | "hour" => Some(count.max(1) / 60),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_rate_formats() {
        assert_eq!(RateLimiter::parse_rate("10/min"), Some(10));
        assert_eq!(RateLimiter::parse_rate("1/sec"), Some(60));
        assert_eq!(RateLimiter::parse_rate("60/hour"), Some(1));
        assert_eq!(RateLimiter::parse_rate("5/m"), Some(5));
        assert_eq!(RateLimiter::parse_rate("invalid"), None);
    }

    #[tokio::test]
    async fn acquire_within_limit() {
        let limiter = RateLimiter::new(60); // 1 per second
        let start = Instant::now();
        limiter.acquire().await;
        limiter.acquire().await;
        // Two immediate acquires should work since bucket starts full
        assert!(start.elapsed() < Duration::from_millis(100));
    }

    #[tokio::test]
    async fn acquire_waits_when_exhausted() {
        let limiter = RateLimiter::new(60); // 1 per second

        // Drain all tokens
        for _ in 0..60 {
            limiter.acquire().await;
        }

        let start = Instant::now();
        limiter.acquire().await;
        // Should have waited ~1 second for a refill
        assert!(start.elapsed() >= Duration::from_millis(900));
    }
}
