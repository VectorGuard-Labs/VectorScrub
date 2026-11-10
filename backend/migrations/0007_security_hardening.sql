-- Migration 0007: security hardening
--
-- 1. login_throttle: failed-login counting and lockout.
--    Keyed by sha256(lower(email)) so (a) no raw email is stored here and (b) an
--    email that has no account is throttled exactly like one that does, which
--    keeps lockout from revealing which emails are registered.
--
-- 2. audit_log immutability. The documentation and the project rules say the audit
--    log is append-only, but only removal_audit_log was protected (migration 0001).
--    The general audit_log (migration 0004) was an ordinary mutable table. These
--    triggers make it append-only and, unlike a RULE ... DO INSTEAD NOTHING, fail
--    loudly instead of silently ignoring a write.
--
--    The only UPDATE allowed is pseudonymization: setting customer_id and/or
--    user_email to NULL (what ON DELETE SET NULL does when a customer is erased).
--    Any other change, any DELETE, and any TRUNCATE is rejected.
--
--    Scope of the guarantee: this protects against the application role and
--    ordinary SQL. A database superuser or the table owner can still drop the
--    trigger; real tamper evidence against them needs an external WORM sink or a
--    hash chain, which this migration does not provide.

CREATE TABLE login_throttle (
    key_hash        TEXT PRIMARY KEY,
    failures        INT NOT NULL DEFAULT 0,
    locked_until    TIMESTAMPTZ,
    last_failure_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_login_throttle_last_failure ON login_throttle(last_failure_at);

CREATE FUNCTION audit_log_guard() RETURNS trigger AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'audit_log is append-only: DELETE is not permitted';
    ELSIF TG_OP = 'TRUNCATE' THEN
        RAISE EXCEPTION 'audit_log is append-only: TRUNCATE is not permitted';
    ELSIF TG_OP = 'UPDATE' THEN
        -- Pseudonymization only: customer_id / user_email may become NULL, nothing else may change.
        IF NEW.customer_id IS NOT NULL AND NEW.customer_id IS DISTINCT FROM OLD.customer_id THEN
            RAISE EXCEPTION 'audit_log is append-only: customer_id may only be cleared';
        END IF;
        IF NEW.user_email IS NOT NULL AND NEW.user_email IS DISTINCT FROM OLD.user_email THEN
            RAISE EXCEPTION 'audit_log is append-only: user_email may only be cleared';
        END IF;
        IF (NEW.id, NEW.timestamp, NEW.action, NEW.resource_type, NEW.resource_id,
            NEW.ip_address, NEW.user_agent, NEW.details, NEW.status, NEW.error_message, NEW.created_at)
           IS DISTINCT FROM
           (OLD.id, OLD.timestamp, OLD.action, OLD.resource_type, OLD.resource_id,
            OLD.ip_address, OLD.user_agent, OLD.details, OLD.status, OLD.error_message, OLD.created_at) THEN
            RAISE EXCEPTION 'audit_log is append-only: records cannot be modified';
        END IF;
        RETURN NEW;
    END IF;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER audit_log_no_update_delete
    BEFORE UPDATE OR DELETE ON audit_log
    FOR EACH ROW EXECUTE FUNCTION audit_log_guard();

CREATE TRIGGER audit_log_no_truncate
    BEFORE TRUNCATE ON audit_log
    FOR EACH STATEMENT EXECUTE FUNCTION audit_log_guard();
