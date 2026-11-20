# Key service (PyHSM): setup, rotation, recovery

**Read this before the first customer.** It explains what the key service protects, the one failure that cannot be undone, and every operation you will run.

## What it is

Every customer has a random data key. The database stores it only **wrapped** (encrypted) by one key-encryption key (KEK) that lives inside PyHSM. The backend asks PyHSM to wrap and unwrap and never holds the KEK. Inside the customer's fields, each kind of value gets its own key (HKDF), and each ciphertext is bound to its customer and field (AES-GCM associated data), so a value copied to another row or column fails to open.

```
customer data  <--(per-field key, HKDF)--  data key (DEK, random, per customer)
                                                 |
                                       wrapped by the KEK  -->  stored in Postgres (customer_keys)
                                                 |
                                       the KEK exists only in PyHSM's keystore
```

**What this protects against:** reading the database, the backups, or the backend's environment no longer yields anything that opens customer data. There is no master key to steal from a process.

**What it does not protect against** (be honest about this with anyone who asks):
- Someone who takes over the **running backend** can ask PyHSM to unwrap keys for as long as they hold it, and can read data keys already cached in memory (`DEK_CACHE_TTL_SECS`, 5 minutes by default). PyHSM logs every unwrap, so it is visible, and it ends when you revoke access. It limits the damage; it does not make a compromised backend harmless.
- Someone with **root on the host that runs PyHSM** can read the keystore, its password file and its memory. PyHSM is software, not a hardware security module: it has no FIPS 140 validation and no tamper-resistant hardware (its own README says so). If a regulator or customer requires certified hardware, this does not meet it, and the design here (a wrap/unwrap interface) is built so a cloud KMS or hardware HSM could replace it behind the same interface.
- The IPC secret and the keystore password still have to live somewhere. Keep them in your secrets manager, not in the repository.

## The failure that cannot be undone

**If every copy of the PyHSM keystore, or its password, is lost, every customer's data is permanently unreadable.** Database backups do not help: the data keys in them are wrapped by a key that no longer exists.

So:
1. Back up the keystore (`infra/scripts/backup_keystore.sh`) on a schedule **and after every rotation**.
2. Keep the keystore password, the IPC secret, the database backups and the keystore backups in **different places with different people able to reach them**. Anyone holding the keystore backup, its password, and a database backup can read everything.
3. Run the drill (below) before launch and after any change.

The backend refuses to start if PyHSM has no key by the expected name, instead of quietly making a new one. A new key would orphan every stored customer key.

## First-time setup

1. **Build the image** from the PyHSM repository: `docker build -t pyhsm:2.4.0 /path/to/PyHSM`.
   **PyHSM must have IPC authentication version 2** (the release after 2.3.7). VectorScrub signs every request that way: the signature covers the whole request and its time, and a request is accepted once. PyHSM 2.3.7 and earlier refuse these requests, and the new PyHSM refuses old-style ones, so upgrade both together. PyHSM also refuses any connecting user other than its own (compose runs both as uid 1000), and writes the kernel-reported uid and pid of every caller into its audit log.
2. **Create the secrets** (never commit them; put them in your secrets manager):
   ```bash
   openssl rand -hex 32                      # PYHSM_IPC_SECRET
   install -m 600 /dev/null pyhsm-password   # then write a strong password (12+ chars) into it
   ```
   Set `PYHSM_IPC_SECRET` and `PYHSM_PASSWORD_FILE` in `.env`. The password file **must be mode 600**: PyHSM refuses to start with a group- or world-readable one ("password file ... is group-readable"), and the container keeps restarting.

   Give the two PyHSM volumes to uid 1000 before the first start. PyHSM and the backend both run as that user, and the socket directory must be writable by PyHSM only:
   ```bash
   docker volume create vectorscrub_pyhsm_data && docker volume create vectorscrub_pyhsm_socket
   docker run --rm -v vectorscrub_pyhsm_data:/d -v vectorscrub_pyhsm_socket:/s alpine sh -c 'chown 1000:1000 /d /s && chmod 750 /d && chmod 700 /s'
   ```
   (The volume names are `<compose project>_pyhsm_data` and `<compose project>_pyhsm_socket`; the project is the directory name, `docker` for `infra/docker`, unless you pass `-p`.)
3. **Start PyHSM, the database and Redis, then create the key** before the backend runs:
   ```bash
   docker compose up -d pyhsm postgres redis
   docker compose run --rm --no-deps backend kms init     # prints "Created key 'vectorscrub-kek'"
   ```
   If you start the backend first it refuses to start ("the key service has no key named 'vectorscrub-kek'") and restarts until the key exists. That is deliberate.
4. **Back up the keystore now**, before any customer exists:
   ```bash
   docker run --rm -v vectorscrub_pyhsm_data:/data:ro -v "$PWD/keystore-backups:/out" alpine cp /data/keystore.enc /out/keystore.enc
   PYHSM_KEYSTORE=$PWD/keystore-backups/keystore.enc BACKUP_GPG_RECIPIENT=<keystore key> infra/scripts/backup_keystore.sh
   ```
   (On a plain host, point `PYHSM_KEYSTORE` at the keystore file directly.) Use a GPG key **different** from the one for database backups.
5. `docker compose up -d`. The backend waits up to 60 seconds for PyHSM, then checks the key exists.
6. `docker compose exec backend ./vectorscrub kms status` should show the key service answering and the key present.

**Running operator commands on a live stack:** use `docker compose exec backend ./vectorscrub kms ...`, not `docker compose run`. The backend has a fixed address on the agent network, so a second backend container from `run` fails with "Address already in use".

## Everyday operations

| Task | Command | Notes |
|---|---|---|
| Is it healthy? | `vectorscrub kms status` | Provider, whether PyHSM answers, whether the key exists, how many customer keys are wrapped under which KEK version, and how many values are still in the old format (should be 0). `/health/ready` also reports the key service. |
| Rotate the KEK | `vectorscrub kms rotate` | Adds a new KEK version in PyHSM and re-wraps every customer data key under it. Customer data is **not** re-encrypted, so it is fast. Old versions stay in the keystore so older database backups still open. **Back up the keystore right after.** |
| Re-wrap only | `vectorscrub kms rewrap` | After a rotation that was interrupted, or to move keys from the development provider. Each key is replaced only after the new wrapping is proven to open to the same key. |
| Verify a backup | `vectorscrub verify-backup` | Opens a sample of data keys, identity profiles, two-factor secrets and listing links through the key service. Exit code 1 means not usable. |

**Rotation schedule:** rotate at least yearly and whenever a person who had access to the keystore password leaves. Rotation protects against future compromise of an old version; it does not undo a compromise that already happened.

## The recovery drill

This is the test that proves you can recover. It was run during development against a real PyHSM (database and keystore both destroyed, both restored from encrypted backups, data opened); the log is at the bottom. Run it on real infrastructure, with the real secrets manager, before launch.

1. Take both backups: `infra/scripts/backup.sh` and `infra/scripts/backup_keystore.sh`.
2. On a **scratch machine**, restore the keystore backup (`gpg --decrypt ... > keystore.enc`), put the keystore password in a file, and start PyHSM on it:
   `vectorguard-pyhsm-server --store keystore.enc --password-file pw --socket s.sock --ipc-secret "$PYHSM_IPC_SECRET"`.
3. Restore and verify the database through that PyHSM:
   ```bash
   RESTORE_ADMIN_URL=postgres://...scratch... \
   KEY_PROVIDER=pyhsm PYHSM_SOCKET=$PWD/s.sock PYHSM_IPC_SECRET=... \
   infra/scripts/restore_verify.sh path/to/vectorscrub-....dump.gpg
   ```
   It must end with `RESULT: usable`.
4. Delete the scratch machine's copies of the keystore and password.

Things the drill catches: a keystore backup that is empty or stale, a wrong password, a wrong IPC secret, a database backup made under a KEK version your keystore backup does not have yet (back up the keystore after every rotation), and a PyHSM version that cannot read the file.

## Restoring for real

1. Restore the **keystore** first and start PyHSM on it. Check `kms status` says the key exists.
2. Restore the database (`docs/runbooks/backup-and-restore.md`), then run `vectorscrub verify-backup` against it. Do not start the service on "NOT usable".
3. Start the backend. It applies any missing migrations.
4. If the keystore backup is **older** than the database backup, a rotation may be missing. `verify-backup` will say data keys do not unwrap. Find a newer keystore backup; do not rotate or `kms init` over it.

## Moving existing data to this scheme (once)

Data written before envelope encryption used a key derived from `MASTER_ENCRYPTION_KEY`. Move it **after a verified database backup**:

```bash
vectorscrub kms init                               # the KEK must exist
MASTER_ENCRYPTION_KEY=<the old key> vectorscrub kms migrate-legacy
vectorscrub kms status                             # "values still in the old encryption format: 0"
vectorscrub verify-backup
```

It converts each customer in one transaction (all of a customer's fields or none), is safe to re-run, and reports failed customers by id without printing data. Scan tokens already queued in Redis stop validating; the recovery loop re-queues those jobs. Afterwards remove `MASTER_ENCRYPTION_KEY` from every environment and from your secrets manager history as far as you can. The prototype's data can instead simply be deleted.

## Rate limit

PyHSM limits operations **per key per minute** (`--rate-limit`, set from `PYHSM_RATE_LIMIT` in compose, default here 1200; PyHSM's own default of 100 is too low). VectorScrub wraps every customer's data key with one key, so all of these count against the same limit:

| What | Calls to PyHSM |
|---|---|
| A customer's request when their data key is not in the backend's cache | 1 |
| A customer's first encrypted value (their data key is created) | 1 |
| Re-wrapping one customer during `kms rotate` or `kms rewrap` | 3 (unwrap, wrap, check) |
| `verify-backup`, per sampled customer | 1 |

**When the limit is reached:** a customer's request fails at once with a server error (it never waits), the backend logs a warning and counts `key_service_rate_limited_total`. Operator commands (`kms rotate`, `kms rewrap`, `kms migrate-legacy`, `verify-backup`) slow down and retry instead of failing, so a rotation always finishes; it just takes longer.

**Sizing.** Allow for your busiest minute of cache misses, with headroom:

> limit ≥ 2 × (customers active in a 5-minute window ÷ 5)   (with the default 5-minute cache)

1200 per minute covers about 3,000 customers active within any 5 minutes. With it, rotating 10,000 customers (30,000 calls) takes about 25 minutes. Watch `key_service_rate_limited_total`: any growth during normal traffic means the limit or `DEK_CACHE_TTL_SECS` is too low.

**Why not set it very high:** the limit is also how fast someone who has taken over the backend can unwrap customer keys in bulk. At 1200 per minute, unwrapping 100,000 customers takes over an hour, which gives monitoring time to notice. Alert on a sustained rise in PyHSM unwrap calls (its audit log and metrics), not only on errors.

## When the key service is down

Requests that need a customer's key fail with a server error; nothing falls back to anything weaker. Keys cached in memory (up to `DEK_CACHE_TTL_SECS`) keep working. `/health/ready` reports not ready so a load balancer can stop sending traffic. Restart PyHSM; the backend reconnects by itself and recovers without a restart.

## Permissions

PyHSM's socket is mode 600 and owned by the user PyHSM runs as, so the backend must run as the same user (compose runs both as uid 1000) and be the only container that mounts the socket volume. PyHSM has no network access at all (`network_mode: none`).

## Settings

| Variable | Meaning |
|---|---|
| `KEY_PROVIDER` | `pyhsm` (production) or `env` (development and tests: the KEK is derived from `MASTER_ENCRYPTION_KEY` in the process). Production refuses `env` unless `KEY_PROVIDER_ALLOW_ENV=true`. |
| `PYHSM_SOCKET` | Default `/run/pyhsm/pyhsm.sock`. |
| `PYHSM_IPC_SECRET` or `PYHSM_IPC_SECRET_FILE` | 32+ characters. The backend's credential to PyHSM. |
| `PYHSM_KEY_ID` | Default `vectorscrub-kek`. |
| `PYHSM_TIMEOUT_MS` | Default 5000. A stalled PyHSM fails a request instead of hanging it. |
| `DEK_CACHE_TTL_SECS` | Default 300; 0 disables the cache (every request asks PyHSM). |
| `PYHSM_RATE_LIMIT` (compose, PyHSM side) | Default 1200 operations per key per minute. See "Rate limit". |

## Drill log

| Date | Environment | Result | Notes |
|---|---|---|---|
| 2026-11-20 | Developer machine, PyHSM with IPC authentication v2 (unreleased, after 2.3.7) | Register, encrypted reads, `kms rotate`, `verify-backup`: usable; zero authentication rejections; PyHSM audit log records `vectorscrub-backend(uid=…,pid=…)` | The Rust client reproduces PyHSM's frozen signing vector. Separately checked in the PyHSM image on Linux: replay refused, version-1 token refused, root refused by uid even with the right secret, health check healthy. |
| 2026-11-20 | `docker compose` stack in production and development mode: PyHSM container (no network, read-only, all capabilities dropped, uid 1000), backend with no master key in its environment | Backend refused to start before `kms init`, started after it; register, save identity, add a listing, read them back (address state and listing link decrypted); `kms rotate` on the live stack; PyHSM stopped: `/health/ready` 503 with `key_service: false` and reads fail with a server error (cache off); PyHSM started again: recovered with no backend restart; keystore backed up from its volume with `backup_keystore.sh` | Found and fixed during this run: a duplicate `volumes:` key that made the compose file unparsable, and PyHSM's own Dockerfile not building (since fixed in PyHSM itself). |
| 2026-11-20 | Developer machine, real PyHSM 2.3.7 over its Unix socket, GPG-encrypted backups with separate keys for database and keystore | Database dropped and live keystore deleted; both restored from backup; `verify-backup`: usable (1 data key, 1 identity profile, 1 two-factor secret, 1 listing link opened) after three KEK rotations | Also checked: wrong IPC secret, a different keystore with the same key name, and an unreachable socket each give "NOT usable". Says nothing about production secrets handling, size or network. |

## Before you rely on this (all open)

- [ ] PyHSM image built from a reviewed PyHSM release and pinned by digest
- [ ] Keystore password and IPC secret in a secrets manager, readable by two people, not by the backup operator
- [ ] Keystore backup schedule installed with its own heartbeat monitor; first backup taken before the first customer
- [ ] Keystore backups stored in a different account or provider from database backups, under a different GPG key
- [ ] The drill run on real infrastructure with the real secrets, and logged above
- [ ] A named person responsible for rotation, and a calendar reminder
- [ ] Decision recorded: PyHSM is a software key service (no FIPS validation, no tamper-resistant hardware), or a plan to replace it behind the same interface
