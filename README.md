# VectorScrub

> **Your personal information. Under your control.**

VectorScrub is an AI-powered personal privacy protection platform built for everyday people. It automatically discovers where your personal information is exposed on the internet, identifies exactly what has been leaked, helps you remove it, and monitors for it coming back.

Built by **VectorGuard Labs** — the same team behind PyHSM, VectorLedger, and Vercre.

---

## Table of Contents

1. [What Is VectorScrub?](#what-is-vectorscrub)
2. [The Problem It Solves](#the-problem-it-solves)
3. [What VectorScrub Does](#what-vectorscrub-does)
4. [Benefits to End Users](#benefits-to-end-users)
5. [How the AI Works](#how-the-ai-works)
6. [Architecture Overview](#architecture-overview)
7. [Security Model](#security-model)
8. [Subscription Plans](#subscription-plans)
9. [Project Structure](#project-structure)
10. [Prerequisites](#prerequisites)
11. [Local Development Setup](#local-development-setup)
12. [Environment Variables](#environment-variables)
13. [Running Each Service](#running-each-service)
14. [API Reference](#api-reference)
15. [Database Schema](#database-schema)
16. [MVP Roadmap](#mvp-roadmap)
17. [California DROP Integration](#california-drop-integration)
18. [Legal and Operational Risk](#legal-and-operational-risk)
19. [Scan Job Recovery](#scan-job-recovery)
20. [Testing](#testing)
21. [Production Deployment](#production-deployment)
22. [Competitive Landscape](#competitive-landscape)
23. [VectorGuard Labs Product Family](#vectorguard-labs-product-family)
24. [Recent Updates](#recent-updates)
25. [Contributing](#contributing)
26. [License](#license)

---

## What Is VectorScrub?

VectorScrub is a B2C SaaS privacy protection service. Customers create an account, enter their personal information (name, address, email, phone number), and VectorScrub's AI agents go to work — scanning people-search engines, data broker websites, and public directories to find everywhere that information appears online.

Once exposures are discovered, VectorScrub classifies each one by type and severity, prepares authorized removal requests, submits them to the data sources, and tracks whether those records are actually taken down. The platform then continues monitoring on a scheduled basis, alerting customers if removed information ever reappears.

The result is a continuously updated **Privacy Score** and a full dashboard showing customers exactly what was found, what has been removed, what is still pending, and where they stand overall.

VectorScrub is designed from the ground up to be more intelligent, more transparent, and more security-focused than existing data removal services — and to sit naturally alongside the broader VectorGuard Labs trust and identity infrastructure.

---

## The Problem It Solves

Most people have no idea how exposed their personal information is online.

Right now, for the average person:

- Their **full name and home address** are publicly listed on people-search sites like Spokeo, WhitePages, and BeenVerified
- Their **phone number** is available through public directories without their knowledge or consent
- Their **email address** appears in dozens of data broker databases, making them a target for spam and phishing
- Their **family members and associates** may be listed alongside their own record, exposing others without consent
- Their **past addresses, employment history, and vehicle records** may be surfaced by background-check sites
- Their information may have been included in one or more **data breaches**, spreading their details further across the web

The traditional options are bad. Manually opting out of each data broker requires finding every site, creating accounts, filling out forms, and re-checking months later when records reappear. It takes dozens of hours and needs to be repeated indefinitely.

Existing paid services help, but most are essentially automated form-submitters. They lack real AI reasoning, provide limited transparency about what was actually verified as removed, and offer no meaningful security posture around the sensitive identity data customers hand them.

**VectorScrub is built to fix all of that.**

---

## What VectorScrub Does

VectorScrub delivers six core capabilities:

### 1. AI Personal Data Discovery
Specialized discovery agents search supported people-search engines, data broker websites, and public directories using the customer's identity profile. Sources are scanned concurrently and results are ranked by confidence.

### 2. PII Detection and Classification
A detection agent analyzes each discovery candidate using a large language model. It classifies what type of personal information is exposed (name, address, phone, email, date of birth, family associations, employment history, etc.) and assigns a severity level (low, medium, high, or critical) based on the combination of fields present.

**Critical security note — no customer PII ever reaches an LLM, and this is enforced in code.** Every model call goes through one gateway (`DetectionAgent._run_classifier`) that applies `agents/shared/pii_guard.py` first:

- The customer's identity is **never sent, not even masked hints**. Matching against their name (in any ordering or case, with or without accents), addresses, phones, emails, and date of birth happens locally.
- The broker-page excerpt is **redacted** before the model sees it. The customer's values become typed placeholders (`[CUSTOMER_NAME]`, `[CUSTOMER_ADDRESS]`, `[CUSTOMER_PHONE]`, `[CUSTOMER_EMAIL]`, `[CUSTOMER_DOB]`). Other personal data on the page, such as other people's emails, phones, street addresses, ZIP codes, dates, ages, and SSNs, becomes generic placeholders (`[EMAIL]`, `[PHONE]`, ...). Person names that are not the customer's (relatives, neighbours, namesakes) become `[NAME]` via a local spaCy NER pass. The model judges the match from how the placeholders relate, plus yes/no flags computed locally.
- **Deny by default.** After redaction, every remaining word that is not on a short list of structural words (`age`, `born`, `lives`, `phone`, `relatives`, `no results found`, ... in `agents/shared/pii_vocab.py`) is dropped, in any script and any casing; a run of dropped words becomes one `…`. This, not NER, is what keeps third-party names out: pattern rules and NER are probabilistic, the vocabulary filter is not. A bonus: free text on a hostile page (such as "ignore previous instructions") is dropped too.
- The page **URL is not sent** (profile URLs commonly contain the person's name).
- **Fail closed:** immediately before the call, the excerpt is re-checked twice: no customer value may remain, and nothing outside the allowed vocabulary may remain. Either failure blocks the call with `OutboundPiiError` and no request is made. If the NER model is required (the default) but cannot be loaded, the call is blocked too. In monitoring, a blocked call counts as *inconclusive*, never "removed".
- The masked summary shown on the dashboard (e.g. `J*** S***, 1** M*** St`) is built locally from the match evidence; the model never writes it.

**What is guaranteed, and how it was measured.** Customer values cannot reach the model: they are removed by pattern rules and the fail-closed check blocks the call if one survives. Third-party names cannot reach the model unless they are spelled exactly like one of the 436 allowed structural words, and that list is kept free of names: it is intersected with the name data of every locale the Faker library ships (about 49,000 distinct name words) and any collision is removed. On 2,500 generated pages with names from 21 locales (including CJK and Arabic scripts) in mixed casings and "Last, First" order, **no name survived the full pipeline**, whereas patterns plus NER *without* the vocabulary filter let a name through on 60% of those pages. For US-style names the same comparison was: normal capitalization 1.0% (patterns) → 0.0% (with filter); lowercase 100% (patterns) → 20.1% (patterns + NER) → 0.0% (with filter).

**What this costs.** The model sees much less text, so detection runs on structure ("`[CUSTOMER_NAME]` near `[CUSTOMER_ADDRESS]`, relatives `[NAME]`, `[NAME]`") rather than prose. That is enough for the match decision in the scenarios tested, but accuracy against real broker pages and a real model has not been measured: compare detection results before and after on a sample of real scans before relying on it. Redaction adds about 50 ms per 3,000-character page (three NER passes over the original, title-cased, and lowercased text) and runs off the event loop.

**Operating it.** NER is a runtime dependency: `spacy` and the `en_core_web_sm` model are pinned in `agents/requirements.txt` (the model wheel is hash-pinned) and install with the rest. Settings: `PII_NER_REQUIRED` (default `true`; if the model cannot load, LLM calls are blocked) and `PII_NER_MODEL` (default `en_core_web_sm`). Setting `PII_NER_REQUIRED=false` is for local development only; it keeps the vocabulary filter, so nothing unsafe is sent, but the model then sees `…` where it would have seen `[NAME]`.

Remaining limits: a person whose name is spelled exactly like an allowed structural word would pass through that word alone; and the vocabulary list is curated for English-language pages, so non-English structural words are dropped (safe, but the model loses that signal).

### 3. Automated Removal Request Submission
For each confirmed exposure, VectorScrub identifies the correct opt-out procedure for that specific data source — web form, email submission, or the California DROP API. After explicit customer authorization, VectorScrub automatically submits removal requests using Playwright browser automation for web forms, capturing screenshots as evidence and extracting confirmation references.

### 4. Removal Tracking and Verification
Every removal request is tracked through its full lifecycle: authorized → submitted → (acknowledged) → verified removed. VectorScrub distinguishes between these states clearly so customers always know the true status of their records — not just that a request was sent, but whether removal was actually confirmed. All actions are logged to an immutable, append-only audit trail enforced at the database level.

"Verified removed" is earned, not assumed. Once a request's check date arrives, the monitoring agent re-fetches the source page and reports one of three results: the record is **gone** (HTTP 404/410, or the customer no longer appears on the page), **still there**, or **inconclusive** (blocked, CAPTCHA, fetch error, unreadable page, or an LLM failure). Only "gone" marks the request verified. "Still there" is re-checked weekly; "inconclusive" is retried the next day. A request that never verifies within 45 days of submission ends as `rejected` (the source did not honor it) or `unverifiable` (we could not tell), and the customer is alerted. A request whose automated submission could not be completed ends as `failed` and can be re-authorized.

### 5. Continuous Monitoring
After a removal is verified, VectorScrub re-checks the source on the customer's plan schedule — **Premium weekly, Personal monthly** (Free accounts are not monitored). If the record has come back (which happens frequently as data brokers re-acquire information), the exposure is reactivated, the privacy score is recalculated, the removal request is marked `reappeared`, and the customer is alerted.

**Reappearance never triggers an automatic re-submission.** Every removal needs explicit customer authorization, so the alert asks the customer to review and re-authorize from the dashboard. Reappearance alerts use a stricter confidence bar (0.7) than discovery (0.5) to keep false alarms down.

### 6. Privacy Score and Exposure Report
Customers see a live **Privacy Score** (0–100) that reflects their current exposure level. The score decreases based on the severity and volume of active exposures, and increases as records are removed. A full exposure history shows every finding, removal, and reappearance over time.

---

## Benefits to End Users

### Know exactly what's out there
Most people discover they have dozens of active exposures on their first scan. VectorScrub surfaces the full picture in a single dashboard rather than requiring customers to check each site manually.

### Get it removed — with proof
VectorScrub prepares and submits opt-out requests to data brokers using intelligent browser automation. Every submission is captured with screenshot evidence (SHA-256 hashed for tamper-resistance), confirmation references are extracted, and all actions are logged to an immutable audit trail. Customers see a clear before-and-after record, not just a promise that "we tried."

### Stop it from coming back
Data brokers regularly re-acquire and republish information even after opt-outs. Continuous monitoring catches a returning record and alerts you, so you don't have to keep re-checking the sites yourself. You stay in control: re-removal happens only after you re-authorize it.

### Plans are per person
Every plan covers one person. There is no shared or family plan: each family member who wants protection has their own account, their own encrypted profile, and their own Privacy Score.

### Know your data is protected — for real
VectorScrub encrypts all customer identity data with AES-256-GCM using a unique cryptographic key derived for each customer. The key material never leaves the Rust backend process. Customer PII is never used to train AI models. The audit log is tamper-evident and append-only.

### Understand your exposure trend
The Privacy Score tracks over time, so customers can see the concrete impact of removals and understand whether their exposure footprint is growing or shrinking.

### Support for California DROP
California residents can take advantage of the state's official Delete Request and Opt-Out Platform (DROP), which covers 600+ registered data brokers in a single submission. VectorScrub integrates DROP as one removal pathway alongside its own direct opt-out coverage.

---

## How the AI Works

VectorScrub uses a pipeline of specialized AI agents rather than a single model doing everything. Each agent has a narrow, well-defined responsibility.

```
Customer Identity Profile (encrypted)
            │
            ▼
  ┌─────────────────────┐
  │   Orchestrator      │  Pulls scan jobs from Redis queue,
  │                     │  coordinates agent pipeline, polls for
  │                     │  authorized removal requests (dual workers)
  └────────┬────────────┘
           │
           ▼
  ┌─────────────────────┐
  │   Discovery Agent   │  Searches people-search sites and
  │                     │  public directories for candidate URLs
  └────────┬────────────┘
           │  candidate URLs + text snippets
           ▼
  ┌─────────────────────┐
  │   Detection Agent   │  LLM classifies each candidate:
  │                     │  PII category, severity, confidence
  │                     │  (LLM sees only redacted text — no customer PII)
  └────────┬────────────┘
           │  confirmed exposures
           ▼
  ┌─────────────────────┐
  │   Removal Agent     │  Looks up opt-out procedure for each
  │                     │  source, prepares authorized requests
  └────────┬────────────┘
           │  customer authorizes via dashboard
           ▼
  ┌─────────────────────┐
  │  Removal Executor   │  Submits via Playwright (web forms),
  │                     │  email, or DROP API; captures evidence
  └────────┬────────────┘
           │  verification + monitoring (migration 0006)
           ▼
  ┌─────────────────────┐
  │  Monitoring Agent   │  Re-checks removed records on schedule,
  │                     │  detects reappearance
  └─────────────────────┘
           │
           ▼
  Rust Backend API  ←──  agents call back via internal HTTP endpoints
           │
           ▼
  PostgreSQL (exposures, scan jobs, removal requests, audit log)
```

The Rust backend owns all PII. The Python agent layer never has the master encryption key. When an agent needs identity data to run a search, it requests the minimum necessary fields from a Rust internal endpoint — the Rust process decrypts and returns only what that specific operation requires.

---

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                        VectorScrub Platform                      │
│                                                                  │
│  ┌────────────────┐       ┌──────────────────────────────────┐  │
│  │  Next.js 15    │──────▶│  Rust API (Axum)                 │  │
│  │  Dashboard     │       │  Auth · Encryption · Audit Trail │  │
│  └────────────────┘       └─────────────┬────────────────────┘  │
│                                         │  Redis scan queue      │
│                           ┌─────────────▼────────────────────┐  │
│                           │  Python Agent Orchestrator        │  │
│                           │  Discovery → Detection →          │  │
│                           │  Removal → Monitoring             │  │
│                           └──────────────────────────────────┘  │
│                                                                  │
│  PostgreSQL 16 (encrypted PII, exposures, audit log)             │
│  Redis 7 (scan job queue)                                        │
└─────────────────────────────────────────────────────────────────┘
```

| Layer      | Technology                                      | Responsibility                               |
|------------|-------------------------------------------------|----------------------------------------------|
| Backend    | Rust · Axum · SQLx · AES-256-GCM                | API, auth (Argon2id + JWT), PII encryption, audit trail |
| Agents     | Python · LangChain · httpx · Playwright         | Discovery, detection, removal, monitoring    |
| Frontend   | Next.js 15.5.29 · Tailwind CSS 4.1.8 · TypeScript 5.8.3 | Landing page, auth, customer dashboard       |
| Database   | PostgreSQL 16                                   | All persistent state — PII stored encrypted  |
| Queue      | Redis 7                                         | Scan job dispatch from Rust to Python        |

**Why Rust for the backend?** Memory safety, no garbage collector, predictable performance under load, and a strong cryptography ecosystem. The backend handles every operation that touches raw customer PII — authentication, encryption, database writes, and the audit trail.

**Why Python for the agents?** The LLM orchestration ecosystem (LangChain, LangGraph) is mature and moves quickly in Python. Web scraping and browser automation tools (Playwright, BeautifulSoup) are best-in-class. Rapid iteration on prompts and agent logic is straightforward. The Python layer never holds encryption keys.

---

## Security Model

Security is the core architectural constraint, not an afterthought. Every design decision in VectorScrub is made with the understanding that customers are handing over exactly the information they want to protect.

| Concern | How VectorScrub handles it |
|---|---|
| PII at rest | AES-256-GCM encryption with a per-customer key derived from the master key as SHA-256(master ‖ label ‖ customer id). (Earlier docs said HMAC-SHA256; it is a keyed hash, not an HMAC or HKDF. Changing it would make stored data undecryptable without a migration.) One customer's ciphertext cannot be decrypted with another's key. Known limit: no AEAD associated data, so a blob moved between two fields of the same customer would still decrypt. |
| Master key | Loaded once into the Rust process from the `MASTER_ENCRYPTION_KEY` environment variable and validated at startup (the process refuses to start if it is not 32 base64 bytes). Never written to disk, never accessible to the Python agent layer. **There is no HSM or KMS integration**: a PyHSM module and its documentation existed but were never wired in (and put a key-store password in the agent process, against this rule), so they were removed. For production, inject the key from a secrets manager or KMS. |
| PII and LLMs | No customer PII and no third-party names are ever sent to an LLM, enforced in code. All model calls go through one gateway that redacts the customer's values (patterns), detects other people's names (local spaCy NER), drops every word outside a short structural vocabulary, sends no identity (not even masked hints) and no URLs, and re-checks the final excerpt, blocking the call if anything unsafe remains (`agents/shared/pii_guard.py`, `pii_ner.py`, `pii_vocab.py`). See [How the AI Works](#how-the-ai-works). |
| Removal authorization | No removal request is ever submitted to a data source without an explicit customer authorization recorded in the database with timestamp. The agent prepares; the customer approves; the executor submits. |
| Audit trail | Two append-only logs. `removal_audit_log` records every action on a removal request (PostgreSQL rules turn UPDATE/DELETE into no-ops). `audit_log` records registration, login (success, failure, lockout, with client IP), identity submission, **every release of decrypted identity to the agents (and every refusal)**, scan start/recovery/failure, and removal authorization. `audit_log` is enforced append-only by database triggers (migration 0007): edits, deletes and TRUNCATE fail loudly; the only permitted change is clearing `customer_id`/`user_email` (pseudonymization). This protects against the application and ordinary SQL, not against a database superuser or table owner, who could drop the trigger; real tamper evidence against them needs an external write-once sink or a hash chain, which does not exist. |
| Evidence collection | Screenshot evidence captured during web form submission is SHA-256 hashed. The hash is stored in the database; screenshots can be stored in encrypted S3 for retrieval. Hashes prove authenticity without storing large binary data. |
| Model training | Customer PII is never used to fine-tune or train any AI model — not VectorScrub's, not a provider's. |
| Agent authentication and scoping | Internal endpoints accept only the `X-Agent-Key` header (compared in constant time, length-hiding), not customer JWTs; the key must be 32+ characters or the backend will not start. The key is **not a skeleton key**: decrypted identity is released only against a concrete, currently-actionable work item, and the customer is derived from that item, never supplied by the caller. A scan needs the job-scoped token minted for exactly that job (AES-GCM under the customer's key, 24 h TTL) and the job must be queued or running; a removal request must be `authorized` or due for verification; an exposure must be due for a reappearance check for an eligible customer. Every release and every refusal is audited, and a release fails if its audit write fails. There is no "identity by customer id" endpoint. Remaining limit: the key is still one shared secret, and a release contains every identity field because redaction and form filling need them. |
| Tenant isolation | Per-customer encryption keys mean a compromise of one customer's encrypted data does not affect others. |
| Password storage | Argon2id with random per-user salt. No plaintext or reversibly encoded passwords stored anywhere. |
| **Production Security** | **Additional hardening for production deployments** |
| CSRF protection | Double-submit cookie on all state-changing requests (POST/PUT/DELETE; login/register and agent endpoints are exempt). The server issues a URL-safe token at `/api/v1/csrf-token`, sets it as the `csrf_token` cookie (`Secure` in production, `SameSite=Strict`), and requires the same value in `X-CSRF-Token`, compared in constant time. The frontend sends it automatically through `csrfFetch` (`frontend/src/lib/csrf.ts`). |
| Security headers | CSP, HSTS, X-Frame-Options, X-Content-Type-Options, Referrer-Policy configured via dedicated middleware. |
| Rate limiting and lockout | Per-client-IP token buckets (tower_governor): a strict one on login and register (default 5/minute, burst 5) and a general one on the customer API (20/s, burst 60); 429 responses carry `Retry-After`. Internal agent routes and health probes are exempt. Limits are in-memory per backend instance. Client IP is the TCP peer unless `TRUST_PROXY_HEADERS=true` (only behind a proxy that overwrites `X-Forwarded-For`). Separately, 5 failed logins for an email lock it for 15 minutes, doubling every 5 further failures up to an hour; keyed on a hash of the email in Postgres so a non-existent email locks exactly like a real one, and unknown emails still pay for one Argon2 verification so response time does not reveal which emails are registered. Trade-off: someone who knows a victim's email can lock them out for up to an hour; the per-IP limit bounds how fast one source can do it. Request timeout is 30 s. |
| Circuit breakers | Per-host breakers (`agents/shared/circuit_breaker.py`) around the discovery and monitoring page fetches: after 5 failures or 429/5xx answers a host is left alone for 5 minutes, and those checks come back inconclusive instead of piling on. **Not covered**: the opt-out form submission, SMTP, and the DROP API. |
| Input validation | Request bodies are validated with `validator` (Rust) and `pydantic` (Python); callback ranges and lengths are bounded; agent writes are checked for ownership (an exposure can only be recorded against a running job of that customer; an audit entry only against the request's real owner). Errors that are not the caller's fault return a generic 500 and are logged server-side. A separate Python "PII validation framework" existed but was never connected and was removed. |
| Request tracing | Unique request IDs tracked across all services for security auditing and debugging. |
| Metrics and observability | Prometheus metrics exported for monitoring request latency, error rates, database pool stats, and custom business metrics. |
| Data retention | **Not implemented, apart from two housekeeping jobs**: expired login-throttle rows and privacy-score snapshots older than a year are pruned. There is no automated customer, exposure, scan or audit deletion and no right-to-erasure workflow. A module that claimed to do this existed but was never run and would have failed (foreign keys into the immutable removal audit log); it was removed. See `DATA_RETENTION.md` for the open design question. Do not describe this product as GDPR/CCPA-compliant on retention until that exists. |
| SOC 2 | `SOC2_COMPLIANCE.md` is a target-state policy framework, **not** an audit or attestation, and it now opens with a status table verified against the code. |
| Scan reliability | A scan job survives a crashed worker: workers claim a job atomically, send heartbeats, and fence every update with an attempt number; a backend recovery loop requeues silent jobs, re-dispatches jobs whose queue entry was lost, and fails jobs that keep dying (alerting the customer). See [Scan job recovery](#scan-job-recovery). |

---

## Subscription Plans

| Plan     | Price/month | Key Features |
|----------|-------------|--------------|
| **Free**     | $0     | Initial exposure scan, privacy score, exposure report |
| **Personal** | $8.99  | Automated removal requests, continuous monitoring (monthly re-check), monthly rescan, alerts |
| **Premium**  | $14.99 | Everything in Personal + expanded source coverage, priority removal, weekly rescan and weekly reappearance re-check, evidence collection |

All plans are per person. A Family tier ($24.99, up to 4 members) existed in earlier versions and was removed; migration `0005_remove_family_tier.sql` moved any existing Family customers to Premium.

Free accounts are limited to one scan per 24 hours and are never rescanned or monitored automatically. Paid accounts can initiate multiple scans per day and have access to a wider range of data sources depending on tier.

---

## Project Structure

```
VectorScrub/
│
├── backend/                          # Rust API service
│   ├── Cargo.toml                    # Dependencies and build config
│   ├── migrations/
│   │   ├── 0001_initial_schema.sql   # Full PostgreSQL schema
│   │   ├── 0002_removal_requests_index.sql  # Performance index for removal polling
│   │   ├── 0003_phase3_features.sql  # Monitoring, family (now unused), alerts
│   │   ├── 0004_audit_log.sql        # Comprehensive audit logging
│   │   ├── 0005_remove_family_tier.sql  # Drops the 'family' tier; plans are per person
│   │   ├── 0006_monitoring_support.sql  # 'failed' removal status, monitoring on by default, verification index
│   │   ├── 0007_security_hardening.sql  # login_throttle table, audit_log immutability triggers
│   │   └── 0008_scan_job_recovery.sql   # scan_jobs attempts / heartbeat_at / dispatched_at
│   ├── .sqlx/                        # Offline query metadata (commit this; see "Regenerating .sqlx")
│   └── src/
│       ├── main.rs                   # Entry point, server setup
│       ├── db.rs                     # Shared AppState (pool, redis, config)
│       ├── test_support.rs           # Test-only helpers (throwaway DB, fixtures)
│       ├── docs.rs                   # OpenAPI documentation
│       ├── observability.rs          # Prometheus metrics and tracing
│       ├── api/
│       │   ├── mod.rs                # Route table
│       │   ├── auth.rs               # POST /auth/register, /auth/login
│       │   ├── customers.rs          # GET /me, POST /me/identity
│       │   ├── scans.rs              # POST /scans, GET /scans, GET /scans/:id
│       │   ├── exposures.rs          # GET /exposures, GET /exposures/summary
│       │   ├── removals.rs           # GET /removal-requests, POST /removal-requests/:id/authorize
│       │   ├── agent_callbacks.rs    # Internal (X-Agent-Key): claim, scan status, exposures, scoped identity release
│       │   ├── health.rs             # GET /health/live, /health/ready
│       │   ├── client_ip.rs          # Client IP for rate limiting / lockout (proxy headers opt-in)
│       │   ├── error.rs              # JSON error responses (internal details never returned)
│       │   ├── csrf.rs               # GET /csrf-token for CSRF protection
│       │   └── metrics.rs            # GET /metrics for Prometheus scraping
│       ├── crypto/
│       │   └── mod.rs                # AES-256-GCM encrypt/decrypt, key derivation
│       ├── middleware/
│       │   ├── mod.rs                # CORS, tracing, timeout, request IDs
│       │   ├── auth_extractor.rs     # JWT extractor, agent key extractor
│       │   ├── csrf.rs               # CSRF protection middleware
│       │   ├── rate_limiter.rs       # Request rate limiting
│       │   ├── security_headers.rs   # Security headers (CSP, HSTS, etc.)
│       │   └── metrics_middleware.rs # Request metrics collection
│       ├── models/
│       │   ├── customer.rs           # Customer, IdentityProfile, CleartextIdentity
│       │   ├── exposure.rs           # Exposure, ExposureSummary
│       │   ├── scan_job.rs           # ScanJob, ScanJobPayload
│       │   ├── removal_request.rs    # RemovalRequest, RemovalAuditEntry
│       │   ├── privacy_score.rs      # Privacy score history and trends
│       │   ├── notification.rs       # Notification preferences
│       │   └── alert.rs              # Customer alerts
│       ├── services/
│       │   ├── auth.rs               # Argon2id hashing, JWT issue/validate
│       │   ├── config.rs             # AppConfig from env vars
│       │   ├── customer.rs           # Account creation, identity encrypt/decrypt
│       │   ├── exposure.rs           # Record exposures, privacy score calculation
│       │   ├── removal.rs            # Removal request lifecycle, audit log
│       │   ├── scan.rs               # Scan job lifecycle, Redis dispatch
│       │   ├── audit.rs              # Comprehensive audit logging service
│       │   ├── identity_access.rs    # Scan tokens + scoped, audited identity release to agents
│       │   ├── login_throttle.rs     # Per-email login lockout
│       │   ├── privacy_score.rs      # Privacy score history tracking
│       │   ├── monitoring.rs         # Removal verification + reappearance monitoring state
│       │   └── notification.rs       # Alert and notification service
│       └── jobs/
│           ├── scheduler.rs          # Scheduled rescan dispatcher
│           └── recovery.rs           # Requeue/fail/redispatch stuck scans; housekeeping
│
├── agents/                           # Python AI agent layer
│   ├── requirements.txt              # Runtime dependencies (hash-pinned spaCy model)
│   ├── requirements-dev.txt          # pytest etc.
│   ├── tests/                        # pytest suite (PII guard, orchestrator, monitoring, executor, ...)
│   ├── shared/
│   │   ├── config.py                 # pydantic-settings config
│   │   ├── models.py                 # Shared Pydantic data models
│   │   ├── backend_client.py         # HTTP client → Rust internal API
│   │   ├── pii_guard.py              # Redaction + vocabulary filter + fail-closed checks before any LLM call
│   │   ├── pii_ner.py                # Local spaCy NER: finds third-party person names
│   │   ├── pii_vocab.py              # The only words allowed to reach the LLM (kept free of known names)
│   │   ├── page_text.py              # Page text extraction / block-page detection for discovery
│   │   └── circuit_breaker.py        # Per-host circuit breaker used by discovery and monitoring
│   ├── discovery/
│   │   └── agent.py                  # Scans 8+ data broker sources per tier
│   ├── detection/
│   │   └── agent.py                  # LLM-based PII classification; the only LLM call site
│   ├── removal/
│   │   ├── agent.py                  # Opt-out procedure registry + request preparation
│   │   └── executor.py               # Playwright automation, email/DROP submission
│   ├── monitoring/
│   │   └── agent.py                  # Reappearance detection for removed records
│   └── orchestrator/
│       └── orchestrator.py           # Redis worker: claim, heartbeat, pipeline, monitoring loop
│
├── frontend/                         # Next.js 15 customer dashboard
│   ├── package.json
│   ├── next.config.ts                # API proxy rewrites
│   ├── postcss.config.mjs            # Tailwind CSS 4 PostCSS plugin config
│   ├── tailwind.config.ts            # Tailwind CSS 4 configuration
│   ├── tsconfig.json
│   └── src/
│       ├── app/
│       │   ├── layout.tsx            # Root layout, metadata
│       │   ├── globals.css           # Tailwind CSS 4 base styles
│       │   ├── page.tsx              # Public landing page
│       │   ├── register/page.tsx     # Account creation
│       │   ├── login/page.tsx        # Sign in
│       │   ├── dashboard/
│       │   │   ├── page.tsx          # Privacy dashboard (authenticated)
│       │   │   ├── profile/page.tsx  # Identity profile management
│       │   │   └── removals/page.tsx # Removal request tracking
│       ├── lib/
│       │   ├── api.ts                # Typed API client (all endpoints)
│       │   ├── csrf.ts               # csrfFetch: attaches X-CSRF-Token to POST/PUT/DELETE
│       │   └── utils.ts              # Privacy score labels, severity colors, formatters
│       └── types/
│           └── index.ts              # TypeScript interfaces for all API types
│
├── infra/
│   ├── docker/
│   │   ├── docker-compose.yml        # Full stack: Postgres, Redis, backend, agents, frontend
│   │   ├── Dockerfile.backend        # Multi-stage Rust build
│   │   ├── Dockerfile.agents         # Python + Playwright
│   │   └── Dockerfile.frontend       # Next.js standalone build
│   └── scripts/
│       ├── gen_keys.sh               # Generates JWT secret, master key, agent key, DB/Redis passwords
│       └── sqlx_prepare.sh           # Regenerates / checks backend/.sqlx
│
├── .github/workflows/ci.yml          # CI: backend, agents, frontend, image builds
├── .env.example                      # All required environment variables with descriptions
├── .gitignore                        # Excludes .env, target/, node_modules/, __pycache__/
├── .kiro/steering/vectorscrub.md     # Project conventions for Kiro AI sessions
├── README.md                         # This file
├── CIRCUIT_BREAKERS.md               # Per-host circuit breaker (Python agents)
├── DATA_RETENTION.md                 # Retention: what runs today, and what is still open
├── OBSERVABILITY.md                  # Metrics, logging, and monitoring guide
├── P1_IMPROVEMENTS.md                # Priority 1 feature enhancements
├── P2_IMPROVEMENTS.md                # Priority 2 feature enhancements
├── SECURITY_FIXES.md                 # P0 security improvements implemented
└── SOC2_COMPLIANCE.md                # SOC 2 Type II compliance framework
```

---

## Prerequisites

Before setting up VectorScrub locally, make sure you have the following installed:

| Tool | Version | Purpose |
|------|---------|---------|
| Rust | 1.82+ | Backend compilation (`rustup` recommended) |
| Python | 3.11+ | AI agent layer |
| Node.js | 22 LTS | Next.js frontend |
| Docker | 24+ | PostgreSQL and Redis |
| Docker Compose | 2.20+ | Multi-service orchestration |
| OpenSSL | Any | Key generation script |
| Chromium | Latest | Playwright browser automation (installed via `playwright install chromium`) |

**Install Rust** (if not already installed):
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

**Install Node.js** (if not already installed):
```bash
# macOS with Homebrew
brew install node
```

---

## Local Development Setup

### Step 1 — Clone and enter the project

```bash
cd ~/VectorScrub   # or wherever you cloned it
```

### Step 2 — Generate cryptographic secrets

VectorScrub requires a JWT signing secret and a 32-byte AES-256-GCM master encryption key. Never use placeholder values in any environment.

```bash
bash infra/scripts/gen_keys.sh
```

This prints four values: `JWT_SECRET`, `MASTER_ENCRYPTION_KEY`, `POSTGRES_PASSWORD`, and `REDIS_PASSWORD`. Copy them — you will need them in the next step.

### Step 3 — Create your .env file

```bash
cp .env.example .env
```

Open `.env` and fill in the values from Step 2. See [Environment Variables](#environment-variables) for a full description of every variable.

### Step 4 — Start PostgreSQL and Redis

```bash
cd infra/docker
docker compose up postgres redis -d
```

Both services have health checks. Wait until `docker compose ps` shows them as `healthy` before continuing.

```bash
docker compose ps
# NAME                       STATUS
# vectorscrub-postgres       running (healthy)
# vectorscrub-redis          running (healthy)
```

### Step 5 — Start the Rust backend

The backend automatically applies all database migrations on startup.

```bash
cd backend
cargo run
```

On first run, Cargo downloads and compiles all dependencies (this takes a few minutes). Subsequent runs start in seconds. You should see:

```
{"level":"INFO","message":"Database migrations applied successfully"}
{"level":"INFO","message":"VectorScrub API listening on 0.0.0.0:8080"}
```

**Note**: Phase 2 added a new migration (`0002_removal_requests_index.sql`) for removal request polling performance. If you're upgrading from Phase 1, this migration will be applied automatically on the next backend start.

### Step 6 — Start the Python agent worker

Open a new terminal tab for this.

```bash
cd agents
python3 -m venv .venv
source .venv/bin/activate
pip install -r requirements.txt

# Install Playwright browsers for removal automation
playwright install chromium

python -m orchestrator.orchestrator
```

The worker connects to Redis and blocks waiting for scan jobs. It now runs two concurrent loops: one for scan jobs, one for removal execution:

```
{"event":"orchestrator_worker_started","queue":"vectorscrub:scan_jobs"}
{"event":"removal_worker_started"}
```

### Step 7 — Start the frontend

Open another terminal tab.

```bash
cd frontend
npm install
npm run dev
```

Open [http://localhost:3000](http://localhost:3000) in your browser. You should see the VectorScrub landing page.

### Step 8 — Test the full removal workflow (optional)

To verify the end-to-end removal workflow:

1. **Register and submit identity**: Create an account at http://localhost:3000/register, then submit your identity profile at http://localhost:3000/dashboard/profile
2. **Run a scan**: Click "Start New Scan" and wait for the orchestrator to process it (watch the orchestrator logs)
3. **View exposures**: Once the scan completes, you'll see discovered exposures on your dashboard
4. **Authorize removal**: Click the "Remove" button on any active exposure
5. **Watch orchestrator logs**: The removal worker polls every 60 seconds. You'll see:
   ```
   {"event":"processing_removals","count":1}
   {"event":"submit_web_form","opt_out_url":"https://..."}
   {"event":"web_form_submitted","confirmation_ref":"...","evidence_hash":"..."}
   ```
6. **View removal status**: Navigate to the "Removals" page to see the status change from `authorized` → `submitted`
7. **Check audit trail**: Each action is logged to `removal_audit_log` with actor and timestamp

---

## Environment Variables

All variables are defined in `.env.example`. Copy it to `.env` before starting any service.

| Variable | Required | Description |
|----------|----------|-------------|
| `DATABASE_URL` | Yes | PostgreSQL connection string. Example: `postgres://vectorscrub:password@localhost:5432/vectorscrub` |
| `POSTGRES_PASSWORD` | Yes | Password for the PostgreSQL user (used by Docker Compose) |
| `REDIS_URL` | Yes | Redis connection string. Example: `redis://localhost:6379` |
| `REDIS_PASSWORD` | Yes | Password for Redis (used by Docker Compose) |
| `JWT_SECRET` | Yes | Random string used to sign JWTs, at least 32 characters (the backend refuses to start otherwise). Generate with `openssl rand -base64 48` |
| `MASTER_ENCRYPTION_KEY` | Yes | Base64-encoded 32-byte AES master key, validated at startup. Generate with `openssl rand -base64 32`. Read from the environment only; there is no HSM/KMS integration. Held by the Rust backend only. |
| `PORT` | No | Port for the Rust API (default: `8080`) |
| `ENVIRONMENT` | No | `development`, `staging`, or `production` (default: `development`) |
| `DB_POOL_SIZE` | No | PostgreSQL connection pool size (default: `10`) |
| `ALLOWED_ORIGINS` | No | Comma-separated list of allowed CORS origins (default: `http://localhost:3000`) |
| `AGENT_API_KEY` | Yes | Shared secret for Python agent → Rust internal API authentication, at least 32 characters. Needed by both the backend and the agents. |
| `TRUST_PROXY_HEADERS` | No | `true` only behind a proxy that overwrites `X-Forwarded-For` / `X-Real-IP`; used to identify clients for rate limiting (default: `false`, uses the socket address) |
| `RATE_LIMIT_GENERAL_PER_SECOND` / `RATE_LIMIT_GENERAL_BURST` | No | Per-IP limit for customer routes (defaults: `20`, `60`) |
| `RATE_LIMIT_AUTH_PER_MINUTE` / `RATE_LIMIT_AUTH_BURST` | No | Per-IP limit for login and register (defaults: `5`, `5`) |
| `SCAN_RECOVERY_INTERVAL_SECS` | No | How often the recovery loop runs (default: `60`) |
| `SCAN_STALE_AFTER_SECS` | No | A running scan with no heartbeat for this long is requeued (default: `600`) |
| `SCAN_REDISPATCH_AFTER_SECS` | No | A queued scan nobody claimed for this long is pushed to Redis again (default: `600`) |
| `SCAN_MAX_ATTEMPTS` | No | Attempts before a scan is marked `failed` and the customer is alerted (default: `3`) |
| `TEST_REDIS_URL` | Tests only | Redis URL for the Rust API tests (default `redis://127.0.0.1:6379/14`). A Redis server must be reachable; the tests only delete the scan queue key in that database, but use a scratch database anyway. |
| `BACKEND_API_URL` | No | URL the Python agents use to reach the Rust API (default: `http://localhost:8080`) |
| `OPENAI_API_KEY` | Recommended | OpenAI API key for the detection agent. One of OpenAI or Anthropic required. |
| `ANTHROPIC_API_KEY` | Optional | Anthropic API key as an alternative to OpenAI |
| `DEFAULT_LLM_PROVIDER` | No | `openai` or `anthropic` (default: `anthropic`) |
| `DEFAULT_LLM_MODEL` | No | Model name for the LLM provider (e.g., `claude-3-5-sonnet-20241022`, `gpt-4o`) |
| `NEXT_PUBLIC_API_URL` | No | URL the Next.js frontend uses for API calls (default: `http://localhost:8080`) |
| **Observability** | | **Metrics and Tracing** |
| `OTLP_ENDPOINT` | Optional | OpenTelemetry OTLP endpoint for distributed tracing (e.g., `http://localhost:4317`). Leave empty to disable. Metrics are always available at `GET /api/v1/metrics`. |
| **Agent Pacing** | | **Monitoring and removal workers (Python agents)** |
| `MONITORING_BATCH_SIZE` | No | Verification / reappearance targets fetched per cycle (default: `25`; the backend caps requests at 100) |
| `MONITORING_IDLE_SECONDS` | No | How long the monitoring worker sleeps when nothing is due (default: `3600`) |
| `MONITORING_REQUEST_DELAY_SECONDS` | No | Pause between page fetches while monitoring (default: `2.0`) |
| `REMOVAL_REQUEST_DELAY_SECONDS` | No | Pause between opt-out submissions to third-party sites (default: `5.0`) |
| `PII_NER_REQUIRED` | No | `true` (default): if the spaCy NER model cannot be loaded, LLM calls are blocked (fail closed). `false`: local development only; continues on the vocabulary filter alone. |
| `PII_NER_MODEL` | No | spaCy model used to detect third-party person names (default: `en_core_web_sm`, installed from `requirements.txt`) |
| **Phase 3 Variables** | | **Email and DROP Integration** |
| `SMTP_HOST` | Optional | SMTP server host for email opt-out requests (default: `smtp.gmail.com`) |
| `SMTP_PORT` | Optional | SMTP server port (default: `587`) |
| `SMTP_USERNAME` | Optional | SMTP authentication username |
| `SMTP_PASSWORD` | Optional | SMTP authentication password |
| `SMTP_FROM_EMAIL` | Optional | From address for opt-out emails |
| `SMTP_USE_TLS` | Optional | Enable TLS for SMTP (default: `true`) |
| `DROP_API_URL` | Optional | California DROP API endpoint (default: `https://privacy.ca.gov/api/v1`) |
| `DROP_API_KEY` | Optional | DROP API key for California residents |
| `DROP_API_SECRET` | Optional | DROP API secret |
| `DROP_ENABLE_SANDBOX` | Optional | Use DROP sandbox environment (default: `true`) |

---

## Running Each Service

### Rust Backend

```bash
# Development (with auto-reload via cargo-watch — optional)
cd backend
cargo install cargo-watch   # one-time
cargo watch -x run

# Production build
cargo build --release
./target/release/vectorscrub
```

### Python Agents

```bash
cd agents
source .venv/bin/activate

# Run the orchestrator worker (runs both scan and removal workers)
python -m orchestrator.orchestrator

# Run individual agents in isolation for testing
python -c "
import asyncio
from discovery.agent import DiscoveryAgent
from shared.models import IdentityContext
import uuid

identity = IdentityContext(
    customer_id=uuid.uuid4(),
    first_name='Jane',
    last_name='Smith',
    full_name='Jane Smith',
    email='jane@example.com',
    current_address='123 Main Street, Los Angeles, CA',
    email_addresses=['jane@example.com'],
    phone_numbers=['555-123-4567'],
)
results = asyncio.run(DiscoveryAgent(tier='personal').run(identity))
print(f'Found {len(results)} candidates')
"

# Test removal executor (without submitting)
python -c "
import asyncio
from removal.executor import RemovalExecutor
from removal.agent import RemovalAgent
from shared.models import IdentityContext
import uuid

executor = RemovalExecutor()
agent = RemovalAgent()

# Get opt-out procedure for a known source
procedure = agent.get_procedure('spokeo.com')
print(f'Procedure type: {procedure.procedure_type}')
print(f'Opt-out URL: {procedure.opt_out_url}')
print(f'Estimated days: {procedure.estimated_days}')
"
```

### Frontend

```bash
cd frontend

# Development with hot reload
npm run dev

# Type-check without building
npm run typecheck

# Production build
npm run build
npm start
```

### Full Stack with Docker

To run the entire stack in containers:

```bash
cd infra/docker
docker compose up --build
```

All five services (PostgreSQL, Redis, backend, agents, frontend) start together. The backend waits for PostgreSQL to be healthy before starting.

---

## API Reference

All endpoints are prefixed with `/api/v1`. Customer endpoints require a `Bearer` JWT in the `Authorization` header. Internal agent endpoints require an `X-Agent-Key` header.

### Authentication

| Method | Endpoint | Description |
|--------|----------|-------------|
| `POST` | `/auth/register` | Create a new account. Body: `{ email, password }`. Returns `{ token, customer }`. |
| `POST` | `/auth/login` | Sign in. Body: `{ email, password }`. Returns `{ token, customer }`. After 5 consecutive failures for an email the endpoint returns `429` with a `Retry-After` header (15 min, doubling every further 5 failures, capped at 60 min). Unknown emails lock out identically, so the endpoint does not reveal which accounts exist. |

Login and register are limited to 5 requests/minute per IP, and all customer routes to 20/second (burst 60). Exceeding a limit returns `429` with `Retry-After`. Internal agent routes and health checks are not rate limited.

### Customer Profile

| Method | Endpoint | Auth | Description |
|--------|----------|------|-------------|
| `GET` | `/me` | JWT | Returns the authenticated customer's profile and privacy score. |
| `POST` | `/me/identity` | JWT | Submit or update the identity profile (name, address, phones, emails). PII is encrypted immediately. |

### Scans

| Method | Endpoint | Auth | Description |
|--------|----------|------|-------------|
| `POST` | `/scans` | JWT | Queue a new scan job. Returns `202` with `{ job_id, status, message }`. `422` if no identity profile has been submitted, `429` if the plan's scan limit is reached. If Redis is unreachable the job is still created and the recovery loop dispatches it. |
| `GET` | `/scans` | JWT | List the 20 most recent scan jobs for the authenticated customer. |
| `GET` | `/scans/:job_id` | JWT | Get the status and progress of a specific scan job. |

### Exposures

| Method | Endpoint | Auth | Description |
|--------|----------|------|-------------|
| `GET` | `/exposures` | JWT | List all exposures (active and removed) for the authenticated customer. |
| `GET` | `/exposures/summary` | JWT | Aggregated summary: counts by severity, privacy score, affected sources, last scan date. |

### Removal Requests

| Method | Endpoint | Auth | Description |
|--------|----------|------|-------------|
| `GET` | `/removal-requests` | JWT | List all removal requests for the authenticated customer. |
| `POST` | `/removal-requests/:exposure_id/authorize` | JWT | Authorize a removal request for a specific exposure you own. Creates audit trail entry. Rejected only while another request for it is in flight. |
| `GET` | `/removal-requests/:id` | JWT | Get a specific removal request with full audit log. |

### Privacy Score History (Phase 3)

| Method | Endpoint | Auth | Description |
|--------|----------|------|-------------|
| `GET` | `/privacy-score/history?days=30` | JWT | Retrieve privacy score trend with historical snapshots and delta calculations. |

### Notifications and Alerts (Phase 3)

| Method | Endpoint | Auth | Description |
|--------|----------|------|-------------|
| `GET` | `/notifications/preferences` | JWT | Get customer's notification preferences for email and SMS alerts. |
| `PUT` | `/notifications/preferences` | JWT | Update notification preferences. Body: `{ email_enabled, sms_enabled, phone_number, alert_*  }`. |
| `GET` | `/notifications/alerts` | JWT | Get unread alerts for the customer (new exposures, failures, reappearances). |
| `POST` | `/notifications/alerts/:id/read` | JWT | Mark a specific alert as read. |

### Internal Agent Endpoints

All require `X-Agent-Key` (compared in constant time). Agents never receive a customer's identity in bulk: identity is released one request at a time, only for a work item that is actually actionable, and every release is written to `audit_log` (the release fails if the audit write fails).

| Method | Endpoint | Description |
|--------|----------|-------------|
| `POST` | `/internal/scan/:job_id/claim` | Claim a queued scan. Returns `{ attempt }`, or `409` if the job is not claimable (already claimed, finished, or unknown). The 24-hour scan token travels in the Redis queue payload, not in this response. |
| `PUT` | `/internal/scan/status` | Update scan progress / heartbeat. Body includes `attempt`; a stale attempt (a worker whose job was requeued) gets `409`. |
| `POST` | `/internal/identity` | Body: `{ customer_id, job_id, identity_token }`. Releases the identity for a running scan whose token matches. |
| `POST` | `/internal/exposures` | Record a confirmed exposure found by the detection agent. |
| `GET` | `/internal/removal-requests` | Poll for authorized removal requests ready for execution. |
| `POST` | `/internal/removal-requests/:id/identity` | Release identity for one authorized, not-yet-submitted removal request. The customer is derived from the request. |
| `PUT` | `/internal/removal-requests/:id/status` | Update status. Only legal transitions are accepted (`409` otherwise, `404` if unknown). |
| `POST` | `/internal/removal-requests/:id/audit` | Append an audit entry, attributed to the request's real owner. |
| `GET` | `/internal/removal-requests/due-for-verification?limit=N` | Submitted/acknowledged requests whose check time has passed (default 25, max 100). |
| `POST` | `/internal/removal-requests/:id/verification-result` | Body: `{ customer_id, outcome }`, `outcome` = `removed` \| `still_present` \| `inconclusive`. `404` if unknown, `409` if no longer submitted/acknowledged. |
| `GET` | `/internal/exposures/due-for-monitoring?limit=N` | Removed exposures due for a reappearance check: Premium after 7 days, Personal after 30, never Free or monitoring-disabled customers. |
| `POST` | `/internal/exposures/:id/identity` | Release identity for one exposure that is due for a monitoring check. |
| `POST` | `/internal/exposures/:id/monitoring-result` | Body: `{ customer_id, outcome }`, `outcome` = `clear` \| `reappeared` \| `inconclusive`. `404` if unknown, `409` if the exposure is already active. |

There is no endpoint that returns a customer's identity by customer id; the earlier `GET /internal/customers/:id/identity` was removed because a leaked agent key could read every customer with it.

### Security and Observability

| Method | Endpoint | Auth | Description |
|--------|----------|------|-------------|
| `GET` | `/csrf-token` | None | Get CSRF token for state-changing requests. Sets cookie and returns token in response body. |
| `GET` | `/metrics` | None | Prometheus metrics endpoint. Returns metrics in Prometheus exposition format. |
| `GET` | `/health/live` | None | Liveness probe. Returns 200 if process is running. |
| `GET` | `/health/ready` | None | Readiness probe. Returns 200 if database and Redis are accessible. |

---

## Database Schema

The PostgreSQL schema is defined across eight migrations and applied automatically on backend startup. Applied migrations are never edited (sqlx verifies their checksums); changes go in a new migration.
- `0001_initial_schema.sql` - Core schema (Phase 1 & 2)
- `0002_removal_requests_index.sql` - Performance optimization (Phase 2)
- `0003_phase3_features.sql` - Monitoring, alerts, score history (and the now-unused `family_members` table)
- `0004_audit_log.sql` - Comprehensive audit logging for production (Production hardening)
- `0005_remove_family_tier.sql` - Removes the `family` subscription tier (plans are per person)
- `0006_monitoring_support.sql` - Adds the `failed` removal status, turns `monitoring_enabled` on by default, indexes verification lookups
- `0007_security_hardening.sql` - `login_throttle` table; triggers that make `audit_log` append-only (the only permitted change is clearing `customer_id` / `user_email`)
- `0008_scan_job_recovery.sql` - `scan_jobs.attempts`, `heartbeat_at`, `dispatched_at`

| Table | Purpose |
|-------|---------|
| `customers` | Account records: email, hashed password, subscription tier, privacy score, monitoring_enabled |
| `identity_profiles` | Per-customer encrypted PII: all fields stored as AES-256-GCM ciphertext blobs |
| `scan_jobs` | Scan lifecycle: queued, running, completed, failed; sources and exposures counted; `attempts`, `heartbeat_at`, `dispatched_at` drive [recovery](#scan-job-recovery) |
| `login_throttle` | Failed-login counters keyed by SHA-256 of the lower-cased email (no plaintext emails, unknown emails included) |
| `audit_log` | Append-only audit trail for security-relevant events, including each identity release. Triggers reject UPDATE/DELETE (a database superuser can still disable them). |
| `exposures` | Every discovered PII exposure: source, category, severity, masked summary, active/removed state, last_monitored_at |
| `removal_requests` | Opt-out requests: status lifecycle `authorized → submitted → acknowledged → verified_removed`, plus terminal `rejected`, `unverifiable`, `failed`, and `reappeared` |
| `removal_audit_log` | Append-only tamper-evident log of every action on every removal request |
| `alerts` | Customer notifications: new exposures, removal failures, reappearances |
| `family_members` | **(Unused)** Created by migration 0003; family plans were removed and the table is no longer written |
| `privacy_score_history` | **(Phase 3)** Historical privacy score snapshots with exposure breakdowns for trend analysis |
| `notification_preferences` | **(Phase 3)** Per-customer alert preferences: email/SMS toggles, phone number, alert types |

**Privacy Score Formula:**
```
score = max(0, 100 - (critical × 25 + high × 10 + medium × 5 + low × 2))
```

A customer with no active exposures scores 100. Each critical exposure (e.g., full name + full address + phone) deducts 25 points.

---

## MVP Roadmap

### Phase 1 — Find My Information ✅ Complete

- Customer account creation and authentication
- Encrypted identity profile submission
- AI-powered discovery across 8+ data broker sources (tier-gated)
- LLM-based PII detection on redacted page text (no customer PII sent to the model)
- Exposure dashboard with privacy score
- Scan job lifecycle management

### Phase 2 — Remove My Information ✅ Complete

- Customer authorization workflow with database audit trail
- Opt-out form submission via Playwright browser automation
- Generic web form detection and field population
- Screenshot capture and SHA-256 evidence hashing
- Confirmation reference extraction from success pages
- Removal request status tracking (authorized → submitted → verified_removed)
- Dual-worker orchestrator (scans + removal jobs in parallel)
- Immutable append-only audit log (enforced by PostgreSQL rules)
- Removal tracking dashboard showing all request statuses
- Email-based opt-out (stub for Phase 3 completion)
- California DROP API integration (stub for Phase 3 completion)

### Phase 3 — Keep Me Protected ✅ Complete

- ✅ Scheduled rescans (weekly for Premium, monthly for Personal)
- ✅ Removal verification: submitted requests are re-checked and only marked `verified_removed` on positive evidence
- ✅ Reappearance detection: Premium weekly, Personal monthly; alerts the customer and reactivates the exposure
- ✅ Re-removal after a reappearance, with fresh customer authorization (never automatic)
- ✅ Honest failure reporting for automated submissions (`failed` status, CAPTCHA stop, confirmation required)
- ✅ Privacy score trend charts with historical analysis
- ✅ Email and SMS alert system with customizable preferences
- ✅ Exposure history timeline with event tracking
- ✅ Complete email-based opt-out integration
- ✅ California DROP API integration (600+ brokers)
- ✅ Privacy score history API with trend analysis
- ✅ Notification preferences and alert management
- ➖ Family accounts: built, then removed. All plans are per person.

Known limits of the monitoring implementation are listed under [Legal and Operational Risk](#legal-and-operational-risk).

---

## California DROP Integration

California's [Delete Request and Opt-Out Platform (DROP)](https://privacy.ca.gov/data-deletion/) launched in January 2026. It allows eligible California residents to submit a single deletion request to more than 600 registered data brokers simultaneously. Data brokers began processing DROP requests on August 1, 2026.

VectorScrub's Phase 2 roadmap includes DROP integration as a first-class removal pathway for California residents — handling the identity verification flow, submitting requests through the official platform, and tracking responses alongside VectorScrub's own direct opt-out coverage.

DROP is free to use directly, which creates an important product consideration: VectorScrub must deliver meaningful ongoing value beyond what DROP provides in a single submission. That value comes from:

- Discovery and coverage outside the DROP ecosystem
- Continuous monitoring for reappearance
- Multi-state coverage (not just California)
- Sources that are exempt from DROP or require individual opt-outs
- A unified privacy dashboard with a clear score and history
- Per-person accounts, so each family member gets their own profile, score, and removal history

---

## Legal and Operational Risk

This section describes what the automated opt-out and monitoring code does and does not do today, and what still needs a decision from the business or from counsel. It is an engineering document, not legal advice.

### Operational: what the code now guarantees

- **No false "submitted".** A web-form submission is reported as `submitted` only if the resulting page shows a confirmation reference or acceptance wording and no error wording. If no form field matched, no submit control was found, or the result is ambiguous, the request ends as `failed` with a reason, and the customer is alerted and can re-authorize. A failure is terminal for that attempt: it is not retried automatically.
- **No false "removed".** A request becomes `verified_removed` only on positive evidence (page gone, customer's surname absent from the page, or a clear LLM non-match). Fetch errors, HTTP 403/429/5xx, CAPTCHA or bot-wall pages, pages too thin to judge, and LLM failures are all *inconclusive*, retried the next day, and never reported as removal.
- **No CAPTCHA circumvention.** If an opt-out page presents a CAPTCHA the request fails with an explicit reason. The agent does not attempt to solve or evade it.
- **Pacing.** The removal worker waits 5 seconds between submissions and the monitoring worker 2 seconds between page fetches (`REMOVAL_REQUEST_DELAY_SECONDS`, `MONITORING_REQUEST_DELAY_SECONDS`). A request is never submitted twice by the same worker process.
- **Always customer-authorized.** Removals, including re-removals after a reappearance, are submitted only after the customer authorizes them in the dashboard.

### Operational: known limits

- **Form drift is expected.** Submission uses generic field selectors, not per-site handlers (`OPT_OUT_REGISTRY` stores only a URL and procedure type). Brokers change their forms; those requests will fail until a site-specific handler exists. Failure is now visible rather than silent, but it is still a failure.
- **Email-verification links are not clicked.** Several brokers (per the registry notes, for example Spokeo, Radaris, and TruePeopleSearch) email the customer a link that must be clicked to complete the opt-out. VectorScrub does not do this, so such requests will show as `submitted` and then fail verification after 45 days. Until this is handled in the product, customers need to be told to watch for and click those emails.
- **ID-verification sources.** Registry entries with `requires_id_verification=True` (for example TruthFinder, MyLife, and the California DROP entry) are flagged but not enforced; the agent cannot supply identity documents, so those requests are expected to fail verification.
- **CAPTCHA sources have no manual path yet.** The request fails; there is no hand-off workflow for a human to complete it.
- **Verification is a heuristic.** It relies on a plain HTTP fetch, a local surname check, and an LLM verdict on a short excerpt. JavaScript-rendered pages are inconclusive. A broker that serves bots a page without the record while showing it to people could cause a false "removed"; reappearance monitoring is the backstop.
- **California DROP is unverified against the real API.** The request payload in `executor.py` is documented in the code as "based on typical government API patterns" and must be checked against the published DROP specification before it is used outside the sandbox (`DROP_ENABLE_SANDBOX` defaults to `true`).
- **Duplicate-submission guard is per process.** It is held in memory, so a worker restart between a successful submission and its status update could resubmit.
- **Third-party details on pages.** The LLM receives redacted page excerpts (1,500 characters in discovery, 3,000 in monitoring), reduced to placeholders plus a short structural vocabulary, so neither the customer's nor other people's names reach it; see the guarantee, measurements, and remaining limits under [How the AI Works](#how-the-ai-works). Not yet measured: detection accuracy on real broker pages with a real model now that it sees structure instead of prose.

### Legal: open items for counsel and the product owner

- **Terms of service.** Broker sites may prohibit automated access in their terms. Opt-out forms exist for consumers, but driving them with a bot on a customer's behalf, and fetching pages weekly or monthly to monitor them, may breach those terms or draw other claims. This needs a source-by-source review and a decision on which sources to automate versus guide the customer through manually.
- **Identification of the crawler.** Neither the monitoring fetcher (which sends a desktop-browser `User-Agent`) nor the Playwright sessions (headless Chrome) identify VectorScrub. Whether to identify the service honestly (and accept more blocking), and whether to honor `robots.txt`, is a policy decision that has not been made.
- **Authorized-agent status.** Privacy laws such as CCPA let a consumer use an authorized agent, and brokers may demand proof of that authorization. The current opt-out email is written in the customer's first person and sent from VectorScrub's SMTP account with the customer as Reply-To. The in-app authorization is recorded with a timestamp (`customer_authorized_at`), but there is no standalone written agent-authorization document or agent-style wording. Counsel should say what is required.
- **Sending identity to brokers.** Opting out necessarily transmits the customer's identity (name, address, email) to each broker. The customer-facing authorization should say so plainly.
- **Jurisdiction and DROP eligibility.** DROP is for eligible California residents; the code does not check residency.

### Before relying on this in production

1. Get counsel's review of the ToS, authorized-agent, and crawler-identification items above.
2. Decide the user-facing copy for email-verification links and CAPTCHA/ID-verification sources.
3. Build site-specific handlers (or manual guidance) for the highest-volume brokers.
4. Validate the DROP payload against the official specification in sandbox.
5. Watch the `failed`, `rejected`, and `unverifiable` rates per source; a spike usually means a broker changed its form.

---

## Scan Job Recovery

A scan can be lost in three ways: the worker dies mid-run, the Redis entry is lost, or Redis is down when the scan is requested. The backend's recovery loop (`jobs/recovery.rs`, every `SCAN_RECOVERY_INTERVAL_SECS`) handles each, using only Postgres as the source of truth:

- **Claim and heartbeat.** A worker pops a job from Redis and calls `POST /internal/scan/:job_id/claim`. Only one claim can succeed per attempt; a duplicate queue entry gets `409` and is skipped. While working, the worker heartbeats through `PUT /internal/scan/status`.
- **Stale running jobs.** No heartbeat for `SCAN_STALE_AFTER_SECS` → the job is requeued with a fresh scan token and re-pushed to Redis. After `SCAN_MAX_ATTEMPTS` attempts it is marked `failed` and the customer gets an alert (and an audit entry).
- **Attempt fencing.** Every status update carries its attempt number. A worker that was presumed dead but is still running ("zombie") gets `409` and stops, so it cannot overwrite the newer attempt.
- **Lost queue entries.** A job still `queued` after `SCAN_REDISPATCH_AFTER_SECS` is pushed to Redis again.
- **Redis down at request time.** `POST /scans` still creates the job; recovery dispatches it later.
- **Housekeeping.** The same loop purges expired `login_throttle` rows and prunes `privacy_score_history` older than one year.

Multiple backend instances are safe: recovery claims rows with `FOR UPDATE SKIP LOCKED`.

Verified with real processes (backend, Redis, Postgres, worker): a normal scan; `kill -9` of a worker mid-scan (requeued in a few seconds and finished on attempt 2, with the old attempt's late update rejected); a deleted Redis entry (redispatched); a duplicate entry (skipped); and exhaustion of attempts (job failed, alert and audit entry written). These were run on a development machine, not in production.

---

## Testing

```bash
# Rust: unit + integration tests. Each database test gets its own freshly-migrated throwaway
# database (sqlx::test), so DATABASE_URL must point at a Postgres server where the user may
# create databases (never production).
cd backend
export DATABASE_URL=postgres://user:pass@localhost:5432/vectorscrub_test
export TEST_REDIS_URL=redis://127.0.0.1:6379/14   # optional (this is the default); a Redis server must be running
cargo test
cargo clippy --all-targets -- -D warnings

# Python agents
cd agents
pip install -r requirements.txt -r requirements-dev.txt
python -m playwright install chromium            # needed by the executor browser tests
python -m pytest -q

# Frontend
cd frontend && npm ci && npx tsc --noEmit && npm run build
```

What the suites cover: AES-GCM and key derivation, the removal state machine and its concurrency behaviour, scoped identity release and its audit trail, login lockout and rate limits, scan recovery, the PII guard (redaction, NER, vocabulary, fail-closed behaviour), the detection gateway, the orchestrator, the monitoring agent, and the opt-out executor against a real headless Chromium.

CI (`.github/workflows/ci.yml`) runs the backend (build offline, tests, clippy, `.sqlx` freshness check), agents, frontend, and Docker image builds. **The workflow file has not yet been run on GitHub**; its steps were run locally.

### Regenerating `.sqlx`

The backend builds without a database because query metadata is checked in under `backend/.sqlx/` (`SQLX_OFFLINE=true`, used by the Dockerfile and CI). After adding or changing a `sqlx::query!` macro (currently only `services/audit.rs`):

```bash
createdb vectorscrub_prepare
DATABASE_URL=postgres://user:pass@localhost:5432/vectorscrub_prepare infra/scripts/sqlx_prepare.sh
# CI uses --check, which exits non-zero when .sqlx is stale
git add backend/.sqlx backend/Cargo.lock
```

`backend/Cargo.lock` is tracked (the image build uses `--locked`); commit it with `.sqlx/`.

---

## Production Deployment

This section lists what exists in the code today. Items that are *not* implemented are called out explicitly; see also `SOC2_COMPLIANCE.md` and `PRODUCTION_CHECKLIST.md`.

### Security Hardening

**Secrets.** The backend refuses to start unless `JWT_SECRET` and `AGENT_API_KEY` are at least 32 characters and `MASTER_ENCRYPTION_KEY` decodes to exactly 32 bytes. `infra/scripts/gen_keys.sh` generates all of them. The master key is read from the environment only; in production inject it from a secret manager. There is no HSM/KMS integration and no automatic key rotation.

**Rate limiting and lockout.** Per-IP limits (see [Authentication](#authentication)) and per-email login lockout. Both are held per backend instance (limits in memory, lockout in Postgres). Behind a proxy set `TRUST_PROXY_HEADERS=true` *only if* the proxy overwrites `X-Forwarded-For`/`X-Real-IP`; otherwise every client appears as the proxy. Lockout can be used to lock a known email out for up to an hour; that trade-off was accepted over leaving the endpoint open to guessing.

**CSRF.** Double-submit cookie on POST/PUT/DELETE (`GET /api/v1/csrf-token`); internal agent endpoints are exempt. The frontend sends the token through `csrfFetch`. It works same-origin through the Next.js rewrites with `SameSite=Strict`. A cross-origin deployment would also need `X-CSRF-Token` added to the CORS allowed headers.

**Security headers.** CSP, HSTS, `X-Frame-Options: DENY`, `X-Content-Type-Options`, `Referrer-Policy`, `Permissions-Policy`.

**Agent key.** Compared in constant time. It is still one shared secret, and an identity release contains all identity fields for the one customer/work item it was requested for. The key cannot enumerate customers or read identity in bulk.

**Audit trail.** `audit_log` is protected by triggers (migration 0007) and `removal_audit_log` by rules; neither stops a database superuser.

### Observability

- `GET /api/v1/metrics` exposes Prometheus metrics and is **unauthenticated**; restrict it at the proxy or network layer.
- Structured logging via `tracing` (Rust) and `structlog` (Python). The code is written not to log PII, URLs, or exception text from fetched pages (error type and host only); there is no automatic redaction layer, so new log lines need the same care.
- OpenTelemetry export when `OTLP_ENDPOINT` is set.
- `GET /health/live` always returns 200; `GET /health/ready` checks Postgres and Redis. Docker Compose uses them for healthchecks.

### Circuit Breakers

The Python agents wrap page fetches in discovery and monitoring in a per-host circuit breaker (`agents/shared/circuit_breaker.py`). It is not applied to SMTP, DROP, or LLM calls, and the Rust backend has none. See `CIRCUIT_BREAKERS.md`.

### Data Retention

There is no automated retention or erasure job. The earlier `data_retention.rs` was removed because it was never wired in, and as written would have failed against the immutable `removal_audit_log` foreign key. Today only `login_throttle` rows and `privacy_score_history` older than one year are pruned. How to honor account deletion while keeping an immutable audit trail is an open design decision; see `DATA_RETENTION.md`.

### Docker Compose

`infra/docker/docker-compose.yml` requires the secrets (`POSTGRES_PASSWORD`, `REDIS_PASSWORD`, `JWT_SECRET`, `MASTER_ENCRYPTION_KEY`, `AGENT_API_KEY`) and fails with a clear message if one is missing. Postgres and Redis are bound to `127.0.0.1`. The backend image builds offline from `backend/.sqlx`.

### Compliance

`SOC2_COMPLIANCE.md` is a control framework and plan, not an attestation. It opens with a table of which controls are implemented in code and which are not.

---

## Competitive Landscape

| Service | Price/month | Brokers Covered | AI Capability | Security Posture |
|---------|-------------|-----------------|---------------|-----------------|
| Incogni | $7.99 | 420+ | Basic automation | Standard |
| Optery | $3.99–$24.99 | 635+ | Screenshots, some AI | Standard |
| DeleteMe | ~$9.00 | 750+ | Managed human-assisted | Standard |
| **VectorScrub** | **$8.99–$14.99** | **Growing** | **Multi-agent AI pipeline** | **AES-256-GCM, zero raw PII to LLMs, append-only audit log** |

VectorScrub's differentiation is not the number of sites covered — it is the quality of the AI reasoning, the transparency of the removal evidence, and the security posture around customer data.

---

## VectorGuard Labs Product Family

VectorScrub is one of four trust and privacy products under VectorGuard Labs:

| Product | Protects |
|---------|----------|
| **VectorScrub** | Personal information and digital privacy |
| **PyHSM** | Cryptographic keys and secrets |
| **VectorLedger** | Financial record integrity |
| **Vercre** | Verifiable digital credentials |

Together, these products form a coherent story: protecting identity, information, secrets, and financial integrity for individuals and organizations.

---

## Contributing

VectorScrub is currently in active private development under VectorGuard Labs. If you are contributing:

1. Read `.kiro/steering/vectorscrub.md` before making any changes — it contains the critical security rules that govern this codebase.
2. Never log, print, or transmit raw PII values. Never send customer PII to an LLM: all model calls go through `DetectionAgent._run_classifier`, which applies `shared/pii_guard.py` (redaction plus a fail-closed check). Do not add another LLM call site.
3. Never commit `.env` or any file containing real secrets.
4. The audit log is append-only by design. Do not add update or delete operations to `removal_audit_log`.
5. All PII-touching code belongs in the Rust backend. The Python agent layer should never hold or process raw customer identity data directly.

---

## License

Copyright © 2026 VectorGuard Labs. All rights reserved.

This software is proprietary and confidential. Unauthorized copying, distribution, or use of this software, in whole or in part, is strictly prohibited without express written permission from VectorGuard Labs.

---

## What's New in Phase 3

**Phase 3 is now complete.** VectorScrub now provides continuous protection with scheduled monitoring, removal verification, and comprehensive alert systems. (Family accounts were built during Phase 3 and later removed; all plans are per person.)

### Backend Features Added:

**Scheduled Monitoring, Verification, and Reappearance Detection:**
- Automatic rescans on a tier-based schedule (Personal: 30 days, Premium: 7 days)
- Background scheduler runs every 6 hours checking for customers due for rescan
- Monitoring worker verifies submitted removals and re-checks removed records (Premium weekly, Personal monthly)
- The backend decides what is due (`/internal/.../due-for-*` endpoints); the agent fetches pages and reports `removed` / `still_present` / `clear` / `reappeared` / `inconclusive`
- Inconclusive checks (blocked, CAPTCHA, fetch error, LLM failure) never count as "removed"; they are retried the next day
- Reappearance reactivates the exposure, recalculates the privacy score, and alerts the customer
- Confidence threshold of 0.7 for reappearance alerts
- Monitoring is enabled by default for paid tiers (`customers.monitoring_enabled`, migration 0006)

**Email-Based Opt-Out (Complete):**
- Full SMTP integration with TLS support
- HTML and plain text email templates for professional opt-out requests
- Evidence hashing with SHA-256 for tamper-resistance
- Message ID tracking for confirmation
- Gmail, Outlook, and custom SMTP server support

**California DROP API Integration (Complete):**
- OAuth authentication with the official California privacy.ca.gov API
- Single submission covers 600+ registered data brokers
- Sandbox mode for testing and development
- Request tracking and confirmation reference extraction
- Automatic fallback for non-California residents

**Privacy Score History and Trends:**
- Automatic snapshot recording after each scan completion
- Historical data retention for 1 year
- Delta calculations showing score changes over time
- Breakdown by severity level (critical, high, medium, low)
- RESTful API endpoint for trend retrieval with configurable time ranges

**Alert and Notification System:**
- Customizable email and SMS preferences per customer
- Per-alert-type toggles (new exposures, removal failures, reappearances)
- Auto-triggered alerts on key events
- Unread alert tracking with read status management
- Fire-and-forget background alert delivery

### Frontend Features Added:

**Privacy Score Trend Charts:**
- Interactive line chart showing score changes over time
- Stacked area chart for exposure breakdown by severity
- Time range selector (7, 30, 90, 365 days)
- Delta indicators with visual trend arrows
- Color-coded severity levels matching brand palette
- Responsive Recharts integration

**Exposure History Timeline:**
- Chronological event timeline for all discoveries and removals
- Four event types: discovered, removal_submitted, removal_verified, reappeared
- Advanced filtering by event type, severity, and date range
- CSV export functionality for external analysis
- Visual event markers with color-coded borders

### Technical Enhancements:

**Database Schema (Migration 0003):**
- `family_members` table (no longer used; see above)
- `privacy_score_history` table for trend analysis
- `notification_preferences` table for alert customization
- Performance indexes on monitoring queries
- Additional columns: `customers.monitoring_enabled`, `exposures.last_monitored_at`

**Three-Worker Orchestrator Architecture:**
- Scan worker for discovery and detection
- Removal worker for opt-out execution
- Monitoring worker for reappearance checks
- All three run concurrently in parallel
- Independent polling intervals for optimal performance

**New API Endpoints:**
- `GET /api/v1/privacy-score/history?days=30`
- `GET /api/v1/notifications/preferences`
- `PUT /api/v1/notifications/preferences`
- `GET /api/v1/notifications/alerts`
- `POST /api/v1/notifications/alerts/:id/read`

(Four `/family/*` endpoints were also added in Phase 3; they were later removed along with the family service. The monitoring endpoints are documented under Internal Agent Endpoints.)

### Security Maintained:
- All customer PII remains encrypted with AES-256-GCM
- Master key never exposed to Python agent layer
- Customer PII never reaches LLM APIs: page text is redacted and the final payload is verified before every call (see [How the AI Works](#how-the-ai-works))
- SMTP credentials protected in environment variables
- Append-only audit trail for all removal actions, including verification and reappearance events

---

## Recent Updates

### Monitoring Completed End to End and Docs Reconciled

- **Removal verification and reappearance monitoring now work end to end.** New backend endpoints (`due-for-verification`, `verification-result`, `due-for-monitoring`, `monitoring-result`) and a real monitoring worker replace the previous sleep-and-TODO scaffold. Premium accounts are re-checked weekly and Personal monthly; Free is never monitored.
- **Bugs fixed on the way:** the monitoring agent called the detection agent with the wrong signature (so it could never detect anything); `monitoring_enabled` defaulted to `false` and was never set, so scheduled rescans and monitoring never ran; verified removals failed with a SQL error (nonexistent `exposures.updated_at`); and a reappeared record could never be re-authorized because any prior removal request blocked a new one.
- **Honest failure handling:** a new `failed` removal status (migration 0006) ends the earlier endless-retry behavior; web-form submissions need positive confirmation to count as `submitted`; CAPTCHAs fail explicitly instead of being attempted.
- **Pricing reconciled:** the Family tier is gone everywhere in the docs (migration 0005 already removed it from the database). All plans are per person.
- **No customer PII to LLMs, enforced in code:** every model call now goes through a redaction-and-verify gateway (`agents/shared/pii_guard.py`). The model no longer receives any identity (not even masked hints) or page URLs, and a call is blocked if any customer value is still in the payload.
- **Third-party names closed too:** a local spaCy NER pass (`pii_ner.py`) marks other people's names, and a deny-by-default vocabulary filter (`pii_vocab.py`) drops every word that is not structural, so a name the patterns and NER both miss still cannot leave. Measured on 2,500 multi-locale pages: patterns plus NER alone leaked a name on 60%; the full pipeline on 0. Requires `spacy` and `en_core_web_sm` (pinned in `requirements.txt`).
- **CSRF fixed:** the middleware cut the cookie value at the `=` in base64 padding, so cookie and header could never match, and the frontend never sent the header, so every authenticated POST/PUT/DELETE from the browser was rejected with 403. Tokens are now URL-safe, parsing is correct, comparison is constant-time, `Secure` is set only in production, and the frontend sends the header through `csrfFetch`.
- **Risk documented:** see [Legal and Operational Risk](#legal-and-operational-risk).

### Hardening Pass: Scoped Identity, Limits, Recovery, Tests (Latest)

- **No more "skeleton key".** The agent key used to read any customer's identity by id. That route is gone. Identity is now released one work item at a time (a running scan with its token, an authorized removal request, or an exposure due for monitoring), the customer is derived from the item, and every release is audited (fail-closed). The agent key is compared in constant time and startup rejects weak secrets.
- **Rate limiting and login lockout** added (per-IP limits, per-email lockout with `Retry-After`, no account enumeration).
- **Scan recovery:** scans lost to worker crashes, lost Redis entries, or Redis outages are requeued, redispatched, or failed with an alert (see [Scan Job Recovery](#scan-job-recovery)).
- **Removal state machine** enforced server-side in a locked transaction; audit entries are bound to the request's real owner; `audit_log` is append-only via triggers.
- **Docs now describe what exists.** Removed unused or broken code: Rust `data_retention`, `circuit_breaker`, `experiments`, and the disabled family routes/services; Python `data_brokers/`, `hsm_service.py`, and `validation.py` (the HSM service would have put a key-store password in the agent process). The Python circuit breaker was kept and wired into discovery and monitoring. Matching docs (`AB_TESTING.md`, `DATA_BROKERS.md`, `PYHSM_INTEGRATION.md`) were deleted and others corrected.
- **Bugs found while testing:** Docker Compose never passed `AGENT_API_KEY` to the backend, so the stack could not boot; discovery sent the LLM a placeholder instead of real page text, so detection could not work as intended; the removal-authorize endpoint did not check that the exposure belonged to the caller.
- **Tests and CI:** Rust tests (crypto, state machine, identity release, lockout, limits, recovery), Python tests (PII guard, orchestrator, monitoring, executor in Chromium), checked-in `.sqlx`, `.github/workflows/ci.yml` (not yet run on GitHub), clippy clean.

### Production and Security Hardening (earlier pass, corrected)

An earlier version of this entry listed capabilities that were documented but never wired in (PyHSM key management, a Rust circuit breaker, automated data retention, a broker registry, A/B testing, an input-validation layer). They have been removed from the code and docs; see the hardening entry above and [Production Deployment](#production-deployment) for what exists. What that pass did deliver: CSRF protection, security headers, the audit log (migration 0004), Prometheus metrics, health checks, OTLP tracing, and the supporting docs (`CIRCUIT_BREAKERS.md`, `DATA_RETENTION.md`, `OBSERVABILITY.md`, `SECURITY_FIXES.md`, `SOC2_COMPLIANCE.md`, `P1_IMPROVEMENTS.md`, `P2_IMPROVEMENTS.md`, the last four annotated as historical or planning documents).

### Phase 3 Complete — Keep Me Protected

*Historical entry: the family-related items below (endpoints, `family.rs` files, family dashboard, member limits) were later removed; see Subscription Plans.*

**VectorScrub Phase 3 is now production-ready.** All 12 planned tasks completed and deployed.

**What Changed:**
- **Continuous monitoring**: Scheduled rescans every 7-30 days based on subscription tier
- **Reappearance detection**: Automatic alerts when removed records come back online
- **Per-person plans**: Family plans were removed; each person has their own account
- **Privacy insights**: Historical trend charts showing score changes over time
- **Multi-channel removal**: Email opt-out and California DROP API integration now complete
- **Alert system**: Customizable email/SMS notifications for key events
- **Exposure timeline**: Chronological history of all discoveries and removals

**Backend Additions:**
- 13 new API endpoints across privacy score, family management, and notifications
- 3 new database tables: `family_members`, `privacy_score_history`, `notification_preferences`
- Three-worker orchestrator (scans, removals, monitoring)
- Scheduled rescan dispatcher running every 6 hours
- Full SMTP integration with HTML/text templates
- California DROP API OAuth client
- Alert delivery system with preference filtering

**Frontend Additions:**
- Privacy score trend charts with 7/30/90/365-day views (Recharts)
- Exposure history timeline with filtering and CSV export
- Family dashboard with member management and aggregate stats
- Conditional navigation based on subscription tier
- Enhanced API client with family and notification endpoints

**Security Posture:**
- All PII encryption maintained (AES-256-GCM)
- No raw customer data sent to external LLMs
- SMTP credentials stored in environment variables only
- Family member limits enforced at database level
- Append-only audit trail for all actions

**Files Modified/Added:**
- `backend/migrations/0003_phase3_features.sql` - Phase 3 schema
- `backend/src/jobs/scheduler.rs` - Rescan scheduler
- `backend/src/models/` - privacy_score.rs, family.rs, notification.rs, alert.rs
- `backend/src/services/` - privacy_score.rs, family.rs, notification.rs
- `backend/src/api/` - privacy_score.rs, family.rs, notifications.rs
- `agents/monitoring/agent.py` - Reappearance detection
- `agents/removal/executor.py` - Email and DROP integration
- `agents/orchestrator/orchestrator.py` - Three-worker architecture
- `frontend/src/components/PrivacyScoreChart.tsx` - New chart component
- `frontend/src/app/dashboard/history/page.tsx` - Timeline page
- `frontend/src/app/dashboard/family/page.tsx` - Family dashboard
- `frontend/src/lib/api.ts` - 13 new API client functions

**Testing Readiness:**
- Zero compilation errors
- All TypeScript types valid
- Environment variables documented in `.env.example`
- Migration 0003 applies automatically on backend start
- Full PHASE_3_COMPLETE.md documentation available

### Frontend Modernization (Phase 2)

**Tailwind CSS 4.1.8 Upgrade**
- Migrated from Tailwind CSS 3 to 4.1.8 using the new PostCSS plugin architecture
- Updated configuration to minimal Tailwind v4 format
- Maintained all existing styles with zero visual regression

**Next.js 15.5.29**
- Upgraded from Next.js 15 to 15.5.29 for stability improvements
- Fixed hydration mismatch issues that caused blank page rendering

**New Profile Page**
- Added dedicated `/dashboard/profile` page for identity profile management
- Dynamic field management for addresses, phones, and emails
- Real-time form validation with helpful error messages
- Visual security indicators explaining AES-256-GCM encryption

**Dependencies Updated**
- React 19.1.0 and React DOM 19.1.0
- TypeScript 5.8.3
- lucide-react 0.511.0 for consistent iconography
- Recharts 2.x for data visualization (Phase 3)

---

*Built with security-first principles by VectorGuard Labs.*
*"Your personal data shouldn't be out there. We'll help take it back."*
