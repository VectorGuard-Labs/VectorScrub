-- Migration 0017: let an account be erased without rewriting history.
--
-- Deleting a customer must remove their personal data, but both audit tables are append-only.
-- This migration lets them be PSEUDONYMIZED (never edited otherwise), the way audit_log already
-- allowed for customer_id and user_email:
--
--   removal_audit_log   customer_id and removal_request_id may become NULL, and details may be
--                       reset to '{}' (free text such as an error message could name the
--                       person) once customer_id is NULL. Action, actor and time stay.
--   audit_log           additionally ip_address and user_agent may be cleared.
--
-- Everything else is still rejected: any other UPDATE, any DELETE, any TRUNCATE.

ALTER TABLE removal_audit_log
    ALTER COLUMN customer_id DROP NOT NULL,
    ALTER COLUMN removal_request_id DROP NOT NULL,
    DROP CONSTRAINT removal_audit_log_customer_id_fkey,
    DROP CONSTRAINT removal_audit_log_removal_request_id_fkey,
    ADD CONSTRAINT removal_audit_log_customer_id_fkey
        FOREIGN KEY (customer_id) REFERENCES customers(id) ON DELETE SET NULL,
    ADD CONSTRAINT removal_audit_log_removal_request_id_fkey
        FOREIGN KEY (removal_request_id) REFERENCES removal_requests(id) ON DELETE SET NULL;

CREATE OR REPLACE FUNCTION removal_audit_log_guard() RETURNS trigger AS $$
BEGIN
    IF TG_OP = 'UPDATE' THEN
        IF (NEW.id, NEW.action, NEW.actor, NEW.recorded_at)
           IS NOT DISTINCT FROM (OLD.id, OLD.action, OLD.actor, OLD.recorded_at)
           AND (NEW.customer_id IS NULL OR NEW.customer_id IS NOT DISTINCT FROM OLD.customer_id)
           AND (NEW.removal_request_id IS NULL OR NEW.removal_request_id IS NOT DISTINCT FROM OLD.removal_request_id)
           AND (NEW.details IS NOT DISTINCT FROM OLD.details
                OR (NEW.details = '{}'::jsonb AND NEW.customer_id IS NULL))
        THEN
            RETURN NEW;
        END IF;
    END IF;
    RAISE EXCEPTION 'removal_audit_log is append-only: % is not permitted', TG_OP;
END;
$$ LANGUAGE plpgsql;

CREATE OR REPLACE FUNCTION audit_log_guard() RETURNS trigger AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'audit_log is append-only: DELETE is not permitted';
    ELSIF TG_OP = 'TRUNCATE' THEN
        RAISE EXCEPTION 'audit_log is append-only: TRUNCATE is not permitted';
    ELSIF TG_OP = 'UPDATE' THEN
        -- Pseudonymization only: these four columns may become NULL, nothing else may change.
        IF NEW.customer_id IS NOT NULL AND NEW.customer_id IS DISTINCT FROM OLD.customer_id THEN
            RAISE EXCEPTION 'audit_log is append-only: customer_id may only be cleared';
        END IF;
        IF NEW.user_email IS NOT NULL AND NEW.user_email IS DISTINCT FROM OLD.user_email THEN
            RAISE EXCEPTION 'audit_log is append-only: user_email may only be cleared';
        END IF;
        IF NEW.ip_address IS NOT NULL AND NEW.ip_address IS DISTINCT FROM OLD.ip_address THEN
            RAISE EXCEPTION 'audit_log is append-only: ip_address may only be cleared';
        END IF;
        IF NEW.user_agent IS NOT NULL AND NEW.user_agent IS DISTINCT FROM OLD.user_agent THEN
            RAISE EXCEPTION 'audit_log is append-only: user_agent may only be cleared';
        END IF;
        IF (NEW.id, NEW.timestamp, NEW.action, NEW.resource_type, NEW.resource_id,
            NEW.details, NEW.status, NEW.error_message, NEW.created_at)
           IS DISTINCT FROM
           (OLD.id, OLD.timestamp, OLD.action, OLD.resource_type, OLD.resource_id,
            OLD.details, OLD.status, OLD.error_message, OLD.created_at) THEN
            RAISE EXCEPTION 'audit_log is append-only: records cannot be modified';
        END IF;
        RETURN NEW;
    END IF;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;
