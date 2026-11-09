# Circuit Breakers

VectorScrub implements the circuit breaker pattern to prevent cascading failures and improve system resilience.

## What is a Circuit Breaker?

A circuit breaker monitors failures and prevents requests to failing services, similar to an electrical circuit breaker that cuts power when there's a fault.

### States

1. **Closed** (Normal)
   - Requests pass through normally
   - Failures are counted
   - Opens when failure threshold is reached

2. **Open** (Tripped)
   - Requests fail immediately (fail fast)
   - No requests reach the failing service
   - Transitions to half-open after timeout

3. **Half-Open** (Testing)
   - Limited requests allowed through
   - Success closes the circuit
   - Failure reopens the circuit

```
     Failures ≥ Threshold
  ┌─────────────────────────┐
  │                         │
  ▼                         │
CLOSED ────────────────► OPEN
  ▲         Timeout         │
  │      ◄──────────────────┘
  │                         │
  │    Success ≥ Threshold  │
  └─────────── HALF-OPEN ◄──┘
              Failure
```

## Usage

### Rust Backend

```rust
use crate::services::circuit_breaker::{CircuitBreaker, CircuitBreakerConfig};
use std::time::Duration;

// Create circuit breaker
let config = CircuitBreakerConfig {
    failure_threshold: 5,
    timeout: Duration::from_secs(60),
    success_threshold: 2,
    request_timeout: Duration::from_secs(10),
};

let cb = CircuitBreaker::new("external-api", config);

// Execute with protection
match cb.execute(async {
    // Your risky operation
    external_api_call().await
}).await {
    Ok(result) => {
        // Success
        println!("Result: {:?}", result);
    }
    Err(CircuitBreakerError::CircuitOpen) => {
        // Circuit is open, fail fast
        eprintln!("Service unavailable");
    }
    Err(CircuitBreakerError::Timeout) => {
        // Request timed out
        eprintln!("Request timeout");
    }
    Err(CircuitBreakerError::ExecutionFailed(err)) => {
        // Underlying error
        eprintln!("Error: {:?}", err);
    }
}
```

### Python Orchestrator

```python
from orchestrator.circuit_breaker import CircuitBreaker, CircuitBreakerConfig, CircuitOpenError

# Create circuit breaker
config = CircuitBreakerConfig(
    failure_threshold=5,
    timeout_seconds=60,
    success_threshold=2,
    request_timeout_seconds=10,
)

cb = CircuitBreaker("openai-api", config)

# Synchronous execution
try:
    result = cb.execute(lambda: risky_operation())
except CircuitOpenError:
    print("Circuit is open, using fallback")
    result = fallback_operation()

# Asynchronous execution
try:
    result = await cb.execute_async(async_risky_operation())
except CircuitOpenError:
    print("Circuit is open")
except asyncio.TimeoutError:
    print("Request timed out")
```

## Common Use Cases

### 1. External API Calls

Protect against failing LLM APIs, data brokers, or third-party services:

```python
from orchestrator.circuit_breaker import get_circuit_breaker

# Get or create circuit breaker for OpenAI
openai_cb = get_circuit_breaker("openai-api")

async def call_openai_with_protection(prompt: str):
    try:
        return await openai_cb.execute_async(
            openai.ChatCompletion.create(
                model="gpt-4",
                messages=[{"role": "user", "content": prompt}],
            )
        )
    except CircuitOpenError:
        # Fallback to Anthropic or cached response
        return await call_anthropic(prompt)
```

### 2. Database Operations

Protect against database overload:

```rust
let db_cb = CircuitBreaker::new("database", config);

let result = db_cb.execute(async {
    sqlx::query!("SELECT * FROM customers WHERE id = $1", customer_id)
        .fetch_one(&pool)
        .await
}).await?;
```

### 3. Redis Operations

Prevent cascade failures from Redis issues:

```python
redis_cb = get_circuit_breaker("redis")

def get_cached_value(key: str):
    try:
        return redis_cb.execute(lambda: redis_client.get(key))
    except CircuitOpenError:
        # Skip cache, query directly
        return query_database(key)
```

## Configuration Guidelines

### Failure Threshold

Number of consecutive failures before opening:

- **Low (3-5):** Sensitive services (payment, auth)
- **Medium (5-10):** Normal services (API calls, database)
- **High (10-20):** Tolerant services (caching, analytics)

### Timeout

Duration to keep circuit open:

- **Short (10-30s):** Fast recovery expected
- **Medium (30-60s):** Normal services
- **Long (60-300s):** Slow recovery (database restarts)

### Success Threshold

Successful requests in half-open before closing:

- **Low (1-2):** Quick recovery
- **Medium (2-5):** Cautious recovery
- **High (5-10):** Very cautious

### Request Timeout

Individual request timeout:

- **Fast (1-3s):** Real-time requirements
- **Normal (5-10s):** Standard operations
- **Slow (10-30s):** Batch operations

## Example Configurations

### LLM API (OpenAI, Anthropic)

```python
CircuitBreakerConfig(
    failure_threshold=3,  # Sensitive to failures
    timeout_seconds=30,    # Quick recovery
    success_threshold=2,   # Cautious
    request_timeout_seconds=30,  # LLM responses can be slow
)
```

### Database Queries

```rust
CircuitBreakerConfig {
    failure_threshold: 5,
    timeout: Duration::from_secs(60),
    success_threshold: 3,
    request_timeout: Duration::from_secs(5),
}
```

### Data Broker Scraping

```python
CircuitBreakerConfig(
    failure_threshold=10,  # Expect some failures
    timeout_seconds=120,   # Longer recovery
    success_threshold=3,
    request_timeout_seconds=30,
)
```

### Redis Cache

```rust
CircuitBreakerConfig {
    failure_threshold: 5,
    timeout: Duration::from_secs(10),  # Fast recovery
    success_threshold: 2,
    request_timeout: Duration::from_secs(1),  # Cache should be fast
}
```

## Monitoring

### Metrics Exposed

Circuit breakers export Prometheus metrics:

```
# State changes
circuit_breaker_state_changes_total{circuit="openai-api", from_state="closed", to_state="open"} 5

# Executions
circuit_breaker_executions_total{circuit="openai-api", result="success"} 1523
circuit_breaker_executions_total{circuit="openai-api", result="failure"} 47
circuit_breaker_executions_total{circuit="openai-api", result="timeout"} 12

# Rejections (circuit open)
circuit_breaker_rejections_total{circuit="openai-api"} 234
```

### Alerting Rules

```yaml
groups:
  - name: circuit_breakers
    rules:
      # Alert when circuit is open
      - alert: CircuitBreakerOpen
        expr: circuit_breaker_state{state="open"} == 1
        for: 1m
        annotations:
          summary: "Circuit breaker {{ $labels.circuit }} is open"

      # Alert on high rejection rate
      - alert: CircuitBreakerHighRejections
        expr: rate(circuit_breaker_rejections_total[5m]) > 10
        for: 5m
        annotations:
          summary: "Circuit {{ $labels.circuit }} rejecting many requests"
```

### Dashboard Queries

```promql
# Current state (0=closed, 1=open, 2=half-open)
circuit_breaker_state

# Failure rate
rate(circuit_breaker_executions_total{result="failure"}[5m])

# Success rate
rate(circuit_breaker_executions_total{result="success"}[5m])

# Rejection rate
rate(circuit_breaker_rejections_total[5m])
```

## Best Practices

### 1. Use Descriptive Names

```python
# Good
get_circuit_breaker("openai-gpt4-api")
get_circuit_breaker("database-customer-queries")

# Bad
get_circuit_breaker("api1")
get_circuit_breaker("db")
```

### 2. Implement Fallbacks

Always provide fallback behavior when circuit is open:

```python
try:
    result = await cb.execute_async(primary_operation())
except CircuitOpenError:
    result = fallback_operation()  # Cached data, default value, etc.
```

### 3. Log State Changes

Circuit breakers automatically log state transitions. Monitor these logs:

```
circuit_breaker_opened circuit=openai-api failure_count=5
circuit_breaker_half_open circuit=openai-api
circuit_breaker_closed_after_recovery circuit=openai-api
```

### 4. Don't Wrap Everything

Only use circuit breakers for:
- External services (APIs, databases, caches)
- Operations that can fail independently
- Services you can provide fallbacks for

Don't use for:
- Internal pure functions
- Operations that must succeed
- Critical path without fallbacks

### 5. Test Failure Scenarios

```python
# Force circuit open for testing
cb = get_circuit_breaker("test-service")
for _ in range(cb.config.failure_threshold):
    try:
        cb.execute(lambda: raise_error())
    except:
        pass

assert cb.get_state() == CircuitState.OPEN

# Verify fallback behavior
result = primary_with_fallback()
assert result == fallback_value
```

## Integration Examples

### LLM Provider with Fallback

```python
from orchestrator.circuit_breaker import get_circuit_breaker, CircuitOpenError

openai_cb = get_circuit_breaker("openai-api")
anthropic_cb = get_circuit_breaker("anthropic-api")

async def generate_text(prompt: str) -> str:
    # Try OpenAI first
    try:
        return await openai_cb.execute_async(
            openai.ChatCompletion.create(
                model="gpt-4",
                messages=[{"role": "user", "content": prompt}],
            )
        )
    except CircuitOpenError:
        # OpenAI circuit is open, try Anthropic
        try:
            return await anthropic_cb.execute_async(
                anthropic.messages.create(
                    model="claude-3-5-sonnet-20241022",
                    messages=[{"role": "user", "content": prompt}],
                )
            )
        except CircuitOpenError:
            # Both circuits open, use cached response or fail gracefully
            raise ServiceUnavailableError("All LLM providers unavailable")
```

### Database with Retry

```rust
use tokio::time::sleep;

async fn query_with_retry<T>(
    cb: &CircuitBreaker,
    query_fn: impl Fn() -> Future<Output = Result<T, sqlx::Error>>,
    max_retries: u32,
) -> Result<T, sqlx::Error> {
    let mut attempts = 0;

    loop {
        match cb.execute(query_fn()).await {
            Ok(result) => return Ok(result),
            Err(CircuitBreakerError::CircuitOpen) if attempts < max_retries => {
                attempts += 1;
                sleep(Duration::from_secs(1)).await;
                continue;
            }
            Err(err) => return Err(err.into()),
        }
    }
}
```

## Troubleshooting

### Circuit Opens Too Frequently

- Increase `failure_threshold`
- Increase `request_timeout`
- Check if service is actually failing
- Verify network/DNS issues

### Circuit Never Opens

- Decrease `failure_threshold`
- Check if errors are being caught
- Verify circuit breaker is actually being used

### Slow Recovery

- Decrease `timeout` duration
- Decrease `success_threshold`
- Implement gradual traffic ramp-up

---

**Last Updated:** 2026-11-09
