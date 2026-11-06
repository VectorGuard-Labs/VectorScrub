-- VectorScrub initial schema
-- Migration 0001 — initial schema

-- Custom enum types
CREATE TYPE subscription_tier AS ENUM ('free', 'personal', 'premium', 'family');
CREATE TYPE pii_category AS ENUM (
    'full_name', 'home_address', 'phone_number', 'email_address',
    'date_of_birth', 'family_association', 'employment_history',
    'social_media_handle', 'vehicle_record', 'court_record', 'other'
);
CREATE TYPE exposure_severity AS ENUM ('low', 'medium', 'high', 'critical');
CREATE TYPE scan_job_status AS ENUM ('queued', 'running', 'completed', 'failed', 'cancelled');
CREATE TYPE removal_status AS ENUM (
    'authorized', 'submitted', 'acknowledged',
    'verified_removed', 'rejected', 'unverifiable', 'reappeared'
);

-- Customers
CREATE TABLE customers (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    email             TEXT NOT NULL UNIQUE,
    password_hash     TEXT NOT NULL,
    subscription_tier subscription_tier NOT NULL DEFAULT 'free',
    privacy_score     REAL,
    is_verified       BOOLEAN NOT NULL DEFAULT false,
    is_active         BOOLEAN NOT NULL DEFAULT true,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_scan_at      TIMESTAMPTZ
);

CREATE INDEX idx_customers_email ON customers(email);

-- Identity profiles (all PII fields encrypted at rest)
CREATE TABLE identity_profiles (
    id                       UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    customer_id              UUID NOT NULL UNIQUE REFERENCES customers(id) ON DELETE CASCADE,
    full_name_enc            BYTEA NOT NULL,
    address_enc              BYTEA NOT NULL,
    previous_addresses_enc   BYTEA NOT NULL,
    phone_numbers_enc        BYTEA NOT NULL,
    email_addresses_enc      BYTEA NOT NULL,
    date_of_birth_enc        BYTEA,
    created_at               TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at               TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Scan jobs
CREATE TABLE scan_jobs (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    customer_id       UUID NOT NULL REFERENCES customers(id) ON DELETE CASCADE,
    status            scan_job_status NOT NULL DEFAULT 'queued',
    sources_queued    INT NOT NULL DEFAULT 0,
    sources_completed INT NOT NULL DEFAULT 0,
    sources_failed    INT NOT NULL DEFAULT 0,
    exposures_found   INT NOT NULL DEFAULT 0,
    error_message     TEXT,
    queued_at         TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    started_at        TIMESTAMPTZ,
    completed_at      TIMESTAMPTZ
);

CREATE INDEX idx_scan_jobs_customer_id ON scan_jobs(customer_id);
CREATE INDEX idx_scan_jobs_status ON scan_jobs(status);

-- Exposures discovered by the AI agents
CREATE TABLE exposures (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    customer_id      UUID NOT NULL REFERENCES customers(id) ON DELETE CASCADE,
    scan_job_id      UUID NOT NULL REFERENCES scan_jobs(id),
    source_name      TEXT NOT NULL,
    source_url       TEXT NOT NULL,
    pii_category     pii_category NOT NULL,
    severity         exposure_severity NOT NULL,
    pii_summary      TEXT NOT NULL,   -- masked/truncated, never raw PII
    confidence       REAL NOT NULL CHECK (confidence >= 0.0 AND confidence <= 1.0),
    is_active        BOOLEAN NOT NULL DEFAULT true,
    discovered_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_verified_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    removed_at       TIMESTAMPTZ,
    UNIQUE (customer_id, source_name, pii_category)
);

CREATE INDEX idx_exposures_customer_id ON exposures(customer_id);
CREATE INDEX idx_exposures_is_active ON exposures(is_active);

-- Data removal requests
CREATE TABLE removal_requests (
    id                    UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    customer_id           UUID NOT NULL REFERENCES customers(id) ON DELETE CASCADE,
    exposure_id           UUID NOT NULL REFERENCES exposures(id),
    source_name           TEXT NOT NULL,
    source_url            TEXT NOT NULL,
    status                removal_status NOT NULL DEFAULT 'authorized',
    procedure_used        TEXT NOT NULL DEFAULT 'pending',
    confirmation_ref      TEXT,
    evidence_hash         TEXT,
    customer_authorized_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    submitted_at          TIMESTAMPTZ,
    verified_at           TIMESTAMPTZ,
    next_check_at         TIMESTAMPTZ,
    created_at            TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at            TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_removal_requests_customer_id ON removal_requests(customer_id);
CREATE INDEX idx_removal_requests_status ON removal_requests(status);

-- Tamper-evident audit log (append-only, no updates/deletes)
CREATE TABLE removal_audit_log (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    removal_request_id  UUID NOT NULL REFERENCES removal_requests(id),
    customer_id         UUID NOT NULL REFERENCES customers(id),
    action              TEXT NOT NULL,
    actor               TEXT NOT NULL,
    details             JSONB NOT NULL DEFAULT '{}',
    recorded_at         TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Prevent updates and deletes on the audit log (integrity enforcement)
CREATE RULE no_update_audit AS ON UPDATE TO removal_audit_log DO INSTEAD NOTHING;
CREATE RULE no_delete_audit AS ON DELETE TO removal_audit_log DO INSTEAD NOTHING;

CREATE INDEX idx_audit_log_removal_request_id ON removal_audit_log(removal_request_id);
CREATE INDEX idx_audit_log_customer_id ON removal_audit_log(customer_id);

-- Notification/alert log
CREATE TABLE alerts (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    customer_id  UUID NOT NULL REFERENCES customers(id) ON DELETE CASCADE,
    alert_type   TEXT NOT NULL,   -- 'new_exposure', 'removal_failed', 'reappearance'
    title        TEXT NOT NULL,
    message      TEXT NOT NULL,
    is_read      BOOLEAN NOT NULL DEFAULT false,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_alerts_customer_id ON alerts(customer_id);
CREATE INDEX idx_alerts_is_read ON alerts(is_read);
