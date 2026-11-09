use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// Circuit breaker state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    Closed = 0,   // Normal operation
    Open = 1,     // Circuit is open, requests fail fast
    HalfOpen = 2, // Testing if service recovered
}

impl From<u8> for CircuitState {
    fn from(value: u8) -> Self {
        match value {
            0 => CircuitState::Closed,
            1 => CircuitState::Open,
            2 => CircuitState::HalfOpen,
            _ => CircuitState::Closed,
        }
    }
}

/// Circuit breaker configuration
#[derive(Debug, Clone)]
pub struct CircuitBreakerConfig {
    /// Number of failures before opening circuit
    pub failure_threshold: u64,
    /// Duration to keep circuit open before trying half-open
    pub timeout: Duration,
    /// Number of successful requests in half-open before closing
    pub success_threshold: u64,
    /// Request timeout
    pub request_timeout: Duration,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            timeout: Duration::from_secs(60),
            success_threshold: 2,
            request_timeout: Duration::from_secs(10),
        }
    }
}

/// Circuit breaker for preventing cascading failures
pub struct CircuitBreaker {
    name: String,
    config: CircuitBreakerConfig,
    state: Arc<AtomicU8>,
    failure_count: Arc<AtomicU64>,
    success_count: Arc<AtomicU64>,
    last_failure_time: Arc<RwLock<Option<Instant>>>,
}

impl CircuitBreaker {
    /// Create a new circuit breaker
    pub fn new(name: impl Into<String>, config: CircuitBreakerConfig) -> Self {
        Self {
            name: name.into(),
            config,
            state: Arc::new(AtomicU8::new(CircuitState::Closed as u8)),
            failure_count: Arc::new(AtomicU64::new(0)),
            success_count: Arc::new(AtomicU64::new(0)),
            last_failure_time: Arc::new(RwLock::new(None)),
        }
    }

    /// Get current circuit state
    pub fn state(&self) -> CircuitState {
        CircuitState::from(self.state.load(Ordering::Relaxed))
    }

    /// Check if circuit allows requests
    pub async fn can_execute(&self) -> bool {
        match self.state() {
            CircuitState::Closed => true,
            CircuitState::Open => {
                // Check if timeout has elapsed
                if let Some(last_failure) = *self.last_failure_time.read().await {
                    if last_failure.elapsed() >= self.config.timeout {
                        // Transition to half-open
                        self.state.store(CircuitState::HalfOpen as u8, Ordering::SeqCst);
                        self.success_count.store(0, Ordering::SeqCst);
                        tracing::info!(
                            circuit_breaker = %self.name,
                            "Circuit breaker transitioning to half-open"
                        );
                        return true;
                    }
                }
                false
            }
            CircuitState::HalfOpen => true,
        }
    }

    /// Record successful execution
    pub async fn record_success(&self) {
        let current_state = self.state();

        match current_state {
            CircuitState::Closed => {
                // Reset failure count on success
                self.failure_count.store(0, Ordering::SeqCst);
            }
            CircuitState::HalfOpen => {
                let success_count = self.success_count.fetch_add(1, Ordering::SeqCst) + 1;

                if success_count >= self.config.success_threshold {
                    // Close circuit
                    self.state.store(CircuitState::Closed as u8, Ordering::SeqCst);
                    self.failure_count.store(0, Ordering::SeqCst);
                    self.success_count.store(0, Ordering::SeqCst);
                    *self.last_failure_time.write().await = None;

                    tracing::info!(
                        circuit_breaker = %self.name,
                        "Circuit breaker closed after recovery"
                    );

                    // Record metric
                    metrics::counter!(
                        "circuit_breaker_state_changes_total",
                        "circuit" => self.name.clone(),
                        "from_state" => "half_open",
                        "to_state" => "closed",
                    )
                    .increment(1);
                }
            }
            CircuitState::Open => {}
        }
    }

    /// Record failed execution
    pub async fn record_failure(&self) {
        let current_state = self.state();

        match current_state {
            CircuitState::Closed => {
                let failure_count = self.failure_count.fetch_add(1, Ordering::SeqCst) + 1;

                if failure_count >= self.config.failure_threshold {
                    // Open circuit
                    self.state.store(CircuitState::Open as u8, Ordering::SeqCst);
                    *self.last_failure_time.write().await = Some(Instant::now());

                    tracing::warn!(
                        circuit_breaker = %self.name,
                        failure_count = failure_count,
                        "Circuit breaker opened due to failures"
                    );

                    // Record metric
                    metrics::counter!(
                        "circuit_breaker_state_changes_total",
                        "circuit" => self.name.clone(),
                        "from_state" => "closed",
                        "to_state" => "open",
                    )
                    .increment(1);
                }
            }
            CircuitState::HalfOpen => {
                // Open circuit again
                self.state.store(CircuitState::Open as u8, Ordering::SeqCst);
                *self.last_failure_time.write().await = Some(Instant::now());
                self.success_count.store(0, Ordering::SeqCst);

                tracing::warn!(
                    circuit_breaker = %self.name,
                    "Circuit breaker reopened after failure in half-open state"
                );

                // Record metric
                metrics::counter!(
                    "circuit_breaker_state_changes_total",
                    "circuit" => self.name.clone(),
                    "from_state" => "half_open",
                    "to_state" => "open",
                )
                .increment(1);
            }
            CircuitState::Open => {}
        }
    }

    /// Execute a function with circuit breaker protection
    pub async fn execute<F, T, E>(&self, f: F) -> Result<T, CircuitBreakerError<E>>
    where
        F: std::future::Future<Output = Result<T, E>>,
    {
        // Check if circuit allows execution
        if !self.can_execute().await {
            metrics::counter!(
                "circuit_breaker_rejections_total",
                "circuit" => self.name.clone(),
            )
            .increment(1);

            return Err(CircuitBreakerError::CircuitOpen);
        }

        // Execute with timeout
        let result = tokio::time::timeout(self.config.request_timeout, f).await;

        match result {
            Ok(Ok(value)) => {
                self.record_success().await;
                metrics::counter!(
                    "circuit_breaker_executions_total",
                    "circuit" => self.name.clone(),
                    "result" => "success",
                )
                .increment(1);
                Ok(value)
            }
            Ok(Err(err)) => {
                self.record_failure().await;
                metrics::counter!(
                    "circuit_breaker_executions_total",
                    "circuit" => self.name.clone(),
                    "result" => "failure",
                )
                .increment(1);
                Err(CircuitBreakerError::ExecutionFailed(err))
            }
            Err(_) => {
                self.record_failure().await;
                metrics::counter!(
                    "circuit_breaker_executions_total",
                    "circuit" => self.name.clone(),
                    "result" => "timeout",
                )
                .increment(1);
                Err(CircuitBreakerError::Timeout)
            }
        }
    }

    /// Get metrics for this circuit breaker
    pub fn metrics(&self) -> CircuitBreakerMetrics {
        CircuitBreakerMetrics {
            name: self.name.clone(),
            state: self.state(),
            failure_count: self.failure_count.load(Ordering::Relaxed),
            success_count: self.success_count.load(Ordering::Relaxed),
        }
    }
}

/// Circuit breaker error
#[derive(Debug, thiserror::Error)]
pub enum CircuitBreakerError<E> {
    #[error("Circuit breaker is open")]
    CircuitOpen,

    #[error("Execution timed out")]
    Timeout,

    #[error("Execution failed: {0}")]
    ExecutionFailed(E),
}

/// Circuit breaker metrics
#[derive(Debug, Clone)]
pub struct CircuitBreakerMetrics {
    pub name: String,
    pub state: CircuitState,
    pub failure_count: u64,
    pub success_count: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_circuit_breaker_opens_after_failures() {
        let config = CircuitBreakerConfig {
            failure_threshold: 3,
            timeout: Duration::from_secs(1),
            success_threshold: 2,
            request_timeout: Duration::from_secs(1),
        };

        let cb = CircuitBreaker::new("test", config);

        assert_eq!(cb.state(), CircuitState::Closed);

        // Record failures
        cb.record_failure().await;
        cb.record_failure().await;
        assert_eq!(cb.state(), CircuitState::Closed);

        cb.record_failure().await;
        assert_eq!(cb.state(), CircuitState::Open);
    }

    #[tokio::test]
    async fn test_circuit_breaker_half_open_transition() {
        let config = CircuitBreakerConfig {
            failure_threshold: 1,
            timeout: Duration::from_millis(100),
            success_threshold: 2,
            request_timeout: Duration::from_secs(1),
        };

        let cb = CircuitBreaker::new("test", config);

        // Open circuit
        cb.record_failure().await;
        assert_eq!(cb.state(), CircuitState::Open);

        // Wait for timeout
        tokio::time::sleep(Duration::from_millis(150)).await;

        // Should allow request and transition to half-open
        assert!(cb.can_execute().await);
        assert_eq!(cb.state(), CircuitState::HalfOpen);
    }
}
