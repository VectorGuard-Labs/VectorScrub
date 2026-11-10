-- Migration 0006: end-to-end monitoring support
--
-- 1. 'failed' removal status: automated submission could not be completed
--    (form changed, CAPTCHA, no confirmation seen, SMTP/API error). Previously the
--    agent reported "failed" but the enum had no such value, so the status update
--    was rejected and the same request was retried against the broker indefinitely.
-- 2. Monitoring is on by default. Nothing in the application ever set
--    customers.monitoring_enabled, so scheduled rescans and reappearance
--    monitoring never ran for anyone. Tier gating (personal/premium only) is
--    enforced in the queries, not by this flag.
-- 3. Index for the removal-verification query.

ALTER TYPE removal_status ADD VALUE IF NOT EXISTS 'failed';

ALTER TABLE customers
  ALTER COLUMN monitoring_enabled SET DEFAULT true;

-- There has never been a way to opt out, so every existing 'false' is the old default.
UPDATE customers SET monitoring_enabled = true WHERE monitoring_enabled = false;

CREATE INDEX IF NOT EXISTS idx_removal_requests_verification
ON removal_requests(next_check_at)
WHERE status IN ('submitted', 'acknowledged');
