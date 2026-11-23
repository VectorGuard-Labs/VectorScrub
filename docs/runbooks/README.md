# VectorScrub operational runbooks

Step-by-step guides for the situations the system is built to handle. Each one describes what exists in the code and the
deployment today. **There is no on-call rotation, pager or status page yet**: a person has to be named for that
(`incident-response.md`, "Contacts") and Alertmanager has to be pointed at them (`infra/monitoring/`).

| Situation | Runbook |
|---|---|
| Security incident, suspected data exposure, outage | [incident-response.md](./incident-response.md) |
| Key service (PyHSM): setup, rotation, loss, rate limits | [key-service.md](./key-service.md) |
| Backups, restores, drills, recovery targets | [backup-and-restore.md](./backup-and-restore.md) |
| The two database roles (owner and runtime) | [database-roles.md](./database-roles.md) |
| The audit hash chain: verify it, store its head | [audit-chain.md](./audit-chain.md) |
| A CAPTCHA appears on a broker's opt-out page | [captcha-detection.md](./captcha-detection.md) |
| Email opt-outs to brokers failing (SMTP) | [smtp-failures.md](./smtp-failures.md) |

Monitoring: `infra/monitoring/alerts.yml` (alert rules, each naming the runbook to open) and `OBSERVABILITY.md`.

## Severity (used by the runbooks)
- **Critical**: customer identity data may have been read or changed by someone else; the key service or keystore is lost or exposed; the audit trail was altered.
- **High**: the service is down, or sign-in or removal submissions fail for everyone.
- **Medium**: one broker or one customer is affected; the rate limiter or Redis is degraded.
- **Low**: cosmetic or single-customer, no data at risk.

Response and resolution times are not promised anywhere (the free pilot says so to customers), so none are listed here. Set them
when someone is on call.
