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
1. Raw PII is NEVER logged, printed, or sent to external LLMs.
2. All PII in the database is encrypted with AES-256-GCM using per-customer derived keys.
3. The master encryption key lives only in the Rust process (env var / PyHSM). The Python agent layer never has it.
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
- Phase 1 (current): Discovery + Detection + Exposure Report + Dashboard
- Phase 2: Removal workflow (authorized opt-out submission + tracking)
- Phase 3: Continuous monitoring, reappearance detection, family accounts

## Subscription Tiers
| Tier     | Price/mo | Key Features                                  |
|----------|----------|-----------------------------------------------|
| Free     | $0       | One scan/24h, exposure report only            |
| Personal | $8.99    | Automated removal, continuous monitoring      |
| Premium  | $14.99   | Expanded sources, weekly rescan, evidence     |
| Family   | $24.99   | Up to 4 members                               |

## Running Locally
See README.md for setup instructions.
