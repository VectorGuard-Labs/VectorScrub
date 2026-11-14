# VectorScrub Operational Runbooks

## Overview
Step-by-step troubleshooting guides for common operational scenarios.

## Available Runbooks

### System Health
1. [Broker Outages](./broker-outage.md) - Handle broker downtime
2. [High Error Rates](./high-error-rates.md) - Diagnose system-wide failures
3. [Database Issues](./database-issues.md) - PostgreSQL troubleshooting

### Agent Issues
4. [Scan Failures](./scan-failures.md) - Discovery/detection problems
5. [Removal Failures](./removal-failures.md) - Submission issues
6. [CAPTCHA Detection](./captcha-detection.md) - New CAPTCHA on broker
7. [Rate Limit Breaches](./rate-limit-breach.md) - IP bans, 429 errors

### Infrastructure
8. [SMTP Failures](./smtp-failures.md) - Email delivery problems
9. [Redis Issues](./redis-issues.md) - Queue/cache problems
10. [Monitoring Alerts](./monitoring-alerts.md) - Alert response procedures

## Runbook Format

Each runbook follows this structure:

1. **Symptoms** - How to identify the issue
2. **Impact** - Customer/system impact
3. **Diagnosis** - Commands to run, logs to check
4. **Resolution** - Step-by-step fix
5. **Prevention** - Avoid recurrence
6. **Escalation** - When to page on-call

## Quick Reference

| Symptom | Runbook | Severity |
|---------|---------|----------|
| Broker returns 429 | [Rate Limit Breach](./rate-limit-breach.md) | Medium |
| All removals failing | [High Error Rates](./high-error-rates.md) | High |
| CAPTCHA on previously working broker | [CAPTCHA Detection](./captcha-detection.md) | Medium |
| Scans timing out | [Scan Failures](./scan-failures.md) | High |
| Email removals not sending | [SMTP Failures](./smtp-failures.md) | High |
| Redis connection errors | [Redis Issues](./redis-issues.md) | Critical |
| Database connection pool exhausted | [Database Issues](./database-issues.md) | Critical |

## On-Call Procedures

### Severity Levels

**Critical (P0)** - System down, all customers affected
- Response time: 15 minutes
- Resolution SLA: 1 hour
- Auto-page on-call

**High (P1)** - Major feature broken, many customers affected
- Response time: 30 minutes
- Resolution SLA: 4 hours
- Page on-call during business hours

**Medium (P2)** - Degraded service, some customers affected
- Response time: 2 hours
- Resolution SLA: 24 hours
- Create ticket, no page

**Low (P3)** - Minor issue, few customers affected
- Response time: Next business day
- Resolution SLA: 1 week
- Create ticket

### Escalation Path

1. **On-call Engineer** - First responder
2. **Engineering Lead** - If unresolved after 1 hour
3. **CTO** - If unresolved after 4 hours or critical data issue
4. **CEO** - Major outage >24 hours or security incident

## Contact List

- On-call: PagerDuty rotation
- Engineering Lead: [contact]
- Database Admin: [contact]
- Infrastructure: [contact]
- Security: [contact]
