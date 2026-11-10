-- Migration 0009: removal request integrity
--
-- 1. One in-flight request per exposure.
--    authorize_removal checked "is a request already in flight?" with a plain SELECT and
--    then INSERTed. Two concurrent authorizations could both pass the check and create two
--    'authorized' rows, i.e. two submissions to the same broker. The database now enforces
--    the rule, and the service maps the violation to a 409.
--
-- 2. A durable submission claim.
--    The removal worker used to remember what it had submitted in process memory only, so
--    a restart, or a second worker replica, could submit the same request again.
--    submission_claimed_at is set by an atomic UPDATE (services/removal.rs claim_submission)
--    that succeeds for exactly one worker. A claim that is never resolved (worker crashed
--    mid-submission) is failed by the recovery loop after a timeout; the customer is told
--    and can re-authorize. The failure mode is therefore "at most once", never "twice".

ALTER TABLE removal_requests
    ADD COLUMN submission_claimed_at TIMESTAMPTZ;

-- Before the unique index can exist, in-flight duplicates that already exist must go.
-- Only requests that were never submitted ('authorized') are touched: of several in-flight
-- requests for one exposure the oldest is kept and the newer, still-unsent 'authorized' ones
-- are ended as 'failed' (a legal transition; the customer may re-authorize). If duplicates
-- exist that were actually submitted, index creation fails loudly and an operator must
-- resolve them by hand rather than this migration guessing.
UPDATE removal_requests r
SET status = 'failed', updated_at = NOW()
WHERE r.status = 'authorized'
  AND EXISTS (
      SELECT 1 FROM removal_requests o
      WHERE o.exposure_id = r.exposure_id
        AND o.id <> r.id
        AND o.status IN ('authorized', 'submitted', 'acknowledged')
        AND (o.created_at < r.created_at OR (o.created_at = r.created_at AND o.id < r.id))
  );

CREATE UNIQUE INDEX idx_removal_requests_one_in_flight
    ON removal_requests (exposure_id)
    WHERE status IN ('authorized', 'submitted', 'acknowledged');

-- Recovery of abandoned claims scans authorized rows that carry a claim.
CREATE INDEX idx_removal_requests_claimed
    ON removal_requests (submission_claimed_at)
    WHERE status = 'authorized' AND submission_claimed_at IS NOT NULL;
