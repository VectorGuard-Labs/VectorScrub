-- Migration 0012: which sources a scan could and could not read
--
-- A scan that reads 3 of 19 sites and finds nothing on the other 16 (blocked, moved, down)
-- used to look identical to one that read all 19 and found nothing. `coverage` records, per
-- source, what happened: a JSON array of {"source": "<site hostname>", "outcome": one of
-- checked | blocked | not_found | unavailable | unreadable}. Only public site names and
-- an outcome: no personal data, no URLs (they embed the customer's name).
--
-- NULL for scans that ran before this migration. `sources_queued` (which was never filled in)
-- is set to the number of sources when coverage is recorded.

ALTER TABLE scan_jobs ADD COLUMN coverage JSONB;
