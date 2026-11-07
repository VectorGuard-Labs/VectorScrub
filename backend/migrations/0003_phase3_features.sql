-- Phase 3: Keep Me Protected
-- Migration 0003 — scheduled monitoring, family accounts, privacy score trends, alerts

-- Family member associations (up to 5 members per family plan)
CREATE TABLE family_members (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    owner_id        UUID NOT NULL REFERENCES customers(id) ON DELETE CASCADE,
    member_id       UUID NOT NULL REFERENCES customers(id) ON DELETE CASCADE,
    role            TEXT NOT NULL CHECK (role IN ('owner', 'member')),
    invited_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    accepted_at     TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (owner_id, member_id)
);

CREATE INDEX idx_family_members_owner_id ON family_members(owner_id);
CREATE INDEX idx_family_members_member_id ON family_members(member_id);

-- Privacy score history for trend tracking
CREATE TABLE privacy_score_history (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    customer_id         UUID NOT NULL REFERENCES customers(id) ON DELETE CASCADE,
    score               REAL NOT NULL CHECK (score >= 0 AND score <= 100),
    critical_count      INT NOT NULL DEFAULT 0,
    high_count          INT NOT NULL DEFAULT 0,
    medium_count        INT NOT NULL DEFAULT 0,
    low_count           INT NOT NULL DEFAULT 0,
    total_exposures     INT NOT NULL DEFAULT 0,
    recorded_at         TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_privacy_score_history_customer_id ON privacy_score_history(customer_id, recorded_at DESC);

-- Notification preferences for email and SMS alerts
CREATE TABLE notification_preferences (
    id                      UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    customer_id             UUID NOT NULL UNIQUE REFERENCES customers(id) ON DELETE CASCADE,
    email_enabled           BOOLEAN NOT NULL DEFAULT true,
    sms_enabled             BOOLEAN NOT NULL DEFAULT false,
    phone_number            TEXT,
    alert_new_exposures     BOOLEAN NOT NULL DEFAULT true,
    alert_removal_failures  BOOLEAN NOT NULL DEFAULT true,
    alert_reappearances     BOOLEAN NOT NULL DEFAULT true,
    created_at              TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at              TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Add monitoring capabilities to customers
ALTER TABLE customers
ADD COLUMN monitoring_enabled BOOLEAN NOT NULL DEFAULT false;

-- Add monitoring timestamp to exposures for reappearance tracking
ALTER TABLE exposures
ADD COLUMN last_monitored_at TIMESTAMPTZ;

-- Index for monitoring query: find removed exposures due for re-check
CREATE INDEX idx_exposures_monitoring
ON exposures(customer_id, last_monitored_at)
WHERE is_active = false;
