#[cfg(feature = "providers-api")]
pub mod api;
pub mod cli;
pub mod factory;
#[cfg(feature = "providers-api")]
pub mod proxy;
pub mod rate_limiter;
pub mod traits;

// Re-exported for `factory.rs` wiring (rate-limit Lot 1, Task 3); not yet
// consumed on this branch, hence the allow.
#[allow(unused_imports)]
pub use rate_limiter::{Rate, RateLimitedProvider, RateLimiter, shared_provider_limiter};
