# Backup and restore

Status: the scripts and the verification command exist and were exercised end to end on a development machine (see the drill log). **No production backup schedule, off-site copy or restore drill on real infrastructure has been set up or evidenced.** Until the checklist at the bottom is complete, treat the service as having no backups.

## What a backup is, and the one thing that can make it worthless

The database holds everything. Personal data in it (identity profiles, listing links, two-factor secrets) is AES-256-GCM ciphertext under per-customer data keys, and those keys are stored wrapped by a key that lives only in the **PyHSM keystore**. The keystore is **not** in the database and **not** in the database backup.

- A database backup without the PyHSM keystore restores to a database full of data nobody can open.
- Losing the keystore (or its password) loses every customer's data, whatever the state of the database backups. **Back the keystore up separately and on its own schedule: `infra/scripts/backup_keystore.sh`, and see `docs/runbooks/key-service.md`.**
- Rotating the key adds a version to the keystore and keeps the old ones, so an older database backup still opens with a newer keystore. The reverse is not true: a database made after a rotation needs a keystore backup taken after it.

So: keep the keystore backups, the keystore password and the database backups in different places with different access. Anyone who can reach all three can read everything.

The dump also holds, in the clear: account emails, password hashes, client IP addresses, user agents, and the audit trail. That is why `backup.sh` refuses to write an unencrypted backup unless told to, and encrypts to a public key whose private half is **not** on the database host.

## Taking a backup

```bash
DATABASE_URL=postgres://backup_user:...@db-host:5432/vectorscrub \
BACKUP_GPG_RECIPIENT=backups@your-domain.example \
BACKUP_DIR=/var/backups/vectorscrub \
BACKUP_S3_URI=s3://your-bucket/vectorscrub \
BACKUP_HEARTBEAT_URL=https://hc-ping.com/your-uuid \
infra/scripts/backup.sh
```

`infra/scripts/backup.sh` dumps (custom format), confirms the dump can be read back and contains the tables that matter, encrypts it, writes a SHA-256 file, copies it off the machine, deletes local copies older than `BACKUP_KEEP_DAYS` (default 30, minimum 7), and pings a heartbeat URL only after everything else worked. Read the header of the script for every setting.

Run it from cron or a scheduler at least daily:

```
17 2 * * *  /opt/vectorscrub/infra/scripts/backup.sh >> /var/log/vectorscrub-backup.log 2>&1
```

Use a database role that can only read for `DATABASE_URL`. Generate the GPG key pair on a different machine, put only the public key on the database host, and store the private key where the master key's backup is kept (offline copy, two people able to reach it).

**Heartbeat.** Silent failure is the usual way backups stop. Point `BACKUP_HEARTBEAT_URL` at a monitor that alerts when the ping does not arrive (healthchecks.io or similar). Without it, nobody learns that the cron job broke.

**Managed Postgres.** If the database is AWS RDS or similar, turn on its automated backups with point-in-time recovery as well (`--backup-retention-period 7` or more). That gives a much smaller data-loss window than a daily dump. Keep the script too: it is the copy you control, off the provider, and the one `restore_verify.sh` can test.

## Proving a backup works (the drill)

```bash
RESTORE_ADMIN_URL=postgres://postgres:...@scratch-host:5432/postgres \
KEY_PROVIDER=pyhsm PYHSM_SOCKET=/path/to/restored-keystore-pyhsm.sock \
PYHSM_IPC_SECRET="$(fetch-from-secrets-manager)" \
GNUPGHOME=/path/to/keyring-with-the-private-key \
infra/scripts/restore_verify.sh /var/backups/vectorscrub/vectorscrub-20261109T021700Z.dump.gpg
```

It checks the file against its checksum, decrypts it, restores it into a new database it creates and later drops, then runs `vectorscrub verify-backup` against that copy. That command (`backend/src/services/backup_check.rs`) fails the drill when any of these is wrong:

| Check | What it catches |
|-------|-----------------|
| Migration history | Backup from an older or newer version than the build you are restoring with, or SQL that differs from what was applied |
| Audit-log guards | Triggers missing after restore (the append-only guarantee silently lost). It also tries to delete one audit row inside a transaction it always rolls back |
| Trial decryption | A sample of identity profiles, two-factor secrets and listing links must open with the key given. This is the check that proves **this backup and this key belong together** |
| Nothing to test the key on | Customers exist but no encrypted row was found: reported as "not proven", never as usable |

Output is counts and problem descriptions only. It prints no customer data. It exits non-zero when the backup is not usable. `RESTORE_SKIP_DECRYPT_CHECK=1` skips the key check and the script then says plainly that the result is "restores", not "usable".

Run the drill:

- after setting up backups, before launch;
- monthly after that, on a different day each time, using the newest off-site copy and the key as the secrets manager holds it (not a copy on your laptop);
- after any change to the backup scripts, the database version, or key handling.

Record each drill in the log below. The restore time it prints is your real recovery time for the database step.

## Restoring for real

1. **Stop the writers.** Stop the backend and the agents so nothing writes to the database while you restore.
2. **Decide the target.** A new database (preferred: keep the damaged one for investigation) or the existing one (`pg_restore --clean --if-exists`).
3. **Restore.** `gpg --decrypt FILE | pg_restore --no-owner --no-privileges --exit-on-error --dbname=NEW_DB_URL` (or use the managed provider's point-in-time restore).
4. **Verify before starting anything:** `DATABASE_URL=NEW_DB_URL KEY_PROVIDER=pyhsm PYHSM_SOCKET=... PYHSM_IPC_SECRET=... vectorscrub verify-backup`. Do not continue on "NOT usable".
5. **Start the backend.** It applies any migrations the backup is missing. If `verify-backup` said the backup is from a *newer* version, deploy that version instead.
6. **Redis is not restored.** It holds the scan queue and rate-limit counters only. The scan recovery loop re-dispatches scans that were queued or running, so nothing needs doing.
7. **Sessions.** Everyone is signed in or out according to the restored `sessions` table. If you restored to a point before an incident, revoke every session first: `UPDATE sessions SET revoked_at = NOW() WHERE revoked_at IS NULL;` (a deliberate operator action, not something the service does).
8. **Re-run erasures (see next section).**
9. **Check** `/api/v1/health/ready`, sign in with a test account, run one scan.

## Restores bring back erased accounts

Account erasure deletes the customer and pseudonymizes the audit trail. A backup taken before an erasure still contains that customer. After restoring, an erased person's data is back in the live database. The service does not keep a list of erased accounts (it deliberately stores no identifier of them), so it cannot redo them by itself.

Until that is solved, the procedure is: keep the operators' record of written erasure requests (the email thread or ticket, with date), and after any restore re-run `vectorscrub erase-account <email>` for each request dated after the backup. Backups are also kept for `BACKUP_KEEP_DAYS` (30 by default), so an erased customer's data persists in backups for up to that long. The Privacy Policy and `DATA_RETENTION.md` must say so. If a stricter promise is wanted, shorten the retention or add a signed-off erasure ledger that stores only a salted hash of each erased email; that is a design decision for counsel and is not built.

## Targets (not yet set)

The checklist asks for a recovery time objective and a recovery point objective. Neither is defined, because nothing has been measured on production-sized data.

- **Recovery point** is bounded by the backup interval: with one daily dump, up to a day of data (new accounts, scan results, removal progress, audit entries) can be lost. A removal authorization lost this way is a legal record lost. Point-in-time recovery on the managed database shrinks this to minutes; use it.
- **Recovery time** for the database step is the number the drill prints, on the size of the data at that moment. Add the time to provision, restore the key, deploy and verify. Write the total down after the first real-infrastructure drill and set the objective from it.

## Drill log

| Date | Environment | Backup | Restore time | Result | Notes |
|------|-------------|--------|--------------|--------|-------|
| 2026-11-17 | Developer machine, scratch Postgres 16, one test customer with identity and two-factor | encrypted dump, 14 KB | 1 s | usable with the right key; **not usable** with a wrong key (exit 1); corrupted file rejected by checksum | Proves the scripts and the checks work. Says nothing about production size, network, or the real key store. |

## Before you rely on this (all open)

- [ ] Backup user with read-only rights created
- [ ] GPG key pairs generated off the database host (one for database backups, a different one for keystore backups); private keys stored apart from each other
- [ ] Daily schedule installed; heartbeat monitor alerting
- [ ] Off-site copy in a bucket with versioning and a retention rule, in a different account or provider than the database
- [ ] Managed point-in-time recovery enabled (if on a managed database)
- [ ] First drill run against the off-site copy with the key from the secrets manager; logged above
- [ ] PyHSM keystore backed up on a schedule and after every rotation (`backup_keystore.sh`), its password and IPC secret in a secrets manager, and a second person able to reach them
- [ ] RTO and RPO written down from the drill
- [ ] Erasure record process agreed (previous section) and the Privacy Policy states the backup retention
