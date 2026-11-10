import asyncio
import gc
import warnings

import pytest

from shared.circuit_breaker import (
    CircuitBreaker, CircuitBreakerConfig, CircuitOpenError, CircuitState, get_circuit_breaker,
)


def cfg(**kw):
    return CircuitBreakerConfig(**{"failure_threshold": 3, "timeout_seconds": 0, "success_threshold": 2, "request_timeout_seconds": 1, **kw})


async def ok():
    return "ok"


async def boom():
    raise RuntimeError("down")


async def test_opens_after_the_failure_threshold_and_rejects_fast():
    cb = CircuitBreaker("t", cfg(timeout_seconds=3600))
    for _ in range(3):
        with pytest.raises(RuntimeError):
            await cb.execute_async(boom)
    assert cb.state is CircuitState.OPEN
    ran = []

    async def must_not_run():
        ran.append(1)

    with pytest.raises(CircuitOpenError):
        await cb.execute_async(must_not_run)
    assert ran == []


async def test_a_success_resets_the_failure_count():
    cb = CircuitBreaker("t", cfg())
    for _ in range(2):
        with pytest.raises(RuntimeError):
            await cb.execute_async(boom)
    await cb.execute_async(ok)
    for _ in range(2):
        with pytest.raises(RuntimeError):
            await cb.execute_async(boom)
    assert cb.state is CircuitState.CLOSED, "2 + 2 failures separated by a success is not 3 in a row"


async def test_recovers_through_half_open():
    cb = CircuitBreaker("t", cfg(timeout_seconds=0))
    for _ in range(3):
        with pytest.raises(RuntimeError):
            await cb.execute_async(boom)
    assert cb.state is CircuitState.OPEN
    await cb.execute_async(ok)
    assert cb.state is CircuitState.HALF_OPEN
    await cb.execute_async(ok)
    assert cb.state is CircuitState.CLOSED


async def test_a_failure_while_half_open_reopens():
    cb = CircuitBreaker("t", cfg(timeout_seconds=0))
    for _ in range(3):
        with pytest.raises(RuntimeError):
            await cb.execute_async(boom)
    with pytest.raises(RuntimeError):
        await cb.execute_async(boom)  # half-open probe fails
    assert cb.state is CircuitState.OPEN


async def test_a_slow_call_counts_as_a_failure():
    cb = CircuitBreaker("t", cfg(request_timeout_seconds=0.05, failure_threshold=1))

    async def slow():
        await asyncio.sleep(1)

    with pytest.raises(asyncio.TimeoutError):
        await cb.execute_async(slow)
    assert cb.state is CircuitState.OPEN


async def test_rejecting_a_coroutine_object_does_not_leak_it():
    cb = CircuitBreaker("t", cfg(timeout_seconds=3600, failure_threshold=1))
    with pytest.raises(RuntimeError):
        await cb.execute_async(boom)
    with warnings.catch_warnings():
        warnings.simplefilter("error")  # "coroutine ... was never awaited" would raise
        with pytest.raises(CircuitOpenError):
            await cb.execute_async(ok())  # a coroutine OBJECT, rejected by the open circuit
        gc.collect()


async def test_registry_returns_one_breaker_per_name_and_exceptions_are_not_logged_verbatim(capsys):
    assert get_circuit_breaker("x") is get_circuit_breaker("x")
    assert get_circuit_breaker("x") is not get_circuit_breaker("y")

    async def leaky():
        raise RuntimeError("failed fetching https://broker.example/people/jane-smith")

    with pytest.raises(RuntimeError):
        await CircuitBreaker("z", cfg()).execute_async(leaky)
    out = capsys.readouterr()
    assert "jane-smith" not in out.out + out.err
