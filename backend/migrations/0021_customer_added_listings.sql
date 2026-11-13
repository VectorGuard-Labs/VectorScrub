-- Migration 0021: listings the customer found themselves.
--
-- Most brokers cannot be scanned, so a customer who finds themselves on one can add the listing
-- and have it tracked and re-checked like any other. Such an exposure has no scan job, and is
-- marked origin = 'customer' so it is never mistaken for something our scan detected.

ALTER TABLE exposures ALTER COLUMN scan_job_id DROP NOT NULL;
ALTER TABLE exposures
    ADD COLUMN origin TEXT NOT NULL DEFAULT 'scan' CHECK (origin IN ('scan', 'customer'));
