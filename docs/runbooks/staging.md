# Runbook: the staging environment

`infra/staging/staging.sh` builds a stack the way production is built and runs it with **fake data only**:
the same `infra/docker/docker-compose.yml`, the same three images built from the checkout, PyHSM built from a
pinned commit (`PYHSM_REF`, default `v2.4.0`; the script refuses a different or modified checkout and records the
image digest in `~/.vectorscrub-staging/pyhsm-provenance.log`), `ENVIRONMENT=production`, the two database roles,
and its own generated secrets in `~/.vectorscrub-staging` (mode 700).

| Command | Does |
|---|---|
| `PYHSM_SRC=/path/to/PyHSM staging.sh up` | Build, start, create the key on a new keystore, wait until every service is healthy |
| `staging.sh smoke` | 48 checks from the pentest brief's self-test: closed endpoints, headers and CSP, a common password refused, sessions, cross-customer access, CSRF, sign-out everywhere, lockout, agent isolation and secrets, the database role, the audit chain, erasure, and Redis going down and coming back |
| `staging.sh backup` | Runs the **production** `backup.sh` and `backup_keystore.sh` against staging, encrypted to two staging-only GPG keys, uploaded to the off-site store |
| `staging.sh drill` | `restore_drill.sh` from that store: database and keystore together, and the data must open |
| `staging.sh down` / `destroy` | Stop (keeps volumes) / delete everything, after typing `destroy` |

The off-site store defaults to a **local** S3-compatible stand-in (`moto_server`, in memory). Set `STAGING_S3_URI`
and AWS credentials to use a real bucket from `infra/backup/aws-offsite-setup.sh` instead. Only that makes the drill an
off-site drill.

Run it on a machine that does not run production: the compose file uses fixed container names. Never put real
customer data in it.

## What running it found (2026-11-24, first run on a developer machine)
- **Agents could not reach the backend.** Inside compose, `backend` resolved to its address on the `web` network,
  while the internal listener binds only to the agent network. Every agent call was refused. Fixed with a network
  alias (`backend-internal`) that exists only on the agent network.
- **Redis could not save, and ran as root.** Moving its password into a config file bypassed the image's switch to
  the `redis` user, and with all capabilities dropped root could not write `/data`, so readiness went to "Redis down"
  after the first save. Fixed by running the container as `redis`.
- **The agent worker crash-looped without an LLM key**, taking removals and the nightly page check with it. The model
  client is now created on first use; a classification without a key fails as "could not check".
- `kms status` exits 0 when the key is missing (the script reads its output instead), and rebuilding PyHSM from
  the same commit gives a new image id (the script reuses the image built for a commit).

Smoke: 48 passed. Backup, off-site upload and the restore drill: usable (`drill-log.md`).

## What staging does not show
Production size and load, a real reverse proxy and TLS (point `STAGING_URL` at a real staging hostname to include
them), the real secrets manager, and a real off-site bucket unless `STAGING_S3_URI` is set.
