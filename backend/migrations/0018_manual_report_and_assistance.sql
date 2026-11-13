-- Migration 0018: what a customer reports about a by-hand opt-out, and requests for help.
--
-- manual_*: recorded when the customer says they completed an opt-out themselves. They are the
-- customer's claim (nothing here is evidence of removal; only the independent re-check can
-- reach verified_removed).
--
-- assistance_*: the customer asked for a person to help with a request automation could not do.
-- A free pilot queue worked by hand (`vectorscrub assistance ...`); not a plan or a promise.
-- No free text is collected on purpose, so nothing identifying lands in this table.

ALTER TABLE removal_requests
    ADD COLUMN manual_reported_at       TIMESTAMPTZ,
    ADD COLUMN manual_email_confirmation TEXT
        CHECK (manual_email_confirmation IN ('confirmed', 'not_required')),
    ADD COLUMN manual_submitted_on      DATE,
    ADD COLUMN assistance_requested_at  TIMESTAMPTZ,
    ADD COLUMN assistance_status        TEXT
        CHECK (assistance_status IN ('requested', 'in_progress', 'completed', 'declined')),
    ADD COLUMN assistance_minutes       INT CHECK (assistance_minutes >= 0);

CREATE INDEX removal_requests_assistance_open_idx
    ON removal_requests (assistance_requested_at)
    WHERE assistance_status IN ('requested', 'in_progress');
