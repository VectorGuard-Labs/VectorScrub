-- Migration 0005: Remove 'family' tier from subscription_tier enum
-- Simplified tier structure: free, personal, premium

-- Update any existing 'family' tier customers to 'premium'
UPDATE customers SET subscription_tier = 'premium' WHERE subscription_tier = 'family';

-- Recreate the enum without 'family'
-- PostgreSQL doesn't support ALTER TYPE ... DROP VALUE, so we:
-- 1. Create new enum
-- 2. Alter column to use new enum
-- 3. Drop old enum

CREATE TYPE subscription_tier_new AS ENUM ('free', 'personal', 'premium');

ALTER TABLE customers
  ALTER COLUMN subscription_tier TYPE subscription_tier_new
  USING subscription_tier::text::subscription_tier_new;

DROP TYPE subscription_tier;

ALTER TYPE subscription_tier_new RENAME TO subscription_tier;
