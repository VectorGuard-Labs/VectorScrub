-- Migration 0008: scan job recovery
--
-- Before this, a scan job lived in two places that could disagree: a Redis list
-- entry (popped by BRPOP, i.e. destroyed on read) and a scan_jobs row. If the worker
-- crashed after popping, the job was gone from Redis and stayed 'queued' or
-- 'running' in Postgres forever. The row is now the source of truth:
--
--   attempts      how many times a worker has claimed the job; also the fencing
--                 token, so a worker that was presumed dead and comes back cannot
--                 overwrite the result of the worker that took over
--   heartbeat_at  last sign of life from the worker holding the job
--   dispatched_at when the job was last put on the Redis queue
--
-- A recovery loop in the backend requeues running jobs whose heartbeat has gone
-- stale, re-dispatches queued jobs whose Redis entry may have been lost, and fails
-- jobs that keep dying. Workers claim a job atomically before processing, so a
-- duplicate queue entry is harmless.

ALTER TABLE scan_jobs
    ADD COLUMN attempts      INT NOT NULL DEFAULT 0,
    ADD COLUMN heartbeat_at  TIMESTAMPTZ,
    ADD COLUMN dispatched_at TIMESTAMPTZ;

-- Existing queued jobs were dispatched when they were queued.
UPDATE scan_jobs SET dispatched_at = queued_at WHERE dispatched_at IS NULL;

CREATE INDEX idx_scan_jobs_recovery
    ON scan_jobs (status, heartbeat_at, dispatched_at)
    WHERE status IN ('queued', 'running');
