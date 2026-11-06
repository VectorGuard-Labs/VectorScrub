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
18. [Competitive Landscape](#competitive-landscape)
19. [VectorGuard Labs Product Family](#vectorguard-labs-product-family)
20. [Contributing](#contributing)
21. [License](#license)

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

### 3. Automated Removal Request Preparation
For each confirmed exposure, VectorScrub identifies the correct opt-out procedure for that specific data source — web form, email submission, or the California DROP API. It prepares a complete, authorized removal request. No request is ever submitted without explicit customer authorization.

### 4. Removal Tracking and Verification
Every removal request is tracked through its full lifecycle: authorized → submitted → acknowledged → verified removed. VectorScrub distinguishes between these states clearly so customers always know the true status of their records — not just that a request was sent, but whether removal was actually confirmed.

### 5. Continuous Monitoring
After removals, VectorScrub continues to monitor sources on a scheduled basis. If a removed record reappears (which happens frequently as data brokers re-acquire information), an alert is sent and a new removal request is prepared automatically.

### 6. Privacy Score and Exposure Report
Customers see a live **Privacy Score** (0–100) that reflects their current exposure level. The score decreases based on the severity and volume of active exposures, and increases as records are removed. A full exposure history shows every finding, removal, and reappearance over time.

---

## Benefits to End Users

### Know exactly what's out there
Most people discover they have dozens of active exposures on their first scan. VectorScrub surfaces the full picture in a single dashboard rather than requiring customers to check each site manually.

### Get it removed — with proof
VectorScrub prepares and submits opt-out requests to data brokers, tracks responses, and records evidence of verified removals. Customers see a clear before-and-after record, not just a promise that "we tried."

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
  │                     │  coordinates agent pipeline
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

| Layer      | Technology                              | Responsibility                               |
|------------|-----------------------------------------|----------------------------------------------|
| Backend    | Rust · Axum · SQLx · AES-256-GCM        | API, auth (Argon2id + JWT), PII encryption, audit trail |
| Agents     | Python · LangChain · httpx · Playwright | Discovery, detection, removal, monitoring    |
| Frontend   | Next.js 15 · Tailwind CSS 4 · TypeScript | Landing page, auth, customer dashboard       |
| Database   | PostgreSQL 16                           | All persistent state — PII stored encrypted  |
| Queue      | Redis 7                                 | Scan job dispatch from Rust to Python        |

**Why Rust for the backend?** Memory safety, no garbage collector, predictable performance under load, and a strong cryptography ecosystem. The backend handles every operation that touches raw customer PII — authentication, encryption, database writes, and the audit trail.

**Why Python for the agents?** The LLM orchestration ecosystem (LangChain, LangGraph) is mature and moves quickly in Python. Web scraping and browser automation tools (Playwright, BeautifulSoup) are best-in-class. Rapid iteration on prompts and agent logic is straightforward. The Python layer never holds encryption keys.

---

## Security Model

Security is the core architectural constraint, not an afterthought. Every design decision in VectorScrub is made with the understanding that customers are handing over exactly the information they want to protect.

| Concern | How VectorScrub handles it |
|---|---|
| PII at rest | AES-256-GCM encryption with a per-customer key derived from the master key via HMAC-SHA256. Each customer's ciphertext can only be decrypted with their derived key. |
| Master key | Loaded once into the Rust process from an environment variable (or PyHSM in production). Never written to disk, never accessible to the Python agent layer. |
| PII and LLMs | Raw customer values are never sent to any external LLM API. The detection agent receives only anonymized tokens (first letter + asterisks) for pattern matching. |
| Removal authorization | No removal request is ever submitted to a data source without an explicit customer authorization recorded in the database. The agent prepares; the customer approves. |
| Audit trail | Every action taken on a removal request is recorded in an append-only audit log. The database enforces `NO UPDATE` and `NO DELETE` rules on the audit table. |
| Model training | Customer PII is never used to fine-tune or train any AI model — not VectorScrub's, not a provider's. |
| Agent authentication | Rust internal endpoints accept only the `X-Agent-Key` header, not customer JWTs. The agent key is never exposed to the frontend. |
| Tenant isolation | Per-customer encryption keys mean a compromise of one customer's encrypted data does not affect others. |
| Password storage | Argon2id with random per-user salt. No plaintext or reversibly encoded passwords stored anywhere. |

---

## Subscription Plans

| Plan     | Price/month | Key Features |
|----------|-------------|--------------|
| **Free**     | $0     | Initial exposure scan, privacy score, exposure report |
| **Personal** | $8.99  | Automated removal requests, continuous monitoring, monthly rescan, alerts |
| **Premium**  | $14.99 | Everything in Personal + expanded source coverage, priority removal, weekly rescan, evidence collection |
| **Family**   | $24.99 | Everything in Premium + up to 5 family members, shared dashboard |

Free accounts are limited to one scan per 24 hours. Paid accounts can initiate multiple scans per day and have access to a wider range of data sources depending on tier.

---

## Project Structure

```
VectorScrub/
│
├── backend/                          # Rust API service
│   ├── Cargo.toml                    # Dependencies and build config
│   ├── migrations/
│   │   └── 0001_initial_schema.sql   # Full PostgreSQL schema
│   └── src/
│       ├── main.rs                   # Entry point, server setup
│       ├── db.rs                     # Shared AppState (pool, redis, config)
│       ├── api/
│       │   ├── mod.rs                # Route table
│       │   ├── auth.rs               # POST /auth/register, /auth/login
│       │   ├── customers.rs          # GET /me, POST /me/identity
│       │   ├── scans.rs              # POST /scans, GET /scans, GET /scans/:id
│       │   ├── exposures.rs          # GET /exposures, GET /exposures/summary
│       │   └── agent_callbacks.rs    # Internal: PUT /internal/scan/status, POST /internal/exposures
│       ├── crypto/
│       │   └── mod.rs                # AES-256-GCM encrypt/decrypt, key derivation
│       ├── middleware/
│       │   ├── mod.rs                # CORS, tracing, timeout, request IDs
│       │   └── auth_extractor.rs     # JWT extractor, agent key extractor
│       ├── models/
│       │   ├── customer.rs           # Customer, IdentityProfile, CleartextIdentity
│       │   ├── exposure.rs           # Exposure, ExposureSummary
│       │   ├── scan_job.rs           # ScanJob, ScanJobPayload
│       │   └── removal_request.rs    # RemovalRequest, RemovalAuditEntry
│       ├── services/
│       │   ├── auth.rs               # Argon2id hashing, JWT issue/validate
│       │   ├── config.rs             # AppConfig from env vars
│       │   ├── customer.rs           # Account creation, identity encrypt/decrypt
│       │   ├── exposure.rs           # Record exposures, privacy score calculation
│       │   └── scan.rs               # Scan job lifecycle, Redis dispatch
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
│   │   └── agent.py                  # Opt-out procedure registry + request preparation
│   ├── monitoring/
│   │   └── agent.py                  # Reappearance detection for removed records
│   └── orchestrator/
│       └── orchestrator.py           # Redis worker, coordinates full pipeline
│
├── frontend/                         # Next.js 15 customer dashboard
│   ├── package.json
│   ├── next.config.ts                # API proxy rewrites
│   ├── tsconfig.json
│   └── src/
│       ├── app/
│       │   ├── layout.tsx            # Root layout, metadata
│       │   ├── globals.css           # Tailwind CSS 4 base styles
│       │   ├── page.tsx              # Public landing page
│       │   ├── register/page.tsx     # Account creation
│       │   ├── login/page.tsx        # Sign in
│       │   └── dashboard/page.tsx    # Privacy dashboard (authenticated)
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
└── README.md                         # This file
```

---

## Prerequisites

Before setting up VectorScrub locally, make sure you have the following installed:

| Tool | Version | Purpose |
|------|---------|---------|
| Rust | 1.82+ | Backend compilation (`rustup` recommended) |
| Python | 3.11+ | AI agent layer |
| Node.js | 20+ | Next.js frontend |
| Docker | 24+ | PostgreSQL and Redis |
| Docker Compose | 2.20+ | Multi-service orchestration |
| OpenSSL | Any | Key generation script |

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

### Step 6 — Start the Python agent worker

Open a new terminal tab for this.

```bash
cd agents
python3 -m venv .venv
source .venv/bin/activate
pip install -r requirements.txt
python -m orchestrator.orchestrator
```

The worker connects to Redis and blocks waiting for scan jobs:

```
{"event":"orchestrator_worker_started","queue":"vectorscrub:scan_jobs"}
```

### Step 7 — Start the frontend

Open another terminal tab.

```bash
cd frontend
npm install
npm run dev
```

Open [http://localhost:3000](http://localhost:3000) in your browser. You should see the VectorScrub landing page.

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
| `MASTER_ENCRYPTION_KEY` | Yes | Base64-encoded 32-byte AES master key. Generate with `openssl rand -base64 32` |
| `PORT` | No | Port for the Rust API (default: `8080`) |
| `ENVIRONMENT` | No | `development`, `staging`, or `production` (default: `development`) |
| `DB_POOL_SIZE` | No | PostgreSQL connection pool size (default: `10`) |
| `AGENT_API_KEY` | Yes | Shared secret for Python agent → Rust internal API authentication |
| `BACKEND_API_URL` | No | URL the Python agents use to reach the Rust API (default: `http://localhost:8080`) |
| `OPENAI_API_KEY` | Recommended | OpenAI API key for the detection agent. One of OpenAI or Anthropic required. |
| `ANTHROPIC_API_KEY` | Optional | Anthropic API key as an alternative to OpenAI |
| `DEFAULT_LLM_PROVIDER` | No | `openai` or `anthropic` (default: `openai`) |
| `NEXT_PUBLIC_API_URL` | No | URL the Next.js frontend uses for API calls (default: `http://localhost:8080`) |

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

# Run the orchestrator worker
python -m orchestrator.orchestrator

# Run individual agents in isolation for testing
python -c "
import asyncio
from discovery.agent import DiscoveryAgent
from shared.models import IdentityContext
import uuid

identity = IdentityContext(
    customer_id=uuid.uuid4(),
    full_name='Jane Smith',
    current_address='123 Main Street, Los Angeles, CA',
    email_addresses=['jane@example.com'],
    phone_numbers=['555-123-4567'],
)
results = asyncio.run(DiscoveryAgent(tier='personal').run(identity))
print(f'Found {len(results)} candidates')
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

### Internal Agent Endpoints

| Method | Endpoint | Auth | Description |
|--------|----------|------|-------------|
| `PUT` | `/internal/scan/status` | Agent key | Update scan job progress (called by the orchestrator). |
| `POST` | `/internal/exposures` | Agent key | Record a confirmed exposure found by the detection agent. |

---

## Database Schema

The PostgreSQL schema is defined in `backend/migrations/0001_initial_schema.sql` and applied automatically on backend startup.

| Table | Purpose |
|-------|---------|
| `customers` | Account records: email, hashed password, subscription tier, privacy score |
| `identity_profiles` | Per-customer encrypted PII: all fields stored as AES-256-GCM ciphertext blobs |
| `scan_jobs` | Scan lifecycle: queued, running, completed, failed; sources and exposures counted |
| `exposures` | Every discovered PII exposure: source, category, severity, masked summary, active/removed state |
| `removal_requests` | Opt-out requests: status lifecycle from authorized through verified removed |
| `removal_audit_log` | Append-only tamper-evident log of every action on every removal request |
| `alerts` | Customer notifications: new exposures, removal failures, reappearances |

**Privacy Score Formula:**
```
score = max(0, 100 - (critical × 25 + high × 10 + medium × 5 + low × 2))
```

A customer with no active exposures scores 100. Each critical exposure (e.g., full name + full address + phone) deducts 25 points.

---

## MVP Roadmap

### Phase 1 — Find My Information ✅ Built

- Customer account creation and authentication
- Encrypted identity profile submission
- AI-powered discovery across 8+ data broker sources (tier-gated)
- LLM-based PII detection with anonymized token matching
- Exposure dashboard with privacy score
- Scan job lifecycle management

### Phase 2 — Remove My Information (Next)

- Opt-out form submission via Playwright browser automation
- Email-based opt-out for sources that require it
- California DROP API integration for 600+ brokers (California residents)
- Removal request status tracking (submitted → acknowledged → verified)
- Evidence collection (screenshot hashes, confirmation references)
- Before/after exposure reports

### Phase 3 — Keep Me Protected (Roadmap)

- Scheduled rescans (weekly for Premium/Family, monthly for Personal)
- Reappearance detection and automatic re-removal
- Family accounts (up to 5 members under one subscription)
- Privacy score trend charts
- Email and SMS alerts
- Exposure history timeline

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

*Built with security-first principles by VectorGuard Labs.*
*"Your personal data shouldn't be out there. We'll help take it back."*
