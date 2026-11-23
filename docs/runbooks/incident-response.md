# Runbook: security incident and outage response

Technical steps only. Who must be told, and by when (customers, regulators, card networks), depends on where the customers
live and what was exposed. That is a decision for counsel and is not written here; decide who you call **before** you need
them, and put their number in the contact table below.

## Contacts (fill in; nothing here is a placeholder to leave empty)
| Role | Name | How to reach |
|---|---|---|
| Incident lead (decides, writes the log) | | |
| Second person with access to the secrets manager and the keystore | | |
| Counsel for breach-notification questions | | |
| Hosting provider support | | |

## Severity
- **Critical**: customer identity data may have been read or changed by someone else; the key service or the keystore is lost or exposed; the audit trail was altered.
- **High**: the service is down, or sign-in or removal submissions are failing for everyone.
- **Medium**: one broker or one customer is affected; the rate limiter or Redis is degraded.

## First 15 minutes (any severity)
1. Open an incident log (a document with timestamps). Write what you saw and when. Keep writing until it is over.
2. Do **not** delete anything: no log, volume, container or container image. Copy first.
3. `docker compose ps`, `curl -s localhost:8080/api/v1/health/ready`, and the alerts that fired (`infra/monitoring/alerts.yml` names a runbook for each).
4. Decide the severity above. For Critical, go to the next section now.

## Suspected exposure of customer data (Critical)
Contain first, understand second.
1. **Cut the way in without destroying evidence.** Stop the public path: `systemctl stop nginx` (or block 443 at the firewall). The stack keeps running for forensics.
2. **Rotate what the attacker may hold**, in this order, writing each step in the log:
   - `AGENT_API_KEY` (backend and agents): ends access to the identity-release endpoints.
   - `JWT_SECRET`: signs every customer session out. `docker compose exec backend ./vectorscrub` has no command for this; change the value and restart the backend.
   - `DB_APP_PASSWORD` and the owner's `POSTGRES_PASSWORD` (`ALTER ROLE ... PASSWORD`, then `.env`, then restart).
   - `REDIS_PASSWORD`, `METRICS_TOKEN`, `STRIPE_WEBHOOK_SECRET` (rotating the last one invalidates open checkout links).
   - `PYHSM_IPC_SECRET`: restart PyHSM and the backend together. Changing the IPC secret stops a caller who only had that secret.
   - If the **host** was taken, assume the keystore file and its password were read. `kms rotate` does **not** repair that: old KEK versions stay in the keystore so old backups open, and the customers' data keys are re-wrapped, not replaced, so an attacker holding the old keystore, its password and a database copy can still unwrap them. There is no command that issues new data keys and re-encrypts the data. Treat the data as exposed, tell counsel, and still rotate the KEK and the keystore password (`key-service.md`, "Everyday operations") to protect what is written from now on. Take a keystore backup straight afterwards.
3. **Work out what was read.** The audit trail is the record:
   ```sql
   -- identity releases to the agent layer, most recent first
   SELECT timestamp, action, resource_type, resource_id, status, ip_address FROM audit_log
    WHERE action = 'identity.release' ORDER BY timestamp DESC LIMIT 200;
   ```
   Releases only happen against an actionable work item, one at a time, and each is logged. A bulk read of identities would show as a burst, or as **no** entries if the database or keystore was read directly. Also check `audit_log` for `user.login` failures from one address, for `session.*` and `mfa.*` events, and for gaps (the `AuditWritesFailing` alert).
4. **Preserve**: `docker compose logs --no-color > incident-$(date +%F).log`, `docker inspect` of each container, a database dump taken now (`infra/scripts/backup.sh`), the nginx access log.
5. **Tell counsel** with the incident log. They decide on notifications. Do not guess the deadlines.

## Lost or damaged keystore (Critical)
Stop. Do **not** run `kms init`: it creates a new key and orphans every stored data key. Restore the keystore from its backup (`key-service.md`, "Restoring for real"), run `vectorscrub verify-backup`, and only then start the backend. If no backup exists, customer data is unrecoverable; say so to counsel before saying anything to customers.

## Audit trail altered or the runtime role misused (Critical)
The audit triggers and the two-role arrangement (`database-roles.md`) mean this needs the **owner** role or a superuser. Rotate the owner password, list who holds it, restore the triggers from migrations 0007, 0010 and 0017, and compare the table with the last backup (`pg_restore` into a scratch database).

## Outage (High)
| Symptom | Look at |
|---|---|
| `/health/ready` is 503 with `key_service: false` | PyHSM container (`docker compose logs pyhsm`). The backend recovers by itself when it answers again. Customers see errors until then. |
| `/health/ready` is 503, database false | Postgres container and disk space (`df -h`). Do not remove volumes. |
| Redis down | The API stays up and the rate limiter falls back to per-instance limits (`RateLimiterRunningOnFallback`). Scans stop being dispatched; the recovery loop re-queues them afterwards. |
| Only removals fail | `docs/runbooks/captcha-detection.md` for a CAPTCHA; the agents' logs for `web_form_page_refused`. |
| After a deploy | Roll back by checking out the previous commit and `docker compose up -d --build`. Migrations only move forward: if one must be undone, restore from backup (`backup-and-restore.md`), then run `db_roles.sh`. |

## After it is over
- Write what happened, what was affected, what you changed, what you will change. Keep it with the incident log.
- Add the missing alert or check that would have caught it sooner.
- If a secret was exposed, confirm the old value no longer works.
