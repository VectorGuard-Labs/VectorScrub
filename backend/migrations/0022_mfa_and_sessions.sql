-- Migration 0022: two-factor authentication and server-side sessions.
--
-- Sessions. A login used to produce a 24 hour JWT that nothing on the server could end: logout
-- only cleared the cookie, so a copied token stayed valid until it expired. Every token now
-- carries the id of a row here (the `jti` claim) and is accepted only while that row is live:
-- not revoked, not past `expires_at` (the absolute limit, 24 h) and used within the idle
-- window (see services/session.rs). Deleting or revoking the row ends the session at once.
--
-- Two-factor. `customer_mfa` holds a TOTP secret per customer, encrypted with the customer's
-- own key like the rest of their sensitive data. A row with `enabled_at` NULL is a setup that
-- was started but not confirmed with a code, and gives no protection and requires nothing at
-- login. `last_used_step` is the newest 30-second step already accepted, so a code that was
-- seen once (shoulder-surfed, intercepted) cannot be replayed. Recovery codes are stored only
-- as SHA-256 hashes of 50-bit random codes and are single use.
--
-- All three tables hang off customers with ON DELETE CASCADE, so account erasure removes them
-- with no change to services/account_deletion.rs. None of them holds anything the audit trail
-- needs to keep.

CREATE TABLE sessions (
    id           UUID PRIMARY KEY,
    customer_id  UUID NOT NULL REFERENCES customers(id) ON DELETE CASCADE,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_seen_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at   TIMESTAMPTZ NOT NULL,
    revoked_at   TIMESTAMPTZ,
    -- For the "where am I signed in" list. Free text from the client, capped by the application.
    user_agent   TEXT
);
CREATE INDEX sessions_customer_live_idx ON sessions (customer_id) WHERE revoked_at IS NULL;
CREATE INDEX sessions_expires_idx ON sessions (expires_at);

CREATE TABLE customer_mfa (
    customer_id    UUID PRIMARY KEY REFERENCES customers(id) ON DELETE CASCADE,
    secret_enc     BYTEA NOT NULL,
    enabled_at     TIMESTAMPTZ,
    last_used_step BIGINT NOT NULL DEFAULT 0,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE mfa_recovery_codes (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    customer_id UUID NOT NULL REFERENCES customers(id) ON DELETE CASCADE,
    code_hash   TEXT NOT NULL,
    used_at     TIMESTAMPTZ,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (customer_id, code_hash)
);
