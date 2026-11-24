-- Migration 0026: what a customer authorizes in a deletion-request campaign, and taking it back
--
-- 1. `request_kinds`: the kinds of request the customer chose for the campaign, stored with the
--    authorization. Deleting personal information and opting out of sale or sharing are separate
--    choices; the email a company receives asks only for what was chosen. Existing rows (the feature
--    has never been switched on for customers) keep both, which is what they were sent as.
-- 2. 'withdrawn': a request the customer cancelled before it was claimed for sending. It never left,
--    so it does not count against the 90-day cooldown (services/deletion.rs).
ALTER TABLE deletion_campaigns
    ADD COLUMN request_kinds TEXT[] NOT NULL DEFAULT ARRAY['delete', 'opt_out_sale']::TEXT[],
    ADD CONSTRAINT deletion_campaigns_request_kinds_check
        CHECK (cardinality(request_kinds) >= 1 AND request_kinds <@ ARRAY['delete', 'opt_out_sale']::TEXT[]);

ALTER TABLE deletion_requests DROP CONSTRAINT deletion_requests_status_check;
ALTER TABLE deletion_requests
    ADD CONSTRAINT deletion_requests_status_check
        CHECK (status IN ('queued', 'sending', 'sent', 'failed', 'replied', 'removed', 'denied', 'withdrawn')),
    ADD COLUMN withdrawn_at TIMESTAMPTZ;
