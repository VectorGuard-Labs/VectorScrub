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
- [ ] PostgreSQL database provisioned (RDS recommended for AWS)
- [ ] Redis cache provisioned (ElastiCache recommended for AWS)
- [ ] Backup storage configured (S3, Cloud Storage, etc.)
- [ ] Monitoring service configured (CloudWatch, Datadog, Grafana, etc.)
- [ ] Log aggregation configured (CloudWatch Logs, ELK, etc.)
- [ ] Email service configured (SMTP credentials ready)

### Environment Variables

Verify all required variables are set:

- [ ] `DATABASE_URL` - PostgreSQL connection string
- [ ] `REDIS_URL` - Redis connection string (with password)
- [ ] `JWT_SECRET` - Strong random secret (48+ bytes)
- [ ] `MASTER_ENCRYPTION_KEY` - AES key (32 bytes base64)
- [ ] `AGENT_API_KEY` - Internal API authentication
- [ ] `ANTHROPIC_API_KEY` or `OPENAI_API_KEY` - LLM provider
- [ ] `SMTP_HOST`, `SMTP_USERNAME`, `SMTP_PASSWORD` - Email service
- [ ] `ALLOWED_ORIGINS` - Frontend domain(s)
- [ ] `ENVIRONMENT=production`

### Optional but Recommended

- [ ] `OTLP_ENDPOINT` - OpenTelemetry for distributed tracing
- [ ] `DROP_API_KEY` - California DROP integration
- [ ] `MASTER_ENCRYPTION_KEY` injected from a secrets manager (there is no HSM/KMS integration in the code)
- [ ] `JWT_SECRET` and `AGENT_API_KEY` each at least 32 characters (startup rejects shorter)
- [ ] `TRUST_PROXY_HEADERS=true` only if the proxy overwrites `X-Forwarded-For`/`X-Real-IP`; `/metrics` restricted at the proxy

## Deployment

### Build & Deploy

- [ ] Docker images built from latest code
- [ ] Images pushed to container registry (ECR, Docker Hub, etc.)
- [ ] Database migrations tested in staging environment
- [ ] Load balancer configured and health checks passing
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
- [ ] Password reset flow works
- [ ] Identity profile creation works
- [ ] Scan job submission works
- [ ] Agents processing jobs (check logs)
- [ ] Exposures being detected and stored
- [ ] Removal requests can be authorized
- [ ] Email notifications sending (if configured)

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

- [ ] Database backup tested and working
- [ ] Backup restoration tested
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

## Quick Start Commands

### Simple VPS Deployment

```bash
# One-command deployment
curl -fsSL https://raw.githubusercontent.com/yourusername/VectorScrub/main/infra/scripts/deploy-simple.sh | sudo bash
```

### AWS ECS Deployment

```bash
# Build and push images
cd /path/to/VectorScrub
./infra/scripts/build-and-push-aws.sh <account-id> <region>

# Deploy infrastructure
cd infra/aws
terraform init
terraform plan
terraform apply
```

### Manual Docker Compose

```bash
cd /path/to/VectorScrub/infra/docker

# Generate secrets
../scripts/gen_keys.sh

# Edit .env with your values
nano .env

# Deploy
docker compose up -d

# Check logs
docker compose logs -f
```

---

## Emergency Contacts

| Role | Contact | Phone | Email |
|------|---------|-------|-------|
| On-Call Engineer | TBD | TBD | TBD |
| Security Lead | TBD | TBD | TBD |
| Database Admin | TBD | TBD | TBD |
| Cloud Operations | TBD | TBD | TBD |

---

## Rollback Procedure

If deployment fails or critical issues discovered:

```bash
# Option 1: Rollback to previous Docker images
docker compose down
docker compose pull <previous-tag>
docker compose up -d

# Option 2: Restore from backup (if database corrupted)
gunzip < /var/backups/vectorscrub/db-latest.sql.gz | \
  docker exec -i vectorscrub-postgres psql -U vectorscrub vectorscrub

# Option 3: AWS ECS rollback
aws ecs update-service \
  --cluster vectorscrub-production \
  --service vectorscrub-backend \
  --task-definition vectorscrub-backend:<previous-revision> \
  --force-new-deployment
```

---

**Sign-Off**

Deployment approved by:

- [ ] Engineering Lead: ___________________ Date: ___________
- [ ] Security Lead: ___________________ Date: ___________
- [ ] Operations Lead: ___________________ Date: ___________

Deployment completed by: ___________________ Date: ___________
