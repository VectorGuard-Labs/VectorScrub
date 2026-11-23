-- Tamper evidence for the audit trail.
--
-- The append-only triggers stop the application and ordinary SQL from changing audit rows. They do
-- not stop the table OWNER (or a superuser) from dropping the triggers, editing a row and putting
-- them back. This adds a hash chain over the fields that identify what happened and never change:
-- each new audit row appends a link whose hash covers the previous link's hash and those fields.
-- Editing or removing an audit row, or a link, breaks every hash after it, and `vectorscrub audit
-- verify` finds the first break.
--
-- What it does NOT do: an attacker who is the owner can rewrite the whole chain consistently. The
-- defence for that is to store the head hash (`vectorscrub audit head`) somewhere the database
-- host cannot write, on a schedule. See docs/runbooks/audit-chain.md.
--
-- The hashed fields hold no personal data and are never touched by account erasure (which clears
-- customer_id, email, IP, user agent and free text, none of which are hashed), so erasing a
-- customer does not break the chain.

CREATE TABLE audit_chain (
    seq        BIGSERIAL PRIMARY KEY,
    source     TEXT  NOT NULL CHECK (source IN ('audit_log', 'removal_audit_log')),
    row_id     UUID  NOT NULL,
    link_hash  BYTEA NOT NULL,   -- sha256(previous link_hash || fields)
    UNIQUE (source, row_id)
);

-- The text that is hashed for one audit row. One definition, used by the trigger and by the verifier.
CREATE FUNCTION audit_chain_fields(src TEXT, rid UUID) RETURNS TEXT AS $$
    SELECT CASE src
        WHEN 'audit_log' THEN (
            SELECT concat_ws('|', 'audit_log', a.id, a.action, a.resource_type, coalesce(a.resource_id::text, ''),
                             a.status, to_char(a.created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS.US'))
              FROM audit_log a WHERE a.id = rid)
        WHEN 'removal_audit_log' THEN (
            SELECT concat_ws('|', 'removal_audit_log', r.id, r.action, r.actor,
                             to_char(r.recorded_at AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS.US'))
              FROM removal_audit_log r WHERE r.id = rid)
    END
$$ LANGUAGE sql STABLE;

CREATE FUNCTION audit_chain_append() RETURNS trigger AS $$
DECLARE
    prev BYTEA;
    fields TEXT;
BEGIN
    -- One writer at a time, so the chain has a single order. Held to the end of the transaction.
    PERFORM pg_advisory_xact_lock(7240001);
    SELECT link_hash INTO prev FROM audit_chain ORDER BY seq DESC LIMIT 1;
    IF prev IS NULL THEN
        prev := '\x'::bytea;
    END IF;
    IF TG_TABLE_NAME = 'audit_log' THEN
        fields := concat_ws('|', 'audit_log', NEW.id, NEW.action, NEW.resource_type, coalesce(NEW.resource_id::text, ''),
                            NEW.status, to_char(NEW.created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS.US'));
    ELSE
        fields := concat_ws('|', 'removal_audit_log', NEW.id, NEW.action, NEW.actor,
                            to_char(NEW.recorded_at AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS.US'));
    END IF;
    INSERT INTO audit_chain (source, row_id, link_hash)
    VALUES (TG_TABLE_NAME, NEW.id, sha256(prev || convert_to(fields, 'UTF8')));
    RETURN NULL;
END;
$$ LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp;

CREATE TRIGGER audit_log_chain AFTER INSERT ON audit_log
    FOR EACH ROW EXECUTE FUNCTION audit_chain_append();
CREATE TRIGGER removal_audit_log_chain AFTER INSERT ON removal_audit_log
    FOR EACH ROW EXECUTE FUNCTION audit_chain_append();

-- Only the trigger (running as the owner, SECURITY DEFINER) writes links. Whatever the default
-- privileges granted to other roles, they may read the chain and nothing else, so the runtime role
-- cannot forge a link or move the sequence.
DO $$
DECLARE g TEXT;
BEGIN
    FOR g IN SELECT DISTINCT grantee FROM information_schema.role_table_grants
              WHERE table_schema = 'public' AND table_name = 'audit_chain'
                AND grantee <> current_user AND grantee <> 'PUBLIC'
    LOOP
        EXECUTE format('REVOKE ALL ON audit_chain FROM %I', g);
        EXECUTE format('GRANT SELECT ON audit_chain TO %I', g);
        EXECUTE format('REVOKE ALL ON SEQUENCE audit_chain_seq_seq FROM %I', g);
    END LOOP;
END $$;

-- The chain itself is append-only, like the tables it covers.
CREATE FUNCTION audit_chain_guard() RETURNS trigger AS $$
BEGIN
    RAISE EXCEPTION 'audit_chain is append-only';
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER audit_chain_no_update_delete BEFORE UPDATE OR DELETE ON audit_chain
    FOR EACH ROW EXECUTE FUNCTION audit_chain_guard();
CREATE TRIGGER audit_chain_no_truncate BEFORE TRUNCATE ON audit_chain
    FOR EACH STATEMENT EXECUTE FUNCTION audit_chain_guard();

-- Rows written before this migration are not covered: the chain starts at the first row after it.
-- Only rows written from here on are chained, and `audit verify` says where the chain starts.
