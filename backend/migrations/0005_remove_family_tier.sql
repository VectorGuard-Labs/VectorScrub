-- Migration 0005: Remove 'family' tier from subscription_tier enum
-- Simplified tier structure: free, personal, premium

-- Update existing family customers to premium
UPDATE customers
SET subscription_tier = 'premium'
WHERE subscription_tier = 'family';

-- Remove the old enum default before changing types
ALTER TABLE customers
  ALTER COLUMN subscription_tier DROP DEFAULT;

-- Create the replacement enum without family
CREATE TYPE subscription_tier_new AS ENUM ('free', 'personal', 'premium');

-- Convert the column to the new enum
ALTER TABLE customers
  ALTER COLUMN subscription_tier TYPE subscription_tier_new
  USING subscription_tier::text::subscription_tier_new;

-- Remove the old enum and rename the replacement
DROP TYPE subscription_tier;

ALTER TYPE subscription_tier_new RENAME TO subscription_tier;

-- Restore the default using the new enum
ALTER TABLE customers
  ALTER COLUMN subscription_tier SET DEFAULT 'free'::subscription_tier;

