/// Rate limiting middleware configuration
///
/// Note: This is a placeholder for rate limiting configuration.
/// In production, use tower_governor or a similar crate for proper rate limiting.
/// For now, we're applying timeouts at the middleware level which provides basic protection.

pub struct RateLimiterConfig {
    /// Requests per second for general API endpoints
    pub general_rps: u64,
    /// Burst size (number of requests allowed in a burst)
    pub burst_size: u32,
}

impl Default for RateLimiterConfig {
    fn default() -> Self {
        Self {
            general_rps: 100, // 100 requests per second
            burst_size: 10,   // Allow bursts of 10 requests
        }
    }
}

// TODO: Implement proper rate limiting using tower_governor
// Currently using timeout layer as basic protection
// See: https://github.com/benwis/tower-governor for examples
