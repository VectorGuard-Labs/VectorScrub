#!/bin/sh
# Create (or repair) the database role the running service connects as.
#
# Why: the audit tables are protected by triggers that reject UPDATE, DELETE and TRUNCATE, but
# whoever OWNS a table can drop them. If the service connects as the owner, any bug that reaches
# arbitrary SQL can switch the audit trail off. So the service gets a second role that can read
# and write rows and nothing more: it owns nothing, cannot create objects, and has no TRIGGER,
# TRUNCATE or REFERENCES privilege. The owner role is used only to run migrations
# (`vectorscrub migrate`) and by operators.
#
#   DATABASE_URL=postgres://owner:...@host/vectorscrub DB_APP_PASSWORD=... infra/scripts/db_roles.sh
#
# It is safe to run again at any time (after a restore, or after migrations added tables: new
# tables are also covered by default privileges, so this is a repair, not a routine step).
#
# Settings (environment):
#   DATABASE_URL        libpq URL of the database, as the OWNER role (the one that runs migrations).
#                       Under the postgres image's init hook it is not needed: POSTGRES_USER and
#                       POSTGRES_DB are used over the local socket instead.
#   DB_APP_PASSWORD     password for the runtime role (24+ characters). Read from the environment so
#                       it never appears in a process list. For use in a URL, make it URL-safe:
#                       openssl rand -hex 24
#   DB_APP_ROLE         name of the runtime role (default vectorscrub_app; letters, digits, _).
#
# Then point the service at the new role: DATABASE_URL=postgres://vectorscrub_app:...@host/db,
# RUN_MIGRATIONS=false. The service refuses to start in production if its role is too powerful.
set -eu

die() { echo "db_roles: $*" >&2; exit 1; }

APP_ROLE="${DB_APP_ROLE:-vectorscrub_app}"
case "$APP_ROLE" in
  ""|[0-9]*|*[!a-z0-9_]*) die "DB_APP_ROLE must be lowercase letters, digits and _ (not starting with a digit)" ;;
esac
[ "${#APP_ROLE}" -le 63 ] || die "DB_APP_ROLE is too long"
[ -n "${DB_APP_PASSWORD:-}" ] || die "DB_APP_PASSWORD is not set"
[ "${#DB_APP_PASSWORD}" -ge 24 ] || die "DB_APP_PASSWORD must be at least 24 characters (openssl rand -hex 24)"

command -v psql >/dev/null 2>&1 || die "psql is not installed"
# Run under the postgres image's init hook, or by hand against DATABASE_URL.
if [ -n "${DATABASE_URL:-}" ]; then
  target="$DATABASE_URL"
elif [ -n "${POSTGRES_USER:-}" ]; then
  target="dbname=${POSTGRES_DB:-$POSTGRES_USER} user=$POSTGRES_USER"
else
  die "set DATABASE_URL (the owner role's URL)"
fi

export DB_APP_PASSWORD
psql "$target" -X -q -v ON_ERROR_STOP=1 -v app_role="$APP_ROLE" <<'SQL'
\getenv app_pw DB_APP_PASSWORD

-- The role: can log in, nothing else.
SELECT format('CREATE ROLE %I LOGIN PASSWORD %L', :'app_role', :'app_pw')
 WHERE NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = :'app_role') \gexec
SELECT format('ALTER ROLE %I LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS PASSWORD %L', :'app_role', :'app_pw') \gexec

-- It must not be (or become) a member of the owner role.
SELECT format('REVOKE %I FROM %I', current_user, :'app_role')
 WHERE pg_has_role(:'app_role', current_user, 'MEMBER') \gexec

SELECT format('GRANT CONNECT ON DATABASE %I TO %I', current_database(), :'app_role') \gexec
SELECT format('GRANT USAGE ON SCHEMA public TO %I', :'app_role') \gexec
SELECT format('REVOKE CREATE ON SCHEMA public FROM %I', :'app_role') \gexec

-- Rows only. No TRUNCATE, TRIGGER or REFERENCES, which is what would let it take the audit
-- trail apart (and TRIGGER on a table is enough to drop its triggers' effect).
SELECT format('GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO %I', :'app_role') \gexec
SELECT format('REVOKE TRUNCATE, TRIGGER, REFERENCES ON ALL TABLES IN SCHEMA public FROM %I', :'app_role') \gexec
SELECT format('GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA public TO %I', :'app_role') \gexec

-- Tables and sequences the owner creates later (migrations) get the same, automatically.
SELECT format('ALTER DEFAULT PRIVILEGES FOR ROLE %I IN SCHEMA public GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO %I', current_user, :'app_role') \gexec
SELECT format('ALTER DEFAULT PRIVILEGES FOR ROLE %I IN SCHEMA public GRANT USAGE, SELECT ON SEQUENCES TO %I', current_user, :'app_role') \gexec

-- The service only reads which migrations were applied.
SELECT format('REVOKE INSERT, UPDATE, DELETE ON _sqlx_migrations FROM %I', :'app_role')
 WHERE to_regclass('public._sqlx_migrations') IS NOT NULL \gexec

-- The audit hash chain is written only by its trigger (SECURITY DEFINER, run as the owner). The
-- runtime role may read it and nothing else, whatever the grants above said.
SELECT format('REVOKE INSERT, UPDATE, DELETE ON audit_chain FROM %I', :'app_role')
 WHERE to_regclass('public.audit_chain') IS NOT NULL \gexec
SELECT format('REVOKE ALL ON SEQUENCE audit_chain_seq_seq FROM %I', :'app_role')
 WHERE to_regclass('public.audit_chain_seq_seq') IS NOT NULL \gexec
SQL

echo "db_roles: role ${APP_ROLE} is ready (rows only: no ownership, no TRIGGER/TRUNCATE, cannot create objects)"
