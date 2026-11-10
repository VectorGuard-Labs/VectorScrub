# VectorScrub — Project Steering

## Product
VectorScrub is an AI-powered personal data protection service (B2C) under VectorGuard Labs.
Tagline: "Your personal information. Under your control."

## Architecture
- **Backend**: Rust (Axum, SQLx, AES-256-GCM) — owns PII, encryption, auth, audit trail
- **AI Agents**: Python (LangChain, LangGraph) — discovery, detection, removal, monitoring
- **Frontend**: Next.js 15 + Tailwind CSS 4 — customer dashboard
- **Database**: PostgreSQL 16
- **Queue**: Redis (scan job dispatch between Rust and Python)

## Critical Security Rules
1. Raw PII is NEVER logged, printed, or sent to external LLMs. Enforced in code for LLMs: all model calls go through `DetectionAgent._run_classifier`, which applies `agents/shared/pii_guard.py` (redact the customer's values and other personal data, send no identity and no URLs, then fail closed if any customer value remains). Third parties' names are removed too (spaCy NER in `shared/pii_ner.py`, then a deny-by-default vocabulary in `shared/pii_vocab.py`). When editing the vocabulary, keep it free of anything that is also a given name or surname: re-run the intersection with name corpora. Never add another LLM call site; never log listing URLs (they embed names), log the host instead.
2. All PII in the database is encrypted with AES-256-GCM using per-customer derived keys.
3. The master encryption key lives only in the Rust process (env var; no HSM/KMS integration yet). The Python agent layer never has it.
4. The Python agent layer calls back to the Rust internal API to fetch minimal decrypted fields on a need-to-know basis.
5. Removal requests require explicit customer authorization before submission.
6. The audit log is append-only. No updates or deletes.
7. Customer PII is never used to train AI models.

## Development Conventions
- Rust: use `thiserror` for error types, `tracing` for structured logging, `sqlx` with compile-time query verification
- Python: use `pydantic` for all data models, `structlog` for structured logging, `tenacity` for retries
- All API responses are JSON. Error responses include a human-readable string message.
- Internal agent-to-backend endpoints use `X-Agent-Key` header, not customer JWT.
- Privacy score formula: 100 − (critical×25 + high×10 + medium×5 + low×2), floor at 0.

## MVP Phases
- Phase 1: Discovery + Detection + Exposure Report + Dashboard — implemented
- Phase 2: Removal workflow (authorized opt-out submission + tracking) — implemented
- Phase 3: Removal verification, continuous monitoring, reappearance detection — implemented (Premium weekly, Personal monthly; Free is never monitored). Family accounts were built and then removed.

Monitoring conventions:
- A record is "verified removed" only on positive evidence. Fetch errors, blocks, CAPTCHAs, and LLM failures are *inconclusive*, never "removed".
- A reappearance alerts the customer and reactivates the exposure. It never auto-submits a removal; the customer must re-authorize (rule 5 above).
- Applied migrations are never edited (sqlx checksums); schema changes go in a new migration.

Hardening conventions:
- There is no endpoint that returns a customer's identity by customer id. Agents get identity only through the scoped release endpoints (`/internal/identity`, `/internal/removal-requests/:id/identity`, `/internal/exposures/:id/identity`), which derive the customer from an actionable work item and write an audit entry (fail closed). Do not add a bulk or by-customer identity route.
- Agent key and JWT secret are at least 32 characters; compare secrets with `crypto::secrets_equal`.
- Removal status changes go through `RemovalStatus::can_transition_to` inside the locked transaction in `services/removal.rs`.
- Do not log URLs, page text, or exception text from fetched pages; log error type and host only.
- There is no HSM/KMS, no automated data retention, and no Rust circuit breaker. Do not document them as existing.
- Tests and CI: `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cd agents && python -m pytest`, frontend typecheck and build. After changing a `sqlx::query!` macro run `infra/scripts/sqlx_prepare.sh` and commit `backend/.sqlx/`. Keep `backend/Cargo.lock` tracked.

## Subscription Tiers
All plans are per person. There is no Family tier (removed in migration 0005).

| Tier     | Price/mo | Key Features                                                      |
|----------|----------|-------------------------------------------------------------------|
| Free     | $0       | One scan/24h, exposure report only, no monitoring                 |
| Personal | $8.99    | Automated removal, continuous monitoring (monthly re-check)       |
| Premium  | $14.99   | Expanded sources, weekly rescan and re-check, evidence            |

## Running Locally
See README.md for setup instructions.
