-- Migration 007: Removal Retry System
--
-- Adds support for customer-initiated retries of failed removal requests.
-- Tracks retry attempts, enforces limits, and prevents abuse.

-- Add retry tracking columns to removal_requests table
ALTER TABLE removal_requests
ADD COLUMN IF NOT EXISTS retry_count INTEGER NOT NULL DEFAULT 0,
ADD COLUMN IF NOT EXISTS max_retries INTEGER NOT NULL DEFAULT 3,
ADD COLUMN IF NOT EXISTS last_retry_at TIMESTAMP,
ADD COLUMN IF NOT EXISTS retry_cooldown_until TIMESTAMP,
ADD COLUMN IF NOT EXISTS retry_initiated_by TEXT; -- 'customer' or 'system'

-- Create index for retry queries
CREATE INDEX IF NOT EXISTS idx_removal_requests_retry
ON removal_requests(customer_id, status, retry_count)
WHERE status = 'failed';

-- Create retry history table for audit trail
CREATE TABLE IF NOT EXISTS removal_retry_history (
    id SERIAL PRIMARY KEY,
    removal_request_id UUID NOT NULL REFERENCES removal_requests(id) ON DELETE CASCADE,
    customer_id UUID NOT NULL,
    retry_number INTEGER NOT NULL,
    initiated_by TEXT NOT NULL, -- 'customer' or 'system'
    initiated_at TIMESTAMP NOT NULL DEFAULT NOW(),
    completed_at TIMESTAMP,
    status TEXT NOT NULL, -- 'pending', 'success', 'failed'
    error_message TEXT,
    evidence_hash TEXT,
    confirmation_ref TEXT,

    CONSTRAINT valid_status CHECK (status IN ('pending', 'success', 'failed')),
    CONSTRAINT valid_initiator CHECK (initiated_by IN ('customer', 'system'))
);

CREATE INDEX IF NOT EXISTS idx_retry_history_request
ON removal_retry_history(removal_request_id, retry_number);

CREATE INDEX IF NOT EXISTS idx_retry_history_customer
ON removal_retry_history(customer_id, initiated_at DESC);

-- Create retry rate limiting table (per customer, per day)
CREATE TABLE IF NOT EXISTS removal_retry_rate_limits (
    customer_id UUID NOT NULL,
    date DATE NOT NULL,
    retry_count INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (customer_id, date)
);

-- Function to check if retry is allowed
CREATE OR REPLACE FUNCTION can_retry_removal(
    p_removal_request_id UUID,
    p_customer_id UUID
) RETURNS TABLE (
    allowed BOOLEAN,
    reason TEXT,
    retry_count INTEGER,
    max_retries INTEGER,
    cooldown_until TIMESTAMP,
    daily_limit_reached BOOLEAN
) AS $$
DECLARE
    v_retry_count INTEGER;
    v_max_retries INTEGER;
    v_cooldown_until TIMESTAMP;
    v_status TEXT;
    v_daily_count INTEGER;
    v_daily_limit INTEGER := 10; -- Max 10 retries per day per customer
BEGIN
    -- Get current retry info
    SELECT
        r.retry_count,
        r.max_retries,
        r.retry_cooldown_until,
        r.status
    INTO
        v_retry_count,
        v_max_retries,
        v_cooldown_until,
        v_status
    FROM removal_requests r
    WHERE r.id = p_removal_request_id
    AND r.customer_id = p_customer_id;

    -- Check if request exists
    IF NOT FOUND THEN
        RETURN QUERY SELECT FALSE, 'Removal request not found', 0, 0, NULL::TIMESTAMP, FALSE;
        RETURN;
    END IF;

    -- Check if already succeeded
    IF v_status = 'submitted' THEN
        RETURN QUERY SELECT FALSE, 'Removal already submitted successfully', v_retry_count, v_max_retries, v_cooldown_until, FALSE;
        RETURN;
    END IF;

    -- Check retry limit
    IF v_retry_count >= v_max_retries THEN
        RETURN QUERY SELECT FALSE, 'Maximum retry limit reached', v_retry_count, v_max_retries, v_cooldown_until, FALSE;
        RETURN;
    END IF;

    -- Check cooldown
    IF v_cooldown_until IS NOT NULL AND v_cooldown_until > NOW() THEN
        RETURN QUERY SELECT FALSE, 'Retry cooldown active', v_retry_count, v_max_retries, v_cooldown_until, FALSE;
        RETURN;
    END IF;

    -- Check daily limit
    SELECT COALESCE(retry_count, 0)
    INTO v_daily_count
    FROM removal_retry_rate_limits
    WHERE customer_id = p_customer_id
    AND date = CURRENT_DATE;

    IF v_daily_count >= v_daily_limit THEN
        RETURN QUERY SELECT FALSE, 'Daily retry limit reached', v_retry_count, v_max_retries, v_cooldown_until, TRUE;
        RETURN;
    END IF;

    -- All checks passed
    RETURN QUERY SELECT TRUE, 'Retry allowed', v_retry_count, v_max_retries, v_cooldown_until, FALSE;
END;
$$ LANGUAGE plpgsql SECURITY DEFINER;

-- Function to initiate retry
CREATE OR REPLACE FUNCTION initiate_removal_retry(
    p_removal_request_id UUID,
    p_customer_id UUID,
    p_initiated_by TEXT DEFAULT 'customer'
) RETURNS UUID AS $$
DECLARE
    v_retry_history_id UUID;
    v_retry_number INTEGER;
BEGIN
    -- Verify retry is allowed (function will raise exception if not)
    PERFORM * FROM can_retry_removal(p_removal_request_id, p_customer_id)
    WHERE allowed = TRUE;

    IF NOT FOUND THEN
        RAISE EXCEPTION 'Retry not allowed for this removal request';
    END IF;

    -- Get next retry number
    SELECT retry_count + 1
    INTO v_retry_number
    FROM removal_requests
    WHERE id = p_removal_request_id;

    -- Create retry history entry
    INSERT INTO removal_retry_history (
        removal_request_id,
        customer_id,
        retry_number,
        initiated_by,
        status
    ) VALUES (
        p_removal_request_id,
        p_customer_id,
        v_retry_number,
        p_initiated_by,
        'pending'
    )
    RETURNING id INTO v_retry_history_id;

    -- Update removal request
    UPDATE removal_requests
    SET
        retry_count = retry_count + 1,
        last_retry_at = NOW(),
        retry_cooldown_until = NOW() + INTERVAL '1 hour', -- 1 hour cooldown between retries
        retry_initiated_by = p_initiated_by,
        status = 'pending' -- Reset to pending for retry
    WHERE id = p_removal_request_id;

    -- Update daily rate limit
    INSERT INTO removal_retry_rate_limits (customer_id, date, retry_count)
    VALUES (p_customer_id, CURRENT_DATE, 1)
    ON CONFLICT (customer_id, date)
    DO UPDATE SET retry_count = removal_retry_rate_limits.retry_count + 1;

    RETURN v_retry_history_id;
END;
$$ LANGUAGE plpgsql SECURITY DEFINER;

-- Comments for documentation
COMMENT ON TABLE removal_retry_history IS 'Audit trail of all removal retry attempts';
COMMENT ON TABLE removal_retry_rate_limits IS 'Daily rate limits for customer-initiated retries';
COMMENT ON FUNCTION can_retry_removal IS 'Check if a removal request can be retried';
COMMENT ON FUNCTION initiate_removal_retry IS 'Initiate a retry for a failed removal request';
