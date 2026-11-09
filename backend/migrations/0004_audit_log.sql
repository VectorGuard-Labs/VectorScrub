-- Audit log table for tracking sensitive operations
-- Stores immutable records of all security-relevant actions for compliance and forensics

CREATE TABLE IF NOT EXISTS audit_log (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),

    -- When and who
    timestamp TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    customer_id UUID REFERENCES customers(id) ON DELETE SET NULL,
    user_email VARCHAR(255),  -- Denormalized for resilience (survives customer deletion)

    -- What action
    action VARCHAR(100) NOT NULL,  -- e.g., "user.register", "scan.initiate", "exposure.discover"
    resource_type VARCHAR(50) NOT NULL,  -- e.g., "customer", "scan", "exposure", "removal_request"
    resource_id UUID,  -- ID of the affected resource (scan_id, exposure_id, etc.)

    -- Context
    ip_address TEXT,  -- Source IP (for security analysis)
    user_agent TEXT,  -- Browser/client info

    -- Details (JSON for flexibility)
    details JSONB NOT NULL DEFAULT '{}'::jsonb,  -- Additional metadata (success/failure, error messages, etc.)

    -- Outcome
    status VARCHAR(20) NOT NULL DEFAULT 'success',  -- success, failure, error
    error_message TEXT,  -- If status != success

    -- Indexes for common queries
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Index for querying by customer
CREATE INDEX idx_general_audit_log_customer_id ON audit_log(customer_id);

-- Index for querying by action type
CREATE INDEX idx_general_audit_log_action ON audit_log(action);

-- Index for time-based queries (retention policies, reports)
CREATE INDEX idx_general_audit_log_timestamp ON audit_log(timestamp DESC);

-- Index for resource lookups
CREATE INDEX idx_general_audit_log_resource ON audit_log(resource_type, resource_id);

-- GIN index for JSONB details (allows fast queries on arbitrary fields)
CREATE INDEX idx_general_audit_log_details ON audit_log USING GIN (details);

COMMENT ON TABLE audit_log IS 'Immutable audit trail for compliance and security analysis';
COMMENT ON COLUMN audit_log.action IS 'Dot-separated action name (e.g., user.register, scan.initiate, exposure.authorize_removal)';
COMMENT ON COLUMN audit_log.details IS 'JSON metadata specific to the action (before/after values, scan results, etc.)';
COMMENT ON COLUMN audit_log.status IS 'Outcome: success, failure, or error';
