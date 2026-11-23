# Runbook: the two database roles

## Why
The audit tables (`audit_log`, `removal_audit_log`) are append-only because triggers reject UPDATE, DELETE and
TRUNCATE. Whoever **owns** a table (or is a superuser) can drop those triggers. If the service connects as the
owner, any bug or break-in that reaches arbitrary SQL can switch the audit trail off. So there are two roles:

| Role | Used by | Can do |
|---|---|---|
| **owner** (`vectorscrub`, the `POSTGRES_USER`) | the `migrate` compose service, `vectorscrub migrate`, backups, operators | everything: owns the schema |
| **runtime** (`vectorscrub_app`) | the backend, and the operator CLI commands run inside the backend container | SELECT, INSERT, UPDATE, DELETE on rows and USAGE on sequences. Owns nothing, cannot create objects, no TRIGGER, TRUNCATE or REFERENCES |

The audit triggers still allow the one pseudonymizing update that account erasure needs; the runtime role can do
exactly that and nothing else to those tables. The owner's password is never given to the backend container.

## What enforces it
- On start the backend checks its own role (`services/db_roles.rs`). In **production** it refuses to start if the role
  is a superuser, bypasses row-level security, owns (or belongs to the owner of) an audit table, or holds TRIGGER or
  TRUNCATE on one. Outside production it logs a warning. `DATABASE_ALLOW_OWNER_ROLE=true` overrides the refusal; that is
  a decision to record.
- With `RUN_MIGRATIONS=false` (what compose sets) the backend applies no migrations. It reads `_sqlx_migrations`
  and refuses to start if any embedded migration is missing or its checksum differs. Migrations are applied by
  `vectorscrub migrate`, which uses `DATABASE_URL` as the **owner**.

## docker compose (the supported deployment)
- A new data volume: nothing to do. Postgres runs `infra/scripts/db_roles.sh` once while initialising the volume, using
  `DB_APP_PASSWORD`. The `migrate` service runs on every `docker compose up`, then the backend starts.
- An **existing** volume (created before this change), once, with the stack up:
  ```bash
  cd infra/docker
  export DB_APP_PASSWORD=$(openssl rand -hex 24)        # keep it: it goes into .env
  docker compose exec -T -e DB_APP_PASSWORD postgres sh -s < ../scripts/db_roles.sh
  echo "DB_APP_PASSWORD=$DB_APP_PASSWORD" >> .env       # mode 600
  docker compose up -d --build                          # migrate, then the backend as the runtime role
  ```
  (`db_roles.sh` finds the database through `POSTGRES_USER` and `POSTGRES_DB` inside the container.)

## Anywhere else (managed Postgres, a VM)
```bash
DATABASE_URL='postgres://<owner>:...@<host>/vectorscrub?sslmode=require' DB_APP_PASSWORD="$(openssl rand -hex 24)" infra/scripts/db_roles.sh
# one-off and on every deploy, as the owner:
DATABASE_URL='postgres://<owner>:...' vectorscrub migrate
# the service:
DATABASE_URL='postgres://vectorscrub_app:...@<host>/vectorscrub?sslmode=require'  RUN_MIGRATIONS=false
```
The owner role must not be a role the service can reach. Keep its URL in the deploy pipeline or with the operators only.
On a managed database the owner is not a superuser, which is fine: nothing here needs one.

## After a restore
Restoring a dump recreates the tables owned by whoever restored them, and may drop the grants. Run `db_roles.sh` again as
the owner (it is idempotent), then start the service. `restore_verify.sh` restores into a scratch database and does not need it.

## Check it
```bash
psql "$RUNTIME_URL" -c 'DROP TRIGGER audit_log_no_update_delete ON audit_log'   # ERROR: must be owner of relation audit_log
psql "$RUNTIME_URL" -c 'TRUNCATE audit_log'                                      # ERROR: permission denied
psql "$RUNTIME_URL" -c 'CREATE TABLE x(y int)'                                   # ERROR: permission denied for schema public
```
Those three are also what the backend's own test suite does with a role like this one (`services/db_roles.rs`).

## Limits
- The owner (and any Postgres superuser) can still bypass the audit triggers. They are detectable afterwards through the hash chain (`audit-chain.md`), but only if heads are stored off the host; there is no external log sink.
- The runtime role can still read and change every row it is granted, so a compromised backend can read ciphertext and
  change data. It cannot read plaintext PII without the key service, and it cannot erase the audit trail.
- `_sqlx_migrations` becomes read-only for the runtime role only when `db_roles.sh` runs after that table exists (a re-run after the first `migrate`; the init hook runs before it does). Until then the runtime role can edit it, which affects only the startup check.
