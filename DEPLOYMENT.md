# VectorScrub deployment

> This service holds the data sets used for identity theft and stalking. Read `docs/security/PENTEST_BRIEF.md`
> (the "Known limits" list) and `PRODUCTION_CHECKLIST.md` before putting a real customer on it.

## What is supported

**Docker Compose on a single Linux host, behind nginx with TLS.** That is the one arrangement the repository builds,
tests (the CI `images` job validates the compose file, builds the three images and scans them) and documents.

Not supported, and no files for them are kept: AWS ECS/Fargate, Kubernetes, or any other orchestrator. An earlier set of ECS task
definitions was deleted because it could not start (it injected a master key that production refuses, had no key
service or internal port, used `:latest` images and a health check the image cannot run). If you move to one of those,
you are porting `infra/docker/docker-compose.yml`; keep its properties:

- the key service (PyHSM) reachable only by the backend, over a Unix socket in a shared volume, with no network;
- the backend's internal port (`INTERNAL_PORT`) reachable only by the agents;
- the agents with no route to Postgres and none of the backend-only secrets;
- the backend connecting to Postgres as the **runtime** role (`docs/runbooks/database-roles.md`);
- images pinned by digest, no `latest`.

A managed Postgres or Redis works with the compose stack's backend and agents: point `DATABASE_URL` / `REDIS_URL` at them
(`sslmode=require`, `rediss://`), drop the plaintext overrides, and follow "Anywhere else" in `database-roles.md`.

## Host

4 CPU, 8 GB RAM, 80 GB SSD, Ubuntu 24.04 LTS, Docker with the Compose plugin, nginx, certbot. Only ports 80 and 443 are
public; SSH by key only. Every container port is published on `127.0.0.1` at most (Postgres and Redis publish none).

## Secrets

```bash
infra/scripts/gen_keys.sh        # prints every secret; copy them into your secrets manager, then clear your terminal
```

| Setting | Notes |
|---|---|
| `POSTGRES_PASSWORD` | The schema **owner**. Used by the `migrate` service and by operators only; never given to the backend. |
| `DB_APP_PASSWORD` | The **runtime** role the backend connects as. 24+ characters, URL-safe (hex). |
| `REDIS_PASSWORD` | Hex. Written to a config file on a tmpfs inside the Redis container, not onto its command line. |
| `JWT_SECRET`, `AGENT_API_KEY` | 32+ characters; the backend refuses to start with less. |
| `PYHSM_IPC_SECRET`, `PYHSM_PASSWORD_FILE` | `docs/runbooks/key-service.md`. The password file is mode 600. |
| `PYHSM_IMAGE` | The PyHSM image **by digest** (`pyhsm@sha256:...`). No default; compose refuses to start without it. |
| `METRICS_TOKEN` | Optional. Without it `/api/v1/metrics` answers 404 in production. |
| `ENVIRONMENT=production`, `ALLOWED_ORIGINS=https://...` | Required. A misspelt `ENVIRONMENT` stops the backend rather than running as development. |
| `TRUST_PROXY_HEADERS=true` | Only behind the nginx config in `infra/nginx/`, which overwrites `X-Forwarded-For`. |

`MASTER_ENCRYPTION_KEY` is **not** used in production (`KEY_PROVIDER=pyhsm`; production refuses `env`). The full list of
settings is in `README.md` and `.env.example`.

## First deployment

`infra/scripts/deploy-simple.sh` does the following on a fresh host, prompting for the domain, the PyHSM image digest and the
LLM and SMTP credentials. It has **not** been run end to end against a real PyHSM image; read it first. By hand:

1. Build the PyHSM image from a reviewed release and note its digest (`docs/runbooks/key-service.md`, step 1).
2. Clone the repository to `/opt/vectorscrub` and write `infra/docker/.env` (mode 600) with the settings above. Set `COMPOSE_PROJECT_NAME=vectorscrub`.
3. Give the PyHSM volumes to uid 1000, start PyHSM, Postgres, Redis and the `migrate` service, and create the key
   (`key-service.md`, steps 2 and 3). Postgres creates the runtime role the first time its volume is initialised.
4. **Back up the keystore now**, before any customer exists (`key-service.md`, step 4). Losing it loses all customer data.
5. `docker compose up -d --build --wait`. The backend refuses to start until PyHSM has the key, the migrations are applied
   and its database role is the runtime role.
6. nginx and TLS: `infra/nginx/README.md`. Do not route `/api/` straight to the backend port.
7. Check:
   ```bash
   curl -fsS https://app.example.com/api/v1/health/ready
   docker compose exec backend ./vectorscrub kms status
   curl -s -o /dev/null -w '%{http_code}\n' https://app.example.com/api/v1/internal/removal-requests   # 404
   curl -s -o /dev/null -w '%{http_code}\n' https://app.example.com/api/v1/metrics                      # 404
   ```
   Then the self-test list in `docs/security/PENTEST_BRIEF.md`, "Before you commission it".

## Updating

```bash
cd /opt/vectorscrub && git pull
cd infra/docker && docker compose up -d --build        # the migrate service runs first; the backend waits for it
docker compose ps && curl -fsS localhost:8080/api/v1/health/ready
```
Images are built locally and tagged `vectorscrub-backend:local`; base images are pinned by digest, so a rebuild does not pick
up a new base by surprise. Bump them deliberately (Dependabot opens the pull requests).

**Rollback:** check out the previous commit and run the same command. Migrations only go forward. If one must be undone,
restore the database from backup (`docs/runbooks/backup-and-restore.md`), then run `infra/scripts/db_roles.sh`.

## Backups, monitoring, incidents

- Backups, restores, targets and the drill log: `docs/runbooks/backup-and-restore.md`. **None of it runs on a schedule until you install it** (`infra/backup/`).
- Audit chain: install the daily audit-head job (`infra/backup/README.md`, step 5; `infra/scripts/audit_head.sh`). It verifies the chain, checks the previous stored head and stores today's off the host.
- Alerts: `docs/runbooks/alerting.md`. `infra/monitoring/render.sh` routes them to a named person; `docker-compose.monitoring.yml` runs Prometheus and Alertmanager.
- Staging: `docs/runbooks/staging.md` (`infra/staging/staging.sh`). Deploy there first; run its smoke test and restore drill before each production release.
- Restore drill: `infra/scripts/restore_drill.sh` (database and keystore together); results in `docs/runbooks/drill-log.md`.
- Incidents and outages: `docs/runbooks/incident-response.md`.
- Other runbooks: `docs/runbooks/README.md`.

## Troubleshooting

| Symptom | Cause |
|---|---|
| `docker compose` stops with "... is required" | A secret is missing from `.env`. The message names it. |
| Backend exits: "RUN_MIGRATIONS=false but migrations are not applied" | The `migrate` service did not finish. `docker compose logs migrate`. |
| Backend exits: "The database role in DATABASE_URL is more powerful than the service needs" | `DATABASE_URL` names the owner. Use the runtime role (`database-roles.md`). |
| Backend exits: "the key service has no key named ..." | `kms init` was not run. It is deliberately not automatic (`key-service.md`). |
| `/health/ready` 503 with `key_service: false` | PyHSM is down or its socket volume has the wrong owner. |
| Agents cannot reach the backend | `BACKEND_API_URL` must be `http://backend-internal:8081` (a name that resolves only on the agent network; plain `backend` can resolve to the web-network address, where the internal port is not listening), and `AGENT_API_KEY` must match in both. |
