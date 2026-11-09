"""
Circuit breaker pattern implementation for Python agents.

Prevents cascading failures by detecting failing services and
temporarily stopping requests to them.
"""

import asyncio
from dataclasses import dataclass
from datetime import datetime, timedelta
from enum import Enum
from typing import Callable, Optional, TypeVar, Generic

import structlog

T = TypeVar("T")


class CircuitState(Enum):
    """Circuit breaker state."""

    CLOSED = "closed"  # Normal operation
    OPEN = "open"  # Circuit is open, requests fail fast
    HALF_OPEN = "half_open"  # Testing if service recovered


@dataclass
class CircuitBreakerConfig:
    """Circuit breaker configuration."""

    failure_threshold: int = 5  # Failures before opening
    timeout_seconds: int = 60  # Seconds to wait before trying half-open
    success_threshold: int = 2  # Successes in half-open before closing
    request_timeout_seconds: int = 10  # Individual request timeout


class CircuitBreakerError(Exception):
    """Base exception for circuit breaker errors."""

    pass


class CircuitOpenError(CircuitBreakerError):
    """Exception raised when circuit is open."""

    pass


class CircuitBreaker(Generic[T]):
    """
    Circuit breaker for preventing cascading failures.

    Usage:
        cb = CircuitBreaker("my-service", config)

        # Synchronous
        result = cb.execute(lambda: risky_operation())

        # Asynchronous
        result = await cb.execute_async(async_risky_operation())
    """

    def __init__(self, name: str, config: Optional[CircuitBreakerConfig] = None):
        """
        Initialize circuit breaker.

        Args:
            name: Name of the circuit (for logging/metrics)
            config: Configuration (uses defaults if not provided)
        """
        self.name = name
        self.config = config or CircuitBreakerConfig()
        self.state = CircuitState.CLOSED
        self.failure_count = 0
        self.success_count = 0
        self.last_failure_time: Optional[datetime] = None
        self.log = structlog.get_logger().bind(circuit_breaker=name)

    def _can_execute(self) -> bool:
        """Check if circuit allows requests."""
        if self.state == CircuitState.CLOSED:
            return True

        elif self.state == CircuitState.OPEN:
            # Check if timeout has elapsed
            if self.last_failure_time:
                elapsed = (datetime.now() - self.last_failure_time).total_seconds()
                if elapsed >= self.config.timeout_seconds:
                    # Transition to half-open
                    self.state = CircuitState.HALF_OPEN
                    self.success_count = 0
                    self.log.info("circuit_breaker_half_open")
                    return True
            return False

        elif self.state == CircuitState.HALF_OPEN:
            return True

        return False

    def _record_success(self):
        """Record successful execution."""
        if self.state == CircuitState.CLOSED:
            # Reset failure count
            self.failure_count = 0

        elif self.state == CircuitState.HALF_OPEN:
            self.success_count += 1

            if self.success_count >= self.config.success_threshold:
                # Close circuit
                self.state = CircuitState.CLOSED
                self.failure_count = 0
                self.success_count = 0
                self.last_failure_time = None
                self.log.info("circuit_breaker_closed_after_recovery")

    def _record_failure(self):
        """Record failed execution."""
        if self.state == CircuitState.CLOSED:
            self.failure_count += 1

            if self.failure_count >= self.config.failure_threshold:
                # Open circuit
                self.state = CircuitState.OPEN
                self.last_failure_time = datetime.now()
                self.log.warning(
                    "circuit_breaker_opened",
                    failure_count=self.failure_count,
                )

        elif self.state == CircuitState.HALF_OPEN:
            # Open circuit again
            self.state = CircuitState.OPEN
            self.last_failure_time = datetime.now()
            self.success_count = 0
            self.log.warning("circuit_breaker_reopened_after_half_open_failure")

    def execute(self, func: Callable[[], T]) -> T:
        """
        Execute a function with circuit breaker protection.

        Args:
            func: Function to execute

        Returns:
            Function result

        Raises:
            CircuitOpenError: If circuit is open
            Exception: If function raises an exception
        """
        if not self._can_execute():
            self.log.warning("circuit_breaker_rejected_request")
            raise CircuitOpenError(f"Circuit breaker '{self.name}' is open")

        try:
            result = func()
            self._record_success()
            self.log.debug("circuit_breaker_execution_success")
            return result

        except Exception as e:
            self._record_failure()
            self.log.error("circuit_breaker_execution_failed", error=str(e))
            raise

    async def execute_async(self, coro) -> T:
        """
        Execute an async function with circuit breaker protection.

        Args:
            coro: Coroutine to execute

        Returns:
            Coroutine result

        Raises:
            CircuitOpenError: If circuit is open
            asyncio.TimeoutError: If execution times out
            Exception: If coroutine raises an exception
        """
        if not self._can_execute():
            self.log.warning("circuit_breaker_rejected_async_request")
            raise CircuitOpenError(f"Circuit breaker '{self.name}' is open")

        try:
            # Execute with timeout
            result = await asyncio.wait_for(
                coro, timeout=self.config.request_timeout_seconds
            )
            self._record_success()
            self.log.debug("circuit_breaker_async_execution_success")
            return result

        except asyncio.TimeoutError:
            self._record_failure()
            self.log.error(
                "circuit_breaker_async_execution_timeout",
                timeout=self.config.request_timeout_seconds,
            )
            raise

        except Exception as e:
            self._record_failure()
            self.log.error("circuit_breaker_async_execution_failed", error=str(e))
            raise

    def get_state(self) -> CircuitState:
        """Get current circuit state."""
        return self.state

    def get_metrics(self) -> dict:
        """Get circuit breaker metrics."""
        return {
            "name": self.name,
            "state": self.state.value,
            "failure_count": self.failure_count,
            "success_count": self.success_count,
            "last_failure_time": (
                self.last_failure_time.isoformat() if self.last_failure_time else None
            ),
        }

    def reset(self):
        """Manually reset circuit breaker to closed state."""
        self.state = CircuitState.CLOSED
        self.failure_count = 0
        self.success_count = 0
        self.last_failure_time = None
        self.log.info("circuit_breaker_manually_reset")


# Global circuit breaker registry
_circuit_breakers: dict[str, CircuitBreaker] = {}


def get_circuit_breaker(
    name: str, config: Optional[CircuitBreakerConfig] = None
) -> CircuitBreaker:
    """
    Get or create a circuit breaker by name.

    Args:
        name: Circuit breaker name
        config: Configuration (only used if creating new circuit)

    Returns:
        CircuitBreaker instance
    """
    if name not in _circuit_breakers:
        _circuit_breakers[name] = CircuitBreaker(name, config)

    return _circuit_breakers[name]


def get_all_circuit_breakers() -> dict[str, CircuitBreaker]:
    """Get all registered circuit breakers."""
    return _circuit_breakers.copy()


def reset_all_circuit_breakers():
    """Reset all circuit breakers to closed state."""
    for cb in _circuit_breakers.values():
        cb.reset()
