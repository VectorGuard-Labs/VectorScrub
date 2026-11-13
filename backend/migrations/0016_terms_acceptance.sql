-- Which version of the Terms of Service / Privacy Policy the customer accepted, and when.
-- Nullable: accounts created before this migration have not recorded an acceptance.
ALTER TABLE customers
    ADD COLUMN terms_version TEXT,
    ADD COLUMN terms_accepted_at TIMESTAMPTZ;
