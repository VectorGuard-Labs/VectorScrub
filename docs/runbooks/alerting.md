# Runbook: alert routing to a named person

## What is set up
| File | Role |
|---|---|
| `infra/monitoring/alerts.yml` | The rules: backend down, 5xx rate, latency, audit-write failures, login floods, rate limiter on its fallback, PyHSM rate limiting, scan backlog, failed scans, abandoned removals, and **Watchdog** (always firing) |
| `infra/monitoring/alertmanager.yml.tmpl` | Routing: `severity=page` to the on-call person by email (and PagerDuty if configured), re-sent hourly; `severity=ticket` by email, twice a day; Watchdog to a dead-man's switch every 5 minutes; tickets muted while the backend is down |
| `infra/monitoring/render.sh` | Writes `alertmanager.yml` for one **named** person, checks it with `amtool`, and prints where each kind of alert goes. It refuses to run without a name, an email address and a dead-man's switch URL |
| `infra/monitoring/docker-compose.monitoring.yml` | Prometheus and Alertmanager beside the app, both on 127.0.0.1 only |

## Set it up (once, and whenever the on-call person changes)
1. Choose the person. Write their name and contact details in `incident-response.md`, "Contacts".
2. A dead-man's switch check (healthchecks.io or similar) expecting a ping every 5 minutes, with 10 minutes' grace, alerting that same person by phone or SMS. It is what tells you monitoring itself stopped. Separate from the three backup and audit heartbeats.
3. An SMTP account that can send from your domain (with SPF and DKIM, or the alerts land in spam), and optionally a PagerDuty Events v2 routing key. Put the password and the key in files, mode 600.
4. Render and start:
   ```bash
   ONCALL_NAME="<name>" ONCALL_EMAIL=<address> \
   ALERT_SMTP_SMARTHOST=<host>:587 ALERT_SMTP_FROM=alerts@<your-domain> \
   ALERT_SMTP_USERNAME=<user> ALERT_SMTP_PASSWORD_FILE=/root/secrets/smtp-password \
   ALERT_DEADMAN_URL=https://hc-ping.com/<uuid> \
   ONCALL_PAGERDUTY_ROUTING_KEY_FILE=/root/secrets/pagerduty-key \
   sudo -E infra/monitoring/render.sh

   install -m 600 /dev/null /root/secrets/metrics-token   # write the backend's METRICS_TOKEN into it
   sudo chown 65534:65534 /root/secrets/metrics-token     # Prometheus reads it as uid 65534 (Linux)
   cd infra/monitoring
   APP_PROJECT=vectorscrub METRICS_TOKEN_FILE=/root/secrets/metrics-token \
     docker compose -f docker-compose.monitoring.yml -p vectorscrub-monitoring up -d
   ```
5. **Prove it reaches the person** (do not skip this): stop the backend for three minutes. The person must get a `[VectorScrub PAGE FIRING] BackendDown` mail (and a PagerDuty page), then `RESOLVED` after you start it again. The dead-man's switch must show a ping in the last 5 minutes.
   ```bash
   cd infra/docker && docker compose stop backend && sleep 200 && docker compose start backend
   ```
   Do this outside customer hours; customers see errors for those minutes.
6. Write the date and result in the table below.

## What was tested
| Date | Where | Result |
|---|---|---|
| 2026-11-24 | Staging on a developer machine, with a local mail sink standing in for the inbox and a local listener for the dead-man's switch | Prometheus scraped the staging backend with its metrics token; all 11 rules loaded; Watchdog reached the dead-man listener within a minute; with the backend stopped, `[VectorScrub PAGE FIRING] BackendDown (for Test Oncall)` was delivered to the on-call address about 3 minutes later, and `RESOLVED` after it was started again. No real person, mail provider or PagerDuty account was involved. |
| | Production, real person, real provider | **Not done yet** |

## Limits
- Prometheus and Alertmanager run on the same host as the app. If the host dies, neither can send anything: only the dead-man's switch notices. That is why step 2 is not optional.
- One person. There is no rotation and no escalation if they do not respond; a paging service can add both.
- Thresholds are starting points (`alerts.yml`). Expect to tune them after the first week of real traffic.
