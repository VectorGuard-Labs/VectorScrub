# Production Deployment Checklist

Use this checklist before deploying VectorScrub to production.

## Pre-Deployment

### Security

- [ ] All default passwords changed (`changeme_in_production` must be replaced)
- [ ] Secrets generated using `infra/scripts/gen_keys.sh`
- [ ] Secrets stored in secrets manager (AWS Secrets Manager, HashiCorp Vault, etc.)
- [ ] `.env` files excluded from version control (verify `.gitignore`)
- [ ] SSL/TLS certificate obtained (Let's Encrypt or commercial)
- [ ] Domain DNS configured and pointing to server/load balancer
- [ ] Firewall rules configured (only 80/443 public, restrict SSH)
- [ ] SSH key-only authentication enabled (passwords disabled)
- [ ] Database encryption at rest enabled
- [ ] Database SSL/TLS connections enforced

### Infrastructure

- [ ] Server/VM provisioned with adequate resources (4 CPU, 8GB RAM minimum)
- [ ] PostgreSQL: the compose container, or a managed instance with TLS (`database-roles.md`, "Anywhere else")
- [ ] Redis: the compose container, or a managed instance with TLS
- [ ] Backup storage configured (S3, Cloud Storage, etc.)
- [ ] Prometheus and Alertmanager running with `infra/monitoring/` and routed to a named person
- [ ] Log aggregation configured (CloudWatch Logs, ELK, etc.)
- [ ] SMTP credentials ready if email opt-outs to companies are used (no email is sent to customers; see `docs/runbooks/smtp-failures.md`)

### Environment Variables

Verify all required variables are set:

- [ ] `DATABASE_URL` - PostgreSQL connection string
- [ ] `REDIS_URL` - Redis connection string (with password)
- [ ] `JWT_SECRET` - Strong random secret (48+ bytes)
- [ ] `PYHSM_IPC_SECRET`, the PyHSM keystore password (`PYHSM_PASSWORD_FILE`) and `PYHSM_IMAGE` pinned by digest. No `MASTER_ENCRYPTION_KEY` in production.
- [ ] `DB_APP_PASSWORD`: the backend connects as the runtime role, not the owner (`docs/runbooks/database-roles.md`); `vectorscrub migrate` runs as the owner
- [ ] `AGENT_API_KEY` - Internal API authentication
- [ ] `ANTHROPIC_API_KEY` or `OPENAI_API_KEY` - LLM provider
- [ ] `SMTP_HOST`, `SMTP_USERNAME`, `SMTP_PASSWORD` - only for email opt-outs to companies
- [ ] `ALLOWED_ORIGINS` - Frontend domain(s)
- [ ] `ENVIRONMENT=production`

### Optional but Recommended

- [ ] `OTLP_ENDPOINT` - OpenTelemetry for distributed tracing
- [ ] `DROP_API_KEY` - California DROP integration
- [ ] PyHSM IPC secret and keystore password injected from a secrets manager; `vectorscrub kms status` shows the key present; keystore backup taken and the recovery drill run (`docs/runbooks/key-service.md`)
- [ ] `JWT_SECRET` and `AGENT_API_KEY` each at least 32 characters (startup rejects shorter)
- [ ] `TRUST_PROXY_HEADERS=true` only if the proxy overwrites `X-Forwarded-For`/`X-Real-IP`; `/metrics` restricted at the proxy

## Deployment

### Build & Deploy

- [ ] Docker images built from latest code
- [ ] Database migrations tested in staging environment
- [ ] nginx and TLS in place from `infra/nginx/`; health checks passing
- [ ] Services deployed (backend, agents, frontend)
- [ ] Reverse proxy/CDN configured (Nginx, CloudFront, etc.)

### Verification

- [ ] Health check endpoints responding:
  - [ ] `GET /api/v1/health/live` returns 200
  - [ ] `GET /api/v1/health/ready` returns 200 (verifies DB/Redis)
- [ ] HTTPS working (no certificate warnings)
- [ ] HTTP → HTTPS redirect working
- [ ] CSRF token endpoint working: `GET /api/v1/csrf-token`
- [ ] Metrics endpoint accessible: `GET /api/v1/metrics` (restricted to internal only)
- [ ] Frontend loads and displays correctly
- [ ] API endpoints responding correctly

### Functional Testing

- [ ] User registration works
- [ ] User login works
- [ ] Lost-password behaviour understood: there is **no** password reset and no email verification (no mail is sent to customers). A forgotten password locks the account. Decide how support answers that before launch
- [ ] Identity profile creation works
- [ ] Scan job submission works
- [ ] Agents processing jobs (check logs)
- [ ] Exposures being detected and stored
- [ ] Removal requests can be authorized

## Post-Deployment

### Monitoring Setup

- [ ] Prometheus scraping `/api/v1/metrics`
- [ ] Grafana dashboard configured
- [ ] Alerts configured for:
  - [ ] Service down
  - [ ] High error rate (>1%)
  - [ ] High latency (p99 >1s)
  - [ ] Database connection pool exhausted
  - [ ] Redis connection failures
  - [ ] Disk space <20%
- [ ] Log aggregation working
- [ ] Error tracking configured (Sentry, etc.)

### Backup & Recovery

Procedure, scripts and the drill log: [docs/runbooks/backup-and-restore.md](docs/runbooks/backup-and-restore.md). The PyHSM keystore is part of the backup: without it the data cannot be opened.

- [ ] Database backup tested and working
- [ ] Backup restoration tested (`restore_verify.sh` reports "usable" against the off-site copy, with the key from the secrets manager)
- [ ] Backup retention policy configured (30 days minimum)
- [ ] Off-site backup storage configured
- [ ] Disaster recovery plan documented
- [ ] RTO (Recovery Time Objective) defined
- [ ] RPO (Recovery Point Objective) defined

### Security Hardening

- [ ] Security headers verified:
  - [ ] Content-Security-Policy
  - [ ] Strict-Transport-Security (HSTS)
  - [ ] X-Frame-Options: DENY
  - [ ] X-Content-Type-Options: nosniff
- [ ] Rate limiting tested and working
- [ ] CSRF protection tested
- [ ] SQL injection prevention tested
- [ ] XSS prevention tested
- [ ] Audit log working (check `removal_audit_log` table)
- [ ] `vectorscrub audit verify` passes, and its head is stored daily somewhere the database host cannot write (`docs/runbooks/audit-chain.md`)
- [ ] PII masking in logs verified (no raw PII in logs)

### Performance

- [ ] Load testing performed (minimum expected load)
- [ ] Database query performance acceptable (<100ms p95)
- [ ] API response times acceptable (<200ms p95)
- [ ] Frontend load time acceptable (<3s)
- [ ] Auto-scaling configured (if using cloud)
- [ ] Connection pool sizes optimized
- [ ] Redis cache hit rate >80%

### Compliance

- [ ] Privacy policy published
- [ ] Terms of service published
- [ ] Cookie consent banner (if serving EU users)
- [ ] GDPR compliance verified (if serving EU users)
- [ ] CCPA compliance verified (if serving CA users)
- [ ] Data retention / account-erasure policy decided and implemented (none runs today; see `DATA_RETENTION.md`)
- [ ] Right to deletion mechanism tested
- [ ] Data export mechanism tested
- [ ] Breach notification procedures documented

### Documentation

- [ ] Runbook created for common operations
- [ ] On-call procedures documented
- [ ] Incident response plan documented
- [ ] Escalation paths defined
- [ ] Contact list updated (ops, security, engineering)
- [ ] Architecture diagram created
- [ ] Dependency map documented

### Team Readiness

- [ ] Operations team trained
- [ ] On-call rotation scheduled
- [ ] Incident management tool configured (PagerDuty, etc.)
- [ ] Communication channels set up (Slack, Discord, etc.)
- [ ] Status page configured (if public-facing)

## Ongoing Maintenance

### Daily

- [ ] Check error logs
- [ ] Review security alerts
- [ ] Monitor key metrics (errors, latency, throughput)

### Weekly

- [ ] Review backup logs
- [ ] Check disk usage
- [ ] Review slow queries
- [ ] Update dependencies (security patches)

### Monthly

- [ ] Test backup restoration
- [ ] Review and rotate access keys
- [ ] Security scan (vulnerability assessment)
- [ ] Performance review and optimization
- [ ] Cost optimization review

### Quarterly

- [ ] Disaster recovery drill
- [ ] Security audit
- [ ] Penetration testing
- [ ] Compliance review (SOC 2, GDPR, etc.)
- [ ] Incident response simulation

---

## Deploying

Docker Compose on a single host is the supported deployment. Follow [DEPLOYMENT.md](DEPLOYMENT.md); `infra/scripts/deploy-simple.sh`
automates it but has not been run end to end against a real PyHSM image, so read it first. Do not pipe it from the internet into a shell.

---

## Rollback Procedure

See "Updating" in [DEPLOYMENT.md](DEPLOYMENT.md): check out the previous commit and `docker compose up -d --build`. Migrations only
go forward; to undo one, restore from backup (`docs/runbooks/backup-and-restore.md`, "Restoring for real"), run
`infra/scripts/db_roles.sh`, and `vectorscrub verify-backup` before starting the service.

---

**Sign-Off**

Deployment approved by:

- [ ] Engineering Lead: ___________________ Date: ___________
- [ ] Security Lead: ___________________ Date: ___________
- [ ] Operations Lead: ___________________ Date: ___________

Deployment completed by: ___________________ Date: ___________
