# Installing the backup schedule

These files turn the backup scripts into a running schedule with an off-site copy and alerting. They were checked (`systemd-analyze verify`, shell syntax, JSON policy documents) but **not run against your AWS account or your host**. Nothing here is live until you do the steps below.

| File | What it does |
|---|---|
| `aws-offsite-setup.sh` | Creates the off-site bucket (private, encrypted, versioned, Object Lock so backups cannot be deleted for 35 days, expiry after 400) and a write-only identity for the backup host |
| `systemd/vectorscrub-backup.{service,timer}` | Database backup, daily 02:17 UTC |
| `systemd/vectorscrub-keystore-backup.{service,timer}` | PyHSM keystore backup, daily 02:47 UTC, and by hand after every rotation |

## 1. Off-site bucket (once)

Use a **separate AWS account** for backups, so a compromise of production cannot delete them:

```bash
AWS_PROFILE=<backups-account> BUCKET=<globally-unique-name> REGION=<region> infra/backup/aws-offsite-setup.sh
```

It prints an access key once. That key can only *upload* to `db/` and `keystore/`: it cannot read, list or delete. Create a separate read-only key for restores and keep it offline. Object Lock is in compliance mode: **for 35 days not even the account's root user can delete a backup**, which is the point, and also means a backup you uploaded by mistake stays for 35 days.

## 2. Heartbeat monitors (once)

Create **two** checks in a dead-man's-switch service (healthchecks.io, Better Stack, Cronitor or similar), one per job, each expecting a ping every 24 hours with a few hours' grace, alerting a person by email and phone. The scripts ping only after a complete, verified backup, so a missing ping means the backup did not happen. The URLs are secrets (anyone with one can fake a success), so put them only in the env files below.

## 3. The backup host

```bash
sudo useradd --system --home /nonexistent --shell /usr/sbin/nologin vsbackup
sudo install -d -o vsbackup -g vsbackup -m 700 /var/backups/vectorscrub /var/backups/vectorscrub-keystore
sudo install -d -m 755 /etc/vectorscrub
sudo cp -r . /opt/vectorscrub            # or a checkout of this repository
```

Import the two **public** GPG keys for the `vsbackup` user (one for database backups, another for the keystore). Keep both private keys off this machine, in different places.

`/etc/vectorscrub/backup.env` (mode 600, owner vsbackup):
```
DATABASE_URL=postgres://backup_readonly:...@db-host:5432/vectorscrub?sslmode=require
BACKUP_GPG_RECIPIENT=db-backups@your-domain
BACKUP_DIR=/var/backups/vectorscrub
BACKUP_KEEP_DAYS=30
BACKUP_S3_URI=s3://<bucket>/db
AWS_ACCESS_KEY_ID=...
AWS_SECRET_ACCESS_KEY=...
AWS_DEFAULT_REGION=<region>
BACKUP_HEARTBEAT_URL=https://hc-ping.com/<uuid-1>
PG_BIN_DIR=/usr/lib/postgresql/16/bin
```

`/etc/vectorscrub/keystore-backup.env` (mode 600, owner vsbackup):
```
PYHSM_KEYSTORE=/srv/pyhsm/keystore.enc
BACKUP_GPG_RECIPIENT=keystore-backups@your-domain
KEYSTORE_BACKUP_DIR=/var/backups/vectorscrub-keystore
BACKUP_KEEP_DAYS=90
BACKUP_S3_URI=s3://<bucket>/keystore
AWS_ACCESS_KEY_ID=...
AWS_SECRET_ACCESS_KEY=...
AWS_DEFAULT_REGION=<region>
BACKUP_HEARTBEAT_URL=https://hc-ping.com/<uuid-2>
```

**The keystore must be readable by `vsbackup`.** In docker-compose the keystore sits in a Docker volume only root can read. Either mount it from a host directory (replace `pyhsm_data:/data` with `/srv/pyhsm:/data` in the compose file, `chown 1000:vsbackup /srv/pyhsm` and `chmod 750`), or add `vsbackup` to a group that can read it. The script never needs write access to it.

Create the database role the backups use, read-only:
```sql
CREATE ROLE backup_readonly LOGIN PASSWORD '...';
GRANT pg_read_all_data TO backup_readonly;
```

Install and start:
```bash
sudo cp infra/backup/systemd/*.service infra/backup/systemd/*.timer /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now vectorscrub-backup.timer vectorscrub-keystore-backup.timer
sudo systemctl start vectorscrub-backup vectorscrub-keystore-backup     # first run now, and watch it
journalctl -u vectorscrub-backup -u vectorscrub-keystore-backup -n 50
```

Both heartbeat checks should turn green, and both files should appear in the bucket.

## 4. The drill on real infrastructure

Do this before the first real customer, then monthly. It is the only thing that proves the setup works.
`infra/scripts/restore_drill.sh` does the whole of it in one command, on any machine with Docker:

```bash
# On a scratch machine, with the READ-ONLY restore key (never the backup host's write key):
export AWS_ACCESS_KEY_ID=... AWS_SECRET_ACCESS_KEY=... AWS_DEFAULT_REGION=<region>
export GNUPGHOME=/path/to/keyring-with-both-private-keys
DRILL_S3_DB_URI=s3://<bucket>/db DRILL_S3_KEYSTORE_URI=s3://<bucket>/keystore \
DRILL_PYHSM_PASSWORD_FILE=/run/secrets/pyhsm-password \
PYHSM_IMAGE=pyhsm@sha256:<the digest production runs> \
BACKEND_IMAGE=<the backend image production runs> \
DRILL_ENVIRONMENT="production backups, scratch VM" \
infra/scripts/restore_drill.sh
```

It fetches the newest database and keystore backups, checks both checksums, decrypts both, starts a
PyHSM container (no network) on the restored keystore with the password from the secrets manager,
restores the database into a scratch Postgres on an internal network, runs `vectorscrub verify-backup`
and `vectorscrub audit verify` against it, and appends a line to `docs/runbooks/drill-log.md` with the
timings. It must end with `RESULT: usable`. It removes every scratch container, volume and decrypted
file on exit, and never touches the live stack. A wrong keystore password or a damaged backup fails
loudly (tested). Then wipe the scratch machine.

## 5. The daily audit-head job

The audit hash chain only proves something against the database owner if its head is stored where the
database host cannot write (`docs/runbooks/audit-chain.md`). `infra/scripts/audit_head.sh` verifies the
chain, checks that yesterday's stored head is still in it, stores today's head in the bucket under
`audit-heads/` (the setup script lets the backup identity read and write only that prefix, besides
writing backups), and pings a heartbeat.

```bash
sudo useradd --system --home /nonexistent --shell /usr/sbin/nologin vsaudit
sudo install -d -o vsaudit -g vsaudit -m 700 /var/lib/vectorscrub/audit-heads
sudo cp infra/backup/systemd/vectorscrub-audit-head.{service,timer} /etc/systemd/system/
```

`/etc/vectorscrub/audit-head.env` (mode 600, owner vsaudit):
```
AUDIT_HEAD_DIR=/var/lib/vectorscrub/audit-heads
AUDIT_HEAD_S3_URI=s3://<bucket>/audit-heads
AWS_ACCESS_KEY_ID=...
AWS_SECRET_ACCESS_KEY=...
AWS_DEFAULT_REGION=<region>
AUDIT_HEAD_HEARTBEAT_URL=https://hc-ping.com/<uuid-3>
```
A third heartbeat check, like the two backup ones. Then `sudo systemctl enable --now vectorscrub-audit-head.timer`
and run it once by hand (`sudo systemctl start vectorscrub-audit-head`; `journalctl -u vectorscrub-audit-head`).
The job reaches the backend through `docker compose exec`, so `vsaudit` is in the `docker` group, which is
root-equivalent on that host: keep the account to this job. A failed run says `AUDIT CHAIN BROKEN` or why
it stopped, and skips the heartbeat, so the missing ping is what alerts you.

## After a KEK rotation

`vectorscrub kms rotate` adds a key version that only exists in the live keystore until the next keystore backup. Run `sudo systemctl start vectorscrub-keystore-backup` straight after, and check its heartbeat.
