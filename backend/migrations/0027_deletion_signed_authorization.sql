-- Migration 0027: signed authorization, campaign withdrawal, and recovery of interrupted sends
--
-- 1. The signed authorization. The customer signs each campaign electronically by typing their full
--    name, after agreeing to sign electronically. The signed document (their name, the email address
--    the requests use, the agent, what each company is asked, the time and the authorization version)
--    is stored ENCRYPTED with the customer's own key (purpose `deletion.signed_authorization`), with
--    the SHA-256 of its text in the clear so a copy can be checked against it. Deleted with the
--    campaign when the account is erased (ON DELETE CASCADE, migration 0015).
-- 2. `withdrawn_at` on the campaign: once set, nothing under that authorization is sent, even a
--    request an operator puts back in the queue.
-- 3. `message_id` is now recorded when a request is CLAIMED, before the email is sent, so that after
--    an interrupted send an operator can look the message up in the mail provider's logs and decide
--    whether it left (`vectorscrub deletion interrupted | resolve`).
ALTER TABLE deletion_campaigns
    ADD COLUMN signed_document_enc BYTEA,
    ADD COLUMN signed_document_sha256 TEXT,
    ADD COLUMN esign_consent_at TIMESTAMPTZ,
    ADD COLUMN withdrawn_at TIMESTAMPTZ;
