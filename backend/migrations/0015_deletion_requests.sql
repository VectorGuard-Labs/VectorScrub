-- Migration 0015: deletion requests sent to companies on a customer's behalf
--
-- A campaign is one customer's authorization: the exact text they accepted (and its hash), which
-- details they agreed to share, and when. It holds one request per company they selected. A
-- request is one email to one company, from the evidence-checked channel catalog
-- (data/broker_channels.json); `channel_id` refers to a catalog entry, and the address it is sent
-- to is always taken from the catalog, never stored or supplied by anyone else.
--
-- No personal data is stored here. The details that go into the email are decrypted from the
-- customer's encrypted profile at send time, and only the fields the customer agreed to share.
--
-- Status lifecycle:
--   queued -> sending -> sent | failed                      (the worker)
--   sent -> replied -> removed | denied; sent -> removed | denied   (reported by the customer,
--       whose email the company replies to; we do not read their inbox)
-- A request interrupted while 'sending' is failed with an "abandoned" marker rather than retried:
-- it is not known whether the email left, and sending twice to a company is worse than not at all.

CREATE TABLE deletion_campaigns (
    id                    UUID PRIMARY KEY,
    customer_id           UUID NOT NULL REFERENCES customers(id) ON DELETE CASCADE,
    authorization_version TEXT NOT NULL,
    authorization_text    TEXT NOT NULL,
    authorization_sha256  TEXT NOT NULL,
    include_address       BOOLEAN NOT NULL,
    authorized_at         TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX deletion_campaigns_customer_idx ON deletion_campaigns (customer_id);

CREATE TABLE deletion_requests (
    id            UUID PRIMARY KEY,
    campaign_id   UUID NOT NULL REFERENCES deletion_campaigns(id) ON DELETE CASCADE,
    customer_id   UUID NOT NULL REFERENCES customers(id) ON DELETE CASCADE,
    channel_id    TEXT NOT NULL,
    status        TEXT NOT NULL DEFAULT 'queued'
                  CHECK (status IN ('queued', 'sending', 'sent', 'failed', 'replied', 'removed', 'denied')),
    message_id    TEXT,
    error         TEXT,
    queued_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    claimed_at    TIMESTAMPTZ,
    sent_at       TIMESTAMPTZ,
    outcome_at    TIMESTAMPTZ,
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (campaign_id, channel_id)
);

CREATE INDEX deletion_requests_customer_idx ON deletion_requests (customer_id, channel_id);
CREATE INDEX deletion_requests_queue_idx ON deletion_requests (status, queued_at);
