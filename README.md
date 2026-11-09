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
18. [Production Deployment](#production-deployment)
19. [Competitive Landscape](#competitive-landscape)
20. [VectorGuard Labs Product Family](#vectorguard-labs-product-family)
21. [Recent Updates](#recent-updates)
22. [Contributing](#contributing)
23. [License](#license)

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

**Critical security note:** Raw customer PII is never sent to an external LLM. The detection agent anonymizes identity fields into masked tokens (e.g., `J*** S***` for a name, `1** M*** St***` for an address) before passing anything to the model. Matching is done by reasoning about patterns, not by exposing real values.

### 3. Automated Removal Request Submission
For each confirmed exposure, VectorScrub identifies the correct opt-out procedure for that specific data source — web form, email submission, or the California DROP API. After explicit customer authorization, VectorScrub automatically submits removal requests using Playwright browser automation for web forms, capturing screenshots as evidence and extracting confirmation references.

### 4. Removal Tracking and Verification
Every removal request is tracked through its full lifecycle: authorized → submitted → acknowledged → verified removed. VectorScrub distinguishes between these states clearly so customers always know the true status of their records — not just that a request was sent, but whether removal was actually confirmed. All actions are logged to an immutable, append-only audit trail enforced at the database level.

### 5. Continuous Monitoring
After removals, VectorScrub continues to monitor sources on a scheduled basis. If a removed record reappears (which happens frequently as data brokers re-acquire information), an alert is sent and a new removal request is prepared automatically.

### 6. Privacy Score and Exposure Report
Customers see a live **Privacy Score** (0–100) that reflects their current exposure level. The score decreases based on the severity and volume of active exposures, and increases as records are removed. A full exposure history shows every finding, removal, and reappearance over time.

---

## Benefits to End Users

### Know exactly what's out there
Most people discover they have dozens of active exposures on their first scan. VectorScrub surfaces the full picture in a single dashboard rather than requiring customers to check each site manually.

### Get it removed — with proof
VectorScrub prepares and submits opt-out requests to data brokers using intelligent browser automation. Every submission is captured with screenshot evidence (SHA-256 hashed for tamper-resistance), confirmation references are extracted, and all actions are logged to an immutable audit trail. Customers see a clear before-and-after record, not just a promise that "we tried."

### Stop it from coming back
Data brokers regularly re-acquire and republish information even after opt-outs. Continuous monitoring and automatic re-removal means customers don't have to manage this cycle themselves.

### Protect your whole family
The Family plan extends protection to up to five people, with a shared dashboard and individual privacy scores for each member.

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
  │                     │  (uses anonymized tokens — not raw PII)
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
           │  (Phase 3)
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
| PII at rest | AES-256-GCM encryption with a per-customer key derived from the master key via HMAC-SHA256. Each customer's ciphertext can only be decrypted with their derived key. |
| Master key | Loaded once into the Rust process from an environment variable (or PyHSM in production). Never written to disk, never accessible to the Python agent layer. Production deployments use PyHSM for hardware-backed key management. |
| PII and LLMs | Raw customer values are never sent to any external LLM API. The detection agent receives only anonymized tokens (first letter + asterisks) for pattern matching. |
| Removal authorization | No removal request is ever submitted to a data source without an explicit customer authorization recorded in the database with timestamp. The agent prepares; the customer approves; the executor submits. |
| Audit trail | Every action taken on a removal request is recorded in an append-only `removal_audit_log` table. PostgreSQL rules enforce `NO UPDATE` and `NO DELETE` at the database level, making the log immutable and tamper-evident. Migration 0004 adds comprehensive audit logging across all sensitive operations. |
| Evidence collection | Screenshot evidence captured during web form submission is SHA-256 hashed. The hash is stored in the database; screenshots can be stored in encrypted S3 for retrieval. Hashes prove authenticity without storing large binary data. |
| Model training | Customer PII is never used to fine-tune or train any AI model — not VectorScrub's, not a provider's. |
| Agent authentication | Rust internal endpoints accept only the `X-Agent-Key` header, not customer JWTs. The agent key is never exposed to the frontend. |
| Tenant isolation | Per-customer encryption keys mean a compromise of one customer's encrypted data does not affect others. |
| Password storage | Argon2id with random per-user salt. No plaintext or reversibly encoded passwords stored anywhere. |
| **Production Security** | **Additional hardening for production deployments** |
| CSRF protection | Double-submit cookie pattern on all state-changing requests. Token endpoint at `/api/v1/csrf-token`. |
| Security headers | CSP, HSTS, X-Frame-Options, X-Content-Type-Options, Referrer-Policy configured via dedicated middleware. |
| Rate limiting | Request timeout protection (30s) with configurable IP-based rate limiting. Prevents abuse and DoS attacks. |
| Circuit breakers | Automatic failure detection for external services (data brokers, SMTP, DROP API). Prevents cascading failures and resource exhaustion. |
| Input validation | Multi-layer validation at agent and API boundaries. Sanitizes all user input before processing. |
| Request tracing | Unique request IDs tracked across all services for security auditing and debugging. |
| Metrics and observability | Prometheus metrics exported for monitoring request latency, error rates, database pool stats, and custom business metrics. |
| Data retention | Automated cleanup of old scans, exposures, and audit logs according to configurable retention policies. Complies with GDPR and CCPA data minimization requirements. |
| SOC 2 compliance | Comprehensive control framework covering Security, Availability, Processing Integrity, Confidentiality, and Privacy trust criteria. |

---

## Subscription Plans

| Plan     | Price/month | Key Features |
|----------|-------------|--------------|
| **Free**     | $0     | Initial exposure scan, privacy score, exposure report |
| **Personal** | $8.99  | Automated removal requests, continuous monitoring, monthly rescan, alerts |
| **Premium**  | $14.99 | Everything in Personal + expanded source coverage, priority removal, weekly rescan, evidence collection |
| **Family**   | $24.99 | Everything in Premium + up to 4 family members, shared dashboard |

Free accounts are limited to one scan per 24 hours. Paid accounts can initiate multiple scans per day and have access to a wider range of data sources depending on tier.

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
│   │   ├── 0003_phase3_features.sql  # Monitoring, family, alerts
│   │   └── 0004_audit_log.sql        # Comprehensive audit logging
│   └── src/
│       ├── main.rs                   # Entry point, server setup
│       ├── db.rs                     # Shared AppState (pool, redis, config)
│       ├── docs.rs                   # OpenAPI documentation
│       ├── observability.rs          # Prometheus metrics and tracing
│       ├── api/
│       │   ├── mod.rs                # Route table
│       │   ├── auth.rs               # POST /auth/register, /auth/login
│       │   ├── customers.rs          # GET /me, POST /me/identity
│       │   ├── scans.rs              # POST /scans, GET /scans, GET /scans/:id
│       │   ├── exposures.rs          # GET /exposures, GET /exposures/summary
│       │   ├── removals.rs           # GET /removal-requests, POST /removal-requests/:id/authorize
│       │   ├── agent_callbacks.rs    # Internal: PUT /internal/scan/status, POST /internal/exposures
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
│       │   ├── family.rs             # Family member relationships
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
│       │   ├── circuit_breaker.rs    # Circuit breaker for external services
│       │   ├── data_retention.rs     # Automated data cleanup and retention
│       │   ├── experiments.rs        # A/B testing and feature flags
│       │   ├── privacy_score.rs      # Privacy score history tracking
│       │   ├── family.rs             # Family plan management
│       │   └── notification.rs       # Alert and notification service
│       └── jobs/
│           └── scheduler.rs          # Scheduled rescan dispatcher (Phase 3)
│
├── agents/                           # Python AI agent layer
│   ├── requirements.txt              # All Python dependencies
│   ├── shared/
│   │   ├── config.py                 # pydantic-settings config
│   │   ├── models.py                 # Shared Pydantic data models
│   │   └── backend_client.py         # HTTP client → Rust internal API
│   ├── discovery/
│   │   └── agent.py                  # Scans 8+ data broker sources per tier
│   ├── detection/
│   │   └── agent.py                  # LLM-based PII classification (anonymized tokens)
│   ├── removal/
│   │   ├── agent.py                  # Opt-out procedure registry + request preparation
│   │   └── executor.py               # Playwright automation, email/DROP submission
│   ├── monitoring/
│   │   └── agent.py                  # Reappearance detection for removed records
│   └── orchestrator/
│       ├── orchestrator.py           # Redis worker, coordinates full pipeline (3 workers)
│       ├── circuit_breaker.py        # Failure protection for external dependencies
│       ├── hsm_service.py            # PyHSM integration for production key management
│       ├── validation.py             # Multi-layer input validation and sanitization
│       └── data_brokers/             # Modular data broker integration system
│           ├── __init__.py           # Package initialization
│           ├── base.py               # Base broker interface and common logic
│           ├── registry.py           # Broker registry and factory
│           ├── aggregator.py         # Result deduplication and confidence scoring
│           ├── spokeo.py             # Spokeo-specific implementation
│           ├── whitepages.py         # WhitePages-specific implementation
│           ├── truthfinder.py        # TruthFinder-specific implementation
│           └── beenverified.py       # BeenVerified-specific implementation
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
│       └── gen_keys.sh               # Generates JWT secret, master key, DB/Redis passwords
│
├── .env.example                      # All required environment variables with descriptions
├── .gitignore                        # Excludes .env, target/, node_modules/, __pycache__/
├── .kiro/steering/vectorscrub.md     # Project conventions for Kiro AI sessions
├── README.md                         # This file
├── AB_TESTING.md                     # A/B testing framework and experiment design
├── CIRCUIT_BREAKERS.md               # Failure protection and resilience patterns
├── DATA_BROKERS.md                   # Data source integration architecture
├── DATA_RETENTION.md                 # Data lifecycle and retention policies
├── OBSERVABILITY.md                  # Metrics, logging, and monitoring guide
├── P1_IMPROVEMENTS.md                # Priority 1 feature enhancements
├── P2_IMPROVEMENTS.md                # Priority 2 feature enhancements
├── PYHSM_INTEGRATION.md              # Hardware security module setup
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
| `JWT_SECRET` | Yes | Random string used to sign JWTs. Generate with `openssl rand -base64 48` |
| `MASTER_ENCRYPTION_KEY` | Yes* | Base64-encoded 32-byte AES master key. Generate with `openssl rand -base64 32`. *Not required if using PyHSM. |
| `PORT` | No | Port for the Rust API (default: `8080`) |
| `ENVIRONMENT` | No | `development`, `staging`, or `production` (default: `development`) |
| `DB_POOL_SIZE` | No | PostgreSQL connection pool size (default: `10`) |
| `ALLOWED_ORIGINS` | No | Comma-separated list of allowed CORS origins (default: `http://localhost:3000`) |
| `AGENT_API_KEY` | Yes | Shared secret for Python agent → Rust internal API authentication |
| `BACKEND_API_URL` | No | URL the Python agents use to reach the Rust API (default: `http://localhost:8080`) |
| `OPENAI_API_KEY` | Recommended | OpenAI API key for the detection agent. One of OpenAI or Anthropic required. |
| `ANTHROPIC_API_KEY` | Optional | Anthropic API key as an alternative to OpenAI |
| `DEFAULT_LLM_PROVIDER` | No | `openai` or `anthropic` (default: `anthropic`) |
| `DEFAULT_LLM_MODEL` | No | Model name for the LLM provider (e.g., `claude-3-5-sonnet-20241022`, `gpt-4o`) |
| `NEXT_PUBLIC_API_URL` | No | URL the Next.js frontend uses for API calls (default: `http://localhost:8080`) |
| **Observability** | | **Metrics and Tracing** |
| `OTLP_ENDPOINT` | Optional | OpenTelemetry OTLP endpoint for distributed tracing (e.g., `http://localhost:4317`). Leave empty to disable. Metrics are always available at `GET /api/v1/metrics`. |
| **PyHSM Integration** | | **Production Key Management** |
| `HSM_MASTER_PASSWORD` | Optional | Master password for PyHSM. If set, PyHSM is used instead of `MASTER_ENCRYPTION_KEY` for hardware-backed key storage and automatic rotation. |
| `HSM_STORAGE_PATH` | Optional | Path to PyHSM encrypted keystore file (default: `./keystore.enc`) |
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
| `POST` | `/auth/login` | Sign in. Body: `{ email, password }`. Returns `{ token, customer }`. |

### Customer Profile

| Method | Endpoint | Auth | Description |
|--------|----------|------|-------------|
| `GET` | `/me` | JWT | Returns the authenticated customer's profile and privacy score. |
| `POST` | `/me/identity` | JWT | Submit or update the identity profile (name, address, phones, emails). PII is encrypted immediately. |

### Scans

| Method | Endpoint | Auth | Description |
|--------|----------|------|-------------|
| `POST` | `/scans` | JWT | Queue a new scan job. Returns `{ job_id, status, message }`. |
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
| `POST` | `/removal-requests/:exposure_id/authorize` | JWT | Authorize a removal request for a specific exposure. Creates audit trail entry. |
| `GET` | `/removal-requests/:id` | JWT | Get a specific removal request with full audit log. |

### Privacy Score History (Phase 3)

| Method | Endpoint | Auth | Description |
|--------|----------|------|-------------|
| `GET` | `/privacy-score/history?days=30` | JWT | Retrieve privacy score trend with historical snapshots and delta calculations. |

### Family Management (Phase 3)

| Method | Endpoint | Auth | Description |
|--------|----------|------|-------------|
| `POST` | `/family/members` | JWT | Add a family member by email. Requires Family plan. Body: `{ member_email }`. |
| `GET` | `/family/members` | JWT | List all family members with privacy scores and roles. |
| `DELETE` | `/family/members/:member_id` | JWT | Remove a family member from the family plan. |
| `GET` | `/family/stats` | JWT | Get aggregate family statistics (average score, total exposures, member count). |

### Notifications and Alerts (Phase 3)

| Method | Endpoint | Auth | Description |
|--------|----------|------|-------------|
| `GET` | `/notifications/preferences` | JWT | Get customer's notification preferences for email and SMS alerts. |
| `PUT` | `/notifications/preferences` | JWT | Update notification preferences. Body: `{ email_enabled, sms_enabled, phone_number, alert_*  }`. |
| `GET` | `/notifications/alerts` | JWT | Get unread alerts for the customer (new exposures, failures, reappearances). |
| `POST` | `/notifications/alerts/:id/read` | JWT | Mark a specific alert as read. |

### Internal Agent Endpoints

| Method | Endpoint | Auth | Description |
|--------|----------|------|-------------|
| `PUT` | `/internal/scan/status` | Agent key | Update scan job progress (called by the orchestrator). |
| `POST` | `/internal/exposures` | Agent key | Record a confirmed exposure found by the detection agent. |
| `GET` | `/internal/removal-requests` | Agent key | Poll for authorized removal requests ready for execution. |
| `PUT` | `/internal/removal-requests/:id/status` | Agent key | Update removal request status after submission. |
| `POST` | `/internal/removal-requests/:id/audit` | Agent key | Record an audit log entry for a removal request. |

### Security and Observability

| Method | Endpoint | Auth | Description |
|--------|----------|------|-------------|
| `GET` | `/csrf-token` | None | Get CSRF token for state-changing requests. Sets cookie and returns token in response body. |
| `GET` | `/metrics` | None | Prometheus metrics endpoint. Returns metrics in Prometheus exposition format. |
| `GET` | `/health/live` | None | Liveness probe. Returns 200 if process is running. |
| `GET` | `/health/ready` | None | Readiness probe. Returns 200 if database and Redis are accessible. |

---

## Database Schema

The PostgreSQL schema is defined across four migrations and applied automatically on backend startup:
- `0001_initial_schema.sql` - Core schema (Phase 1 & 2)
- `0002_removal_requests_index.sql` - Performance optimization (Phase 2)
- `0003_phase3_features.sql` - Monitoring, family, and alerts (Phase 3)
- `0004_audit_log.sql` - Comprehensive audit logging for production (Production hardening)

| Table | Purpose |
|-------|---------|
| `customers` | Account records: email, hashed password, subscription tier, privacy score, monitoring_enabled |
| `identity_profiles` | Per-customer encrypted PII: all fields stored as AES-256-GCM ciphertext blobs |
| `scan_jobs` | Scan lifecycle: queued, running, completed, failed; sources and exposures counted |
| `exposures` | Every discovered PII exposure: source, category, severity, masked summary, active/removed state, last_monitored_at |
| `removal_requests` | Opt-out requests: status lifecycle from authorized through verified removed |
| `removal_audit_log` | Append-only tamper-evident log of every action on every removal request |
| `alerts` | Customer notifications: new exposures, removal failures, reappearances |
| `family_members` | **(Phase 3)** Family associations: owner/member relationships, max 5 per family plan |
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
- LLM-based PII detection with anonymized token matching
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

- ✅ Scheduled rescans (weekly for Premium/Family, monthly for Personal)
- ✅ Reappearance detection and automatic re-removal
- ✅ Family accounts (up to 4 members under one subscription)
- ✅ Privacy score trend charts with historical analysis
- ✅ Email and SMS alert system with customizable preferences
- ✅ Exposure history timeline with event tracking
- ✅ Complete email-based opt-out integration
- ✅ California DROP API integration (600+ brokers)
- ✅ Privacy score history API with trend analysis
- ✅ Family management dashboard with aggregate statistics
- ✅ Notification preferences and alert management

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
- Family protection across multiple members

---

## Production Deployment

VectorScrub is production-ready with enterprise-grade security, observability, and compliance features.

### Security Hardening

**CSRF Protection**
- Double-submit cookie pattern on all state-changing requests (POST, PUT, DELETE)
- Token endpoint: `GET /api/v1/csrf-token`
- Automatic validation via dedicated middleware
- Exempts safe methods (GET, HEAD, OPTIONS) and internal agent endpoints

**Security Headers**
- Content Security Policy (CSP) with strict source restrictions
- HTTP Strict Transport Security (HSTS) with 1-year max-age
- X-Frame-Options: DENY (prevents clickjacking)
- X-Content-Type-Options: nosniff
- Referrer-Policy: strict-origin-when-cross-origin
- Permissions-Policy with disabled dangerous features

**Rate Limiting**
- 30-second request timeout prevents long-running attacks
- Configurable IP-based rate limiting (via `rate_limiter.rs`)
- Protects against DoS and abuse

**Input Validation**
- Multi-layer validation at agent and API boundaries
- Sanitizes all user input before processing
- Prevents injection attacks and malformed requests

### Observability

**Prometheus Metrics**
Export endpoint: `GET /api/v1/metrics`

Tracked metrics:
- Request count (by method, path, status)
- Request latency (histogram with p50, p90, p99)
- Active requests (gauge)
- Database connection pool stats (active, idle, max)
- Custom business metrics (scans, exposures, removals)

**Structured Logging**
- JSON-formatted logs with trace context
- Correlation IDs for end-to-end request tracking
- Log levels: ERROR, WARN, INFO, DEBUG
- Sensitive PII automatically redacted

**Distributed Tracing**
- OpenTelemetry OTLP export support
- Configure via `OTLP_ENDPOINT` environment variable
- Compatible with Jaeger, Honeycomb, Datadog, and other OTLP backends

**Health Checks**
- Liveness probe: Always returns 200 (process is running)
- Readiness probe: Checks database and Redis connectivity

### Circuit Breakers

VectorScrub implements circuit breakers for all external dependencies to prevent cascading failures:

**Protected Services:**
- Data broker APIs (Spokeo, WhitePages, TruthFinder, BeenVerified)
- SMTP servers (for email-based opt-outs)
- California DROP API
- External LLM providers (OpenAI, Anthropic)

**Failure Thresholds:**
- Trips open after 5 consecutive failures
- Half-open state after 60-second cooldown
- Automatic recovery when service returns

**Benefits:**
- Prevents resource exhaustion
- Fast-fail behavior reduces latency
- Automatic recovery without restarts

### Data Retention

Automated cleanup policies ensure compliance with data minimization requirements:

| Data Type | Retention Period | Justification |
|-----------|------------------|---------------|
| Scan jobs | 90 days | Operational history for debugging |
| Exposures (removed) | 1 year | Privacy score history and trends |
| Exposures (active) | Indefinite | Required for monitoring |
| Audit logs | 7 years | SOC 2 and legal compliance |
| Customer accounts | Until deletion | Required for service |

Configure retention policies in `backend/src/services/data_retention.rs`.

### PyHSM Integration

For production deployments, use PyHSM for hardware-backed key management:

**Setup:**
```bash
# Install PyHSM
pip install pyhsm

# Set environment variables
export HSM_MASTER_PASSWORD=your-strong-password
export HSM_STORAGE_PATH=./keystore.enc
# Do NOT set MASTER_ENCRYPTION_KEY when using PyHSM
```

**Benefits:**
- Hardware-backed encryption keys
- Automatic key rotation
- Tamper-evident key storage
- FIPS 140-2 compliance (when using certified HSM)

See `PYHSM_INTEGRATION.md` for detailed setup instructions.

### SOC 2 Compliance

VectorScrub implements all five Trust Service Criteria:

1. **Security** - Multi-layer access controls, encryption at rest and in transit
2. **Availability** - Circuit breakers, health checks, auto-scaling support
3. **Processing Integrity** - Audit logs, input validation, idempotent operations
4. **Confidentiality** - Per-customer encryption, role-based access control
5. **Privacy** - Data retention policies, consent management, right to deletion

See `SOC2_COMPLIANCE.md` for the complete control framework and evidence collection procedures.

### A/B Testing and Experimentation

Built-in experimentation framework for data-driven product decisions:

**Features:**
- Runtime feature flags (no deployments required)
- Consistent user bucketing across sessions
- Variant assignment with traffic allocation
- Metrics tracking per experiment
- Statistical significance testing

**Example Usage:**
```rust
let variant = experiments::assign_variant(&customer_id, "new_privacy_score_formula");
match variant {
    Variant::Control => /* existing formula */,
    Variant::Treatment => /* new formula */,
}
```

See `AB_TESTING.md` for experiment design guidelines and analysis procedures.

---

## Competitive Landscape

| Service | Price/month | Brokers Covered | AI Capability | Security Posture |
|---------|-------------|-----------------|---------------|-----------------|
| Incogni | $7.99 | 420+ | Basic automation | Standard |
| Optery | $3.99–$24.99 | 635+ | Screenshots, some AI | Standard |
| DeleteMe | ~$9.00 | 750+ | Managed human-assisted | Standard |
| **VectorScrub** | **$8.99–$24.99** | **Growing** | **Multi-agent AI pipeline** | **AES-256-GCM, zero raw PII to LLMs, append-only audit log** |

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
2. Never log, print, or transmit raw PII values. Use masked/anonymized representations in logs and LLM prompts.
3. Never commit `.env` or any file containing real secrets.
4. The audit log is append-only by design. Do not add update or delete operations to `removal_audit_log`.
5. All PII-touching code belongs in the Rust backend. The Python agent layer should never hold or process raw customer identity data directly.

---

## License

Copyright © 2026 VectorGuard Labs. All rights reserved.

This software is proprietary and confidential. Unauthorized copying, distribution, or use of this software, in whole or in part, is strictly prohibited without express written permission from VectorGuard Labs.

---

## What's New in Phase 3

**Phase 3 is now complete.** VectorScrub now provides continuous protection with scheduled monitoring, family accounts, and comprehensive alert systems.

### Backend Features Added:

**Scheduled Monitoring and Reappearance Detection:**
- Automatic rescans on a tier-based schedule (Personal: 30 days, Premium/Family: 7 days)
- Background scheduler runs every 6 hours checking for customers due for rescan
- Monitoring agent re-checks removed records and detects reappearances
- Automatic alert creation when removed information comes back online
- Confidence threshold of 0.7 for reappearance classification

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

**Family Account Support:**
- Up to 4 family members per Family plan subscription
- Owner/member role system with granular permissions
- Individual privacy scores for each family member
- Aggregate statistics across all members
- Email-based member invitations with pending status tracking

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

**Family Dashboard:**
- Family statistics overview (total members, average score, total exposures)
- Add/remove family member functionality
- Member list with individual privacy scores
- Role indicators (crown icon for owner)
- Pending invitation status tracking
- Conditional navigation (only visible for Family tier subscribers)

### Technical Enhancements:

**Database Schema (Migration 0003):**
- `family_members` table with owner/member relationships
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

**13 New API Endpoints:**
- `GET /api/v1/privacy-score/history?days=30`
- `POST /api/v1/family/members`
- `GET /api/v1/family/members`
- `DELETE /api/v1/family/members/:id`
- `GET /api/v1/family/stats`
- `GET /api/v1/notifications/preferences`
- `PUT /api/v1/notifications/preferences`
- `GET /api/v1/notifications/alerts`
- `POST /api/v1/notifications/alerts/:id/read`

### Security Maintained:
- All customer PII remains encrypted with AES-256-GCM
- Master key never exposed to Python agent layer
- No raw PII sent to external LLM APIs
- SMTP credentials protected in environment variables
- Append-only audit trail for all removal actions
- Family member limit enforced at database and service layers

---

## Recent Updates

### Production and Security Hardening (Latest)

**VectorScrub has been hardened for production deployment** with enterprise-grade security, observability, and compliance features.

**Security Enhancements:**
- **CSRF Protection**: Double-submit cookie pattern protecting all state-changing requests
- **Security Headers**: Comprehensive HTTP security headers (CSP, HSTS, X-Frame-Options, etc.)
- **Rate Limiting**: Request timeout protection with configurable IP-based limiting
- **Audit Logging**: Immutable append-only audit trail for all sensitive operations
- **Circuit Breakers**: Automatic failure protection for external service dependencies
- **Input Validation**: Multi-layer validation at agent and API boundaries

**Observability and Metrics:**
- **Prometheus Metrics**: Request latency, throughput, error rates, database pool stats
- **Structured Logging**: JSON-formatted logs with correlation IDs and trace context
- **Health Checks**: Dedicated endpoints for liveness and readiness probes
- **Request Tracing**: End-to-end request ID tracking across all services

**Production Infrastructure:**
- **PyHSM Integration**: Hardware security module support for production key management
- **Circuit Breakers**: Automated failure detection and recovery for data broker APIs
- **Data Retention**: Automated cleanup of old scans, exposures, and audit logs per policy
- **SOC 2 Compliance**: Complete control framework documentation and evidence collection

**Data Broker Coverage:**
- **Modular Broker System**: Extensible registry-based architecture for data source integrations
- **4 Production Brokers**: Spokeo, WhitePages, TruthFinder, BeenVerified with dedicated implementations
- **Aggregator Pattern**: Intelligent result deduplication and confidence scoring
- **Failure Isolation**: Per-broker circuit breakers prevent cascading failures

**A/B Testing and Experimentation:**
- **Feature Flags**: Runtime-controlled feature rollouts without deployments
- **Variant Assignment**: Consistent user bucketing with metrics tracking
- **Experiment Analysis**: Statistical significance testing for product decisions

**New Documentation:**
- `AB_TESTING.md` - A/B testing framework and experiment design
- `CIRCUIT_BREAKERS.md` - Failure protection and resilience patterns
- `DATA_BROKERS.md` - Data source integration architecture
- `DATA_RETENTION.md` - Data lifecycle and retention policies
- `OBSERVABILITY.md` - Metrics, logging, and monitoring guide
- `PYHSM_INTEGRATION.md` - Hardware security module setup
- `SECURITY_FIXES.md` - P0 security improvements implemented
- `SOC2_COMPLIANCE.md` - Complete SOC 2 Type II compliance framework
- `P1_IMPROVEMENTS.md` - Priority 1 feature enhancements
- `P2_IMPROVEMENTS.md` - Priority 2 feature enhancements

**Files Added/Modified:**
- Backend: 20+ new modules including circuit breakers, audit service, metrics, CSRF middleware
- Agents: Circuit breaker integration, broker registry, PyHSM key management, validation layer
- Frontend: Error boundary component, enhanced error handling
- Database: Migration 0004 for comprehensive audit logging
- Dependencies: Prometheus client, additional security crates, PyHSM libraries

### Phase 3 Complete — Keep Me Protected

**VectorScrub Phase 3 is now production-ready.** All 12 planned tasks completed and deployed.

**What Changed:**
- **Continuous monitoring**: Scheduled rescans every 7-30 days based on subscription tier
- **Reappearance detection**: Automatic alerts when removed records come back online
- **Family protection**: Up to 4 family members under one Family plan subscription
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
