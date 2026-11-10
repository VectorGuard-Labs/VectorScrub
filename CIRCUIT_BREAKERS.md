# Circuit Breakers

This document describes what exists in the code today.

## What exists

A single Python implementation, `agents/shared/circuit_breaker.py`, used by:

- `agents/discovery/agent.py` - one breaker per source host (`discovery:<host>`) around page fetches
- `agents/monitoring/agent.py` - one breaker per host (`monitor:<host>`) around verification and reappearance fetches

Nothing else is protected. There is **no** circuit breaker in the Rust backend, and none around SMTP, the California DROP API, LLM providers, or the backend API client. An earlier version of this document and of the README claimed all of those; that code was never wired in and was removed.

## Behaviour

States: `closed` (normal), `open` (calls fail immediately with `CircuitOpenError`), `half_open` (probing).

| Setting | Discovery | Monitoring |
|---------|-----------|------------|
| Failures before opening | 5 | 5 |
| Cool-down before half-open | 300 s | 300 s |
| Successes in half-open before closing | 2 | 2 |
| Per-request timeout | 60 s | 30 s |

A fetch counts as a failure when it raises, times out, or the source answers HTTP 429 or 5xx (`SourceUnhealthy`). Other responses (including 403 block pages) are not breaker failures; they are handled as inconclusive by the callers.

The registry (`get_circuit_breaker(name, config)`) is per process and in memory, so each worker process has its own breaker state and a restart resets it.

## What an open circuit means for a customer

- Discovery: that source is skipped for the scan and counted in `sources_failed`.
- Monitoring: the check is reported `inconclusive` and retried the next day. It is never reported as `removed`.

## Not done

- Breakers for SMTP, DROP, and LLM calls (those already fail closed or retry with `tenacity`, but do not trip a shared breaker).
- Shared breaker state across worker processes.
- Metrics for breaker state.

## Tests

`agents/tests/test_circuit_breaker.py` covers state transitions, async and sync execution, and the registry. Run with `cd agents && python -m pytest -q`.
