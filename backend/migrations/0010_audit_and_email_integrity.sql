-- Migration 0010: audit-trail guard for removal_audit_log, and case-insensitive account emails
--
-- 1. removal_audit_log is the table that records customer authorization and every step of a
--    removal. Since migration 0001 it was "protected" by two rules,
--        CREATE RULE ... ON UPDATE/DELETE ... DO INSTEAD NOTHING
--    which silently swallow a write: a bug or an attacker rewriting history gets "success" and
--    nothing happens, and nobody finds out. TRUNCATE was not covered at all. audit_log has had
--    proper triggers since migration 0007; this gives removal_audit_log the same: any UPDATE,
--    DELETE or TRUNCATE is refused with an error, which is loud and shows up in logs.
--
--    There is no pseudonymization exception here (unlike audit_log): customer_id is NOT NULL
--    and the foreign keys have no ON DELETE action, so a customer who has authorized a removal
--    cannot be hard-deleted while their removal history exists. That is unchanged from before
--    (the rules never affected foreign keys) and means a future erasure feature has to decide
--    what to do with this table deliberately.
--
--    Scope: this stops the application role and ordinary SQL. A superuser or the table owner
--    can drop the trigger; protection against them needs an external write-once sink or a
--    hash chain, which this migration does not provide.
--
-- 2. Account emails were unique case-SENSITIVELY, so Jane@x.com and jane@x.com could be two
--    accounts, while the login lockout treated them as one. Lookups now compare lower(email),
--    and this unique index makes that the rule the database itself enforces. If two existing
--    accounts differ only by case this CREATE UNIQUE INDEX fails and the migration stops:
--    they must be merged or one renamed by a person (the application cannot know which is
--    the real owner).

DROP RULE IF EXISTS no_update_audit ON removal_audit_log;
DROP RULE IF EXISTS no_delete_audit ON removal_audit_log;

CREATE FUNCTION removal_audit_log_guard() RETURNS trigger AS $$
BEGIN
    RAISE EXCEPTION 'removal_audit_log is append-only: % is not permitted', TG_OP;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER removal_audit_log_no_update_delete
    BEFORE UPDATE OR DELETE ON removal_audit_log
    FOR EACH ROW EXECUTE FUNCTION removal_audit_log_guard();

CREATE TRIGGER removal_audit_log_no_truncate
    BEFORE TRUNCATE ON removal_audit_log
    FOR EACH STATEMENT EXECUTE FUNCTION removal_audit_log_guard();

CREATE UNIQUE INDEX customers_email_lower_key ON customers (lower(email));
