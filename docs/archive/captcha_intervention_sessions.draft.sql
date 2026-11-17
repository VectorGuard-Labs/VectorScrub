-- Phase 3: CAPTCHA Intervention Sessions
-- Human-in-the-loop workflow for CAPTCHA solving

-- CAPTCHA session status enum
CREATE TYPE captcha_session_status AS ENUM (
    'waiting_for_customer',  -- Paused at CAPTCHA, customer notified
    'customer_solving',      -- Customer opened session and is solving
    'solved',                -- Customer solved CAPTCHA, ready to resume
    'expired',               -- Session expired (30 min timeout)
    'failed'                 -- Customer reported they couldn't solve it
);

-- CAPTCHA intervention sessions table
CREATE TABLE captcha_sessions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    customer_id UUID NOT NULL REFERENCES customers(id) ON DELETE CASCADE,
    removal_request_id UUID NOT NULL REFERENCES removal_requests(id) ON DELETE CASCADE,

    -- Session details
    broker TEXT NOT NULL,
    status captcha_session_status NOT NULL DEFAULT 'waiting_for_customer',
    session_url TEXT NOT NULL,

    -- Browser context persistence
    browser_context_path TEXT NOT NULL,
    page_url TEXT NOT NULL,
    screenshot_hash TEXT,  -- SHA-256 of CAPTCHA screenshot

    -- Timestamps
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMPTZ NOT NULL,  -- 30 minutes from creation
    solved_at TIMESTAMPTZ,
    failed_at TIMESTAMPTZ,

    -- Metadata
    user_agent TEXT,
    customer_ip TEXT,  -- For security audit trail

    -- Constraints
    CONSTRAINT captcha_session_expires_in_future CHECK (expires_at > created_at),
    CONSTRAINT solved_at_after_creation CHECK (solved_at IS NULL OR solved_at >= created_at),
    CONSTRAINT failed_at_after_creation CHECK (failed_at IS NULL OR failed_at >= created_at)
);

-- Indexes for performance
CREATE INDEX idx_captcha_sessions_customer_id ON captcha_sessions(customer_id);
CREATE INDEX idx_captcha_sessions_removal_request_id ON captcha_sessions(removal_request_id);
CREATE INDEX idx_captcha_sessions_status ON captcha_sessions(status);
CREATE INDEX idx_captcha_sessions_created_at ON captcha_sessions(created_at);
CREATE INDEX idx_captcha_sessions_expires_at ON captcha_sessions(expires_at) WHERE status IN ('waiting_for_customer', 'customer_solving');

-- Comments
COMMENT ON TABLE captcha_sessions IS 'Phase 3: Browser sessions paused for customer CAPTCHA intervention';
COMMENT ON COLUMN captcha_sessions.browser_context_path IS 'Path to saved Playwright browser context on disk';
COMMENT ON COLUMN captcha_sessions.page_url IS 'URL where CAPTCHA was encountered';
COMMENT ON COLUMN captcha_sessions.screenshot_hash IS 'SHA-256 hash of CAPTCHA screenshot for evidence';
COMMENT ON COLUMN captcha_sessions.session_url IS 'Secure URL for customer to access embedded browser';

-- Function to get pending sessions for a customer
CREATE OR REPLACE FUNCTION get_pending_captcha_sessions(p_customer_id UUID)
RETURNS TABLE (
    session_id UUID,
    broker TEXT,
    status captcha_session_status,
    session_url TEXT,
    created_at TIMESTAMPTZ,
    expires_at TIMESTAMPTZ,
    time_remaining_minutes INTEGER
) AS $$
BEGIN
    RETURN QUERY
    SELECT
        id,
        cs.broker,
        cs.status,
        cs.session_url,
        cs.created_at,
        cs.expires_at,
        GREATEST(0, EXTRACT(EPOCH FROM (cs.expires_at - NOW()))::INTEGER / 60) AS time_remaining_minutes
    FROM captcha_sessions cs
    WHERE cs.customer_id = p_customer_id
      AND cs.status IN ('waiting_for_customer', 'customer_solving')
      AND cs.expires_at > NOW()
    ORDER BY cs.created_at DESC;
END;
$$ LANGUAGE plpgsql STABLE;

-- Function to mark session as solved
CREATE OR REPLACE FUNCTION mark_captcha_session_solved(
    p_session_id UUID,
    p_customer_id UUID  -- Verify ownership
)
RETURNS BOOLEAN AS $$
DECLARE
    v_rows_updated INTEGER;
BEGIN
    UPDATE captcha_sessions
    SET
        status = 'solved',
        solved_at = NOW()
    WHERE id = p_session_id
      AND customer_id = p_customer_id
      AND status IN ('waiting_for_customer', 'customer_solving')
      AND expires_at > NOW();

    GET DIAGNOSTICS v_rows_updated = ROW_COUNT;
    RETURN v_rows_updated > 0;
END;
$$ LANGUAGE plpgsql;

-- Function to mark session as failed
CREATE OR REPLACE FUNCTION mark_captcha_session_failed(
    p_session_id UUID,
    p_customer_id UUID  -- Verify ownership
)
RETURNS BOOLEAN AS $$
DECLARE
    v_rows_updated INTEGER;
BEGIN
    UPDATE captcha_sessions
    SET
        status = 'failed',
        failed_at = NOW()
    WHERE id = p_session_id
      AND customer_id = p_customer_id
      AND status IN ('waiting_for_customer', 'customer_solving')
      AND expires_at > NOW();

    GET DIAGNOSTICS v_rows_updated = ROW_COUNT;
    RETURN v_rows_updated > 0;
END;
$$ LANGUAGE plpgsql;

-- Function to expire old sessions (run periodically)
CREATE OR REPLACE FUNCTION expire_old_captcha_sessions()
RETURNS INTEGER AS $$
DECLARE
    v_rows_updated INTEGER;
BEGIN
    UPDATE captcha_sessions
    SET status = 'expired'
    WHERE status IN ('waiting_for_customer', 'customer_solving')
      AND expires_at <= NOW();

    GET DIAGNOSTICS v_rows_updated = ROW_COUNT;
    RETURN v_rows_updated;
END;
$$ LANGUAGE plpgsql;

-- Trigger to update removal request status when CAPTCHA solved
CREATE OR REPLACE FUNCTION update_removal_request_on_captcha_solved()
RETURNS TRIGGER AS $$
BEGIN
    IF NEW.status = 'solved' AND OLD.status != 'solved' THEN
        -- Update removal request to indicate CAPTCHA was solved
        UPDATE removal_requests
        SET
            updated_at = NOW(),
            notes = COALESCE(notes, '') || E'\nCAPTCHA solved by customer at ' || NEW.solved_at::TEXT
        WHERE id = NEW.removal_request_id;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER captcha_session_solved_trigger
AFTER UPDATE ON captcha_sessions
FOR EACH ROW
WHEN (NEW.status = 'solved')
EXECUTE FUNCTION update_removal_request_on_captcha_solved();

-- Grant permissions
GRANT SELECT, INSERT, UPDATE ON captcha_sessions TO vectorscrub_backend;
GRANT EXECUTE ON FUNCTION get_pending_captcha_sessions(UUID) TO vectorscrub_backend;
GRANT EXECUTE ON FUNCTION mark_captcha_session_solved(UUID, UUID) TO vectorscrub_backend;
GRANT EXECUTE ON FUNCTION mark_captcha_session_failed(UUID, UUID) TO vectorscrub_backend;
GRANT EXECUTE ON FUNCTION expire_old_captcha_sessions() TO vectorscrub_backend;
