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
- Rust: use `thiserror` for error types, `tracing` for structured logging, `sqlx` for queries. Only one query (`services/audit.rs`) uses the compile-time `sqlx::query!` macro; the other ~140 are runtime-checked strings. Their safety net is the test suite: `#[sqlx::test(migrations = "./migrations")]` runs them against a real Postgres with every migration applied, so a wrong column or type fails `cargo test`, not production. New queries need such a test. Do not describe the queries as compile-time verified.
- Python: use `pydantic` for all data models, `structlog` for structured logging, `tenacity` for retries
- All API responses are JSON. Error responses are `{"message": "<human-readable string>"}`; `middleware/json_errors.rs` guarantees it for every layer (handlers, extractors, CSRF, rate limiter, timeout), so handlers may return a bare status or text and still comply.
- Internal agent-to-backend endpoints use `X-Agent-Key` header, not customer JWT.
- Privacy score formula: 100 − (critical×25 + high×10 + medium×5 + low×2), floor at 0.

## MVP Phases
- Phase 1: Discovery + Detection + Exposure Report + Dashboard — implemented
- Phase 2: Removal workflow (authorized opt-out submission + tracking) — implemented
- Phase 3: Removal verification, continuous monitoring, reappearance detection — implemented (Premium weekly, Personal monthly; Free is never monitored). Family accounts were built and then removed.

Monitoring conventions:
- A record is "verified removed" only on positive evidence. Fetch errors, blocks, CAPTCHAs, and LLM failures are *inconclusive*, never "removed".
- A reappearance alerts the customer and reactivates the exposure. It never auto-submits a removal; the customer must re-authorize (rule 5 above).
- "I did it by hand" (`POST /removal-requests/:id/done-by-hand`, `removal::mark_done_by_hand`) only moves a `failed` request to `submitted` (procedure `manual`, first check in 2 days). The customer's word is never evidence: only the independent re-check can reach `verified_removed`. A redirect during that re-check counts only if the landing path still contains the customer's surname; otherwise it is inconclusive.
- Registration requires `accepted_terms_version` equal to `TERMS_VERSION` (backend `models/customer.rs`, mirrored in `frontend/src/lib/legal.ts`); it is stored with a timestamp on `customers` (migration 0016). Bump both when /terms or /privacy change materially. The legal pages are a pilot draft not reviewed by counsel and must stay consistent with `DATA_RETENTION.md` (no self-service account deletion exists).
- Account erasure (`DELETE /me`, `vectorscrub erase-account <email>`, `services/account_deletion.rs`) pseudonymizes the append-only audit tables inside one transaction and then deletes the customer; the audit triggers permit only that clearing (migration 0017). Do not add a delete path that bypasses it, and do not log the email or IP of an erased account.
- Manual opt-outs: the customer's report (`ManualReport`: email confirmation + date) is recorded as their claim (`manual_*` columns, audit `submitted_manually`); never evidence. Re-checks follow `monitoring::VERIFICATION_SCHEDULE_DAYS` (3, 7, 14, 30, 45=timeout) and promise nothing sooner; there are no per-broker adjustments because there is no measured data. Help requests (`removal::request_assistance`, `vectorscrub assistance ...`) are a free, hand-worked pilot queue with no free-text field and no identity release.
- Listing links (`exposures.source_url`, `removal_requests.source_url`) are sealed with the customer's key (`services/url_vault.rs`, `enc1:` prefix) because some contain the customer's name. Seal in the handler that receives a link, open only in the handler that returns it (`open_all`); services pass the opaque value through (authorizing copies the sealed value). Legacy plaintext rows still read and are sealed by `backfill` at startup. Never add a response or log that emits a stored link without opening it, and never log an opened one (host only).
- Pilot tooling (`vectorscrub pilot ...`, `services/pilot.rs`, `PILOT_PLAYBOOK.md`): the scorecard keeps submitted / clearly-checked / verified apart; output never contains identity details, listing links or emails; `pilot case` is audited; `pilot_cohort` holds only a neutral letter (never a descriptive label such as a risk category). Operators guide customers; there is intentionally no operator route that releases identity details.
- Pilot controls: `MAX_ACCOUNTS` caps registrations (not atomic); `pilot_interview_ok` / `pilot_story_ok` are separate, default-false opt-ins and never imply permission to publish; the one-time $1 pilot payment is outside the subscription webhook (payment-mode checkouts are ignored by design); the home page shows the paid plan only when `SHOW_PAID_PLAN=true`. Bump `TERMS_VERSION` (backend + `frontend/src/lib/legal.ts`) when the Terms change.
- Customer-added listings (`POST /me/listings`, `services/listing.rs`): https link to a public site name only (no IP, port, credentials, local names) and to a page, not the home page (a home page never names anyone and would falsely verify as removed), sealed like any link, one per site, capped (5 Free / 25 paid). They become `exposures.origin = 'customer'` with a `failed` (manual action required) request; re-checks fetch only through `safe_get` confined to that site. They are never counted in the public verified-removal figure, and by-hand (`procedure_used = 'manual'`) requests never use up the Free opt-out allowance.
- The by-hand report may carry `listing_url` (the exact profile the customer opted out). It must pass `listing::site_of` and `removal::same_site` (same site or subdomain), is sealed in the handler, replaces the link on both the exposure and the request, and is never written to the audit trail (only `listing_link_updated: true`). `POST /removal-requests/:id/listing-link` does the same afterwards for a failed/submitted/acknowledged request (audit action `listing_link_updated`, empty details); closed requests keep their link.
- Opt-out guide figures are four separate numbers (catalogued, guided, readable now, verified) and must never be summed or presented as removal support.
- Applied migrations are never edited (sqlx checksums); schema changes go in a new migration.

Hardening conventions:
- There is no endpoint that returns a customer's identity by customer id. Agents get identity only through the scoped release endpoints (`/internal/identity`, `/internal/removal-requests/:id/identity`, `/internal/exposures/:id/identity`), which derive the customer from an actionable work item and write an audit entry (fail closed). Do not add a bulk or by-customer identity route.
- Agent key and JWT secret are at least 32 characters; compare secrets with `crypto::secrets_equal`.
- Removal status changes go through `RemovalStatus::can_transition_to` inside the locked transaction in `services/removal.rs`.
- Do not log URLs, page text, or exception text from fetched pages; log error type and host only.
- Agents fetch third-party pages only through `shared/safe_fetch.safe_get` (public addresses only, connection pinned to the checked address, redirects re-validated per hop and confined to the source domain, body size capped). Never call `httpx` with `follow_redirects=True` on a URL from a page or the database.
- The agent process must not have backend-only secrets (`MASTER_ENCRYPTION_KEY`, `JWT_SECRET`, `DATABASE_URL`, `POSTGRES_PASSWORD`); `shared/config.assert_no_backend_secrets` refuses to start if it sees one in the environment or its `.env`. Keep `agents/.env` to agent settings only.
- Audit writes that must not block a request use `AuditEntry::log_or_warn` (logged and counted in `audit_write_failures_total`), never `.ok()`. Where the audit write is the safeguard (identity release) use `.log()` and fail closed.
- Client IP (audit and rate limiting share `api/client_ip.rs`) is the TCP peer unless `TRUST_PROXY_HEADERS=true`, then the `X-Forwarded-For` entry `TRUSTED_PROXY_HOPS` places from the right. Never the leftmost entry: it is client-controlled.
- Password hashing uses the `*_async` functions in `services/auth.rs` (blocking pool, concurrency-capped), never Argon2 directly in a handler. Account emails compare case-insensitively (`lower(email)`).
- Both `audit_log` and `removal_audit_log` are guarded by triggers that reject UPDATE/DELETE/TRUNCATE (`audit_log` allows only pseudonymization).
- Scan coverage is reported per source (`checked | blocked | not_found | unavailable | unreadable | not_supported`) and shown to the customer; a scan must never read as a clean sweep of sites it could not read. Sources flagged `manual` in `discovery/agent.py` have no name-addressable search and are not fetched. Do not add code that defeats bot protection (CAPTCHA solving, disguised browsers, proxy rotation).
- The pricing page shows how many of each plan's brokers recent scans could read, from `GET /public/coverage` (measured, never hard-coded). A source with no recent measurement is not counted as readable, manual sources never are, and with no measurements the page falls back to general wording. Do not put a fixed broker count in marketing copy as if every listed broker works.
- A nightly job (`removal/health.py`, run by the orchestrator) fetches each broker's public opt-out page and reports `reachable | captcha | blocked | not_found | unavailable` to `/internal/optout-health`; the opt-out guide shows it with the date. "Reachable" means the page loaded for a plain HTTP request. It must never be presented as "removal works" or "verified working": only an actual verified removal supports that. Do not weaken it into a way past bot protection.
- The California registry importer (`agents/ca_registry/importer.py`) keeps company-level public data only. Never add contact emails, phone numbers or street addresses to its output (several are individuals'), and never use the registry's contact email as a recipient for consumer requests; the company's own privacy-request page is the channel. Treat every CSV cell as untrusted: links must be http(s) to a real hostname.
- Removal automation is honest about what blocks it. The headless browser is refused (403) or times out on several brokers' opt-out pages (observed Nov 2026: Spokeo 403; BeenVerified and AdvancedBackgroundChecks time out), and PeopleConnect's Suppression Center needs an emailed verification link only the customer can open. The executor must report a refused/dead page as such and point to the manual guide; never reword it as a layout change, and never add CAPTCHA solving, disguised browsers or inbox access to get around it. The plain-HTTP page check can say "opens" for a page the browser cannot load; marketing copy must say opt-outs are automated "where the site allows it".
- `GET /me/residence` returns only a two-letter state (decrypted on request, never stored, never the address). Do not extend it to return other address parts.
- Deletion requests to companies (`services/deletion.rs`, `agents/deletion/`) are OFF by default (`DELETION_REQUESTS_ENABLED` on the backend AND `DELETION_REQUESTS_SEND_ENABLED` on the agents) and the authorization wording is a DRAFT pending legal review: do not turn it on for real customers, and do not describe it as live. A recipient address may only come from `backend/data/broker_channels.json`, each entry carrying a quote from the company's OWN page that contains the address (checked by `broker_channels::validate` at startup and by `python -m deletion.verify_channels`). Never add an address from a third-party guide, from the registry's contact column, or one that is an EU/DPO or general support contact rather than a stated deletion-request channel. The email shares only name, first email, and the address if the customer ticked it; never phones or date of birth. A claimed request is never retried (fail closed); replies go to the customer, we do not read inboxes.
- California DROP is not integrated (no verified consumer-side API). Do not advertise it as a feature; the dashboard links residents to the state's site. Pricing copy must match what is implemented: Free = 3 automated opt-outs (`FREE_REMOVAL_LIMIT`), paid plans unlimited, Premium requests ordered first for the agents.
- `frontend/src/lib/brokers.generated.ts` is generated from `OPT_OUT_REGISTRY` (`python -m removal.frontend_export` from `agents/`); a test fails when it is stale.
- There is no HSM/KMS, no automated data retention, and no Rust circuit breaker. Do not document them as existing.
- Tests and CI: `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cd agents && python -m pytest`, frontend typecheck and build. After changing a `sqlx::query!` macro run `infra/scripts/sqlx_prepare.sh` and commit `backend/.sqlx/`. Keep `backend/Cargo.lock` tracked. Point `TEST_REDIS_URL` at a scratch database (default `/14`), never the dev one: a test deletes the scan queue key.
- Billing (Stripe, `services/billing.rs`): only a signature-verified webhook changes `subscription_tier`; the browser returning from checkout changes nothing. A plan is recomputed from the customer's entitled `stripe_subscriptions` rows, never applied as an upgrade/downgrade delta, so duplicate and out-of-order events are safe. Cancelling drops to Free at period end (`customer.subscription.deleted`). The `client_reference_id` in checkout links is HMAC-signed. `STRIPE_WEBHOOK_SECRET` is backend-only (in `BACKEND_ONLY_SECRETS`). `stripe_subscriptions` stores no personal data; do not store checkout payloads. Accounts with no Stripe subscription are never touched by the webhook.

## Subscription Tiers
All plans are per person. There is no Family tier (removed in migration 0005).

| Tier     | Price/mo | Key Features                                                      |
|----------|----------|-------------------------------------------------------------------|
| Free     | $0       | One scan/7d, 3 automated opt-outs, manual opt-out guide, no monitoring |
| Personal (sold as "Early Access") | $4.99, monthly only | Unlimited automated opt-outs, monitoring (monthly re-check); founding price locked via legacy price ids |
| Premium  | not on sale | 19 sources (Personal 15), weekly rescan and re-check, requests processed first. Hidden: no price/link configured. Its 4 extra sources are all blocked today |

## Running Locally
See README.md for setup instructions.
