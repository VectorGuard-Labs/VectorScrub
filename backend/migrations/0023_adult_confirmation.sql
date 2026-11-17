-- Migration 0023: record that the account holder confirmed being an adult.
--
-- VectorScrub is for people aged 18 or over (Terms of Service, "Who can use VectorScrub"). The
-- registration form asks for an explicit confirmation and the API refuses a registration without
-- one; this column keeps when it was given, next to the terms acceptance (migration 0016).
-- It is a declaration, not a check: we deliberately do not ask for a date of birth at signup.
-- Nullable: accounts created before this migration never made the declaration.
ALTER TABLE customers ADD COLUMN adult_confirmed_at TIMESTAMPTZ;
