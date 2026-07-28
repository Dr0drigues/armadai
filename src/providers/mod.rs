#[cfg(feature = "providers-api")]
pub mod api;
pub mod cli;
pub mod factory;
pub mod json_runner;
#[cfg(feature = "providers-api")]
pub mod proxy;
pub mod rate_limiter;

// Re-exported for `factory.rs` wiring (rate-limit Lot 1, Task 3).
pub use rate_limiter::{Rate, RateLimitedProvider, RateLimiter, shared_provider_limiter};
