-- Migration 0020: optional, separate pilot consents.
--
-- Taking part in the pilot does not require agreeing to be interviewed or quoted. These are two
-- independent opt-ins, false unless the customer ticks them at registration. Anonymized
-- product-performance counts are covered by the Terms and need no flag.

ALTER TABLE customers
    ADD COLUMN pilot_interview_ok BOOLEAN NOT NULL DEFAULT false,
    ADD COLUMN pilot_story_ok     BOOLEAN NOT NULL DEFAULT false;
