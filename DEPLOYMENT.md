# VectorScrub Production Deployment Guide

> **WARNING:** This guide assumes you understand the security implications of handling PII at scale. Review all security sections before deploying.

## Table of Contents

1. [Prerequisites](#prerequisites)
2. [Security Hardening](#security-hardening)
3. [Deployment Options](#deployment-options)
4. [Option 1: Docker Compose (Simple)](#option-1-docker-compose-simple)
5. [Option 2: Cloud Provider (Recommended)](#option-2-cloud-provider-recommended)
6. [Option 3: Kubernetes (Enterprise)](#option-3-kubernetes-enterprise)
7. [Post-Deployment](#post-deployment)
8. [Monitoring & Maintenance](#monitoring--maintenance)
9. [Disaster Recovery](#disaster-recovery)

---

## Prerequisites

### Required Accounts & Services

- [ ] Domain name with DNS control
- [ ] SSL/TLS certificate (Let's Encrypt recommended)
- [ ] Cloud provider account (AWS, GCP, DigitalOcean, etc.)
- [ ] Email service (for SMTP - Gmail, SendGrid, etc.)
- [ ] LLM API access (OpenAI or Anthropic)
- [ ] Secrets manager (AWS Secrets Manager, HashiCorp Vault, etc.)

### Local Tools

```bash
# Install required CLI tools
brew install docker docker-compose openssl postgresql redis-cli

# Verify installations
docker --version
docker-compose --version
psql --version
redis-cli --version
```

---

## Security Hardening

### 1. Generate Production Secrets

**CRITICAL: Never reuse development secrets in production**

```bash
cd /Users/pavondunbar/VectorScrub/infra/scripts
./gen_keys.sh > production-secrets.txt

# Review generated secrets
cat production-secrets.txt

# Store in secrets manager immediately
# DELETE the file after storing:
trash production-secrets.txt  # or rm -P production-secrets.txt
```

### 2. Update Default Passwords

**Critical changes required:**

```bash
# In infra/docker/docker-compose.yml, change ALL defaults:
# - POSTGRES_PASSWORD: changeme_in_production → (use generated value)
# - REDIS_PASSWORD: changeme_in_production → (use generated value)
```

### 3. Environment Variables for Production

Create a production `.env` file (stored in your secrets manager, NOT in git):

```bash
# Database
DATABASE_URL=postgres://vectorscrub:<POSTGRES_PASSWORD>@<DB_HOST>:5432/vectorscrub
POSTGRES_PASSWORD=<generated-strong-password>

# Redis
REDIS_URL=redis://:<REDIS_PASSWORD>@<REDIS_HOST>:6379
REDIS_PASSWORD=<generated-strong-password>

# API Security
JWT_SECRET=<generated-48-byte-base64>
MASTER_ENCRYPTION_KEY=<generated-32-byte-base64>
AGENT_API_KEY=<generated-random-string>

# Application
ENVIRONMENT=production
PORT=8080
ALLOWED_ORIGINS=https://yourdomain.com,https://www.yourdomain.com

# LLM Provider (choose one)
ANTHROPIC_API_KEY=<your-key>
DEFAULT_LLM_PROVIDER=anthropic
DEFAULT_LLM_MODEL=claude-3-5-sonnet-20241022

# SMTP (for removal requests)
SMTP_HOST=smtp.sendgrid.net
SMTP_PORT=587
SMTP_USERNAME=<your-smtp-username>
SMTP_PASSWORD=<your-smtp-password>
SMTP_FROM_EMAIL=noreply@yourdomain.com
SMTP_USE_TLS=true

# Optional: California DROP API
DROP_API_URL=https://privacy.ca.gov/api/v1
DROP_API_KEY=<your-drop-key>
DROP_API_SECRET=<your-drop-secret>
DROP_ENABLE_SANDBOX=false

# Optional: Observability
OTLP_ENDPOINT=https://your-otel-collector:4317
```

### 4. Security Best Practices

- [ ] Enable firewall rules (only ports 80, 443 publicly accessible)
- [ ] Use HTTPS exclusively (redirect HTTP → HTTPS)
- [ ] Enable database encryption at rest
- [ ] Enable database SSL/TLS connections
- [ ] Use private networks for backend ↔ database ↔ Redis
- [ ] Set up automated backups (daily minimum)
- [ ] Enable audit logging
- [ ] Configure fail2ban or equivalent for SSH

---

## Deployment Options

### Comparison Matrix

| Option | Cost | Complexity | Scalability | Best For |
|--------|------|------------|-------------|----------|
| Docker Compose | Low | Low | Limited | MVP, small deployments |
| Cloud VMs (AWS EC2, etc.) | Medium | Medium | Medium | Production, most users |
| Kubernetes | High | High | High | Enterprise, high scale |

---

## Option 1: Docker Compose (Simple)

### Quick deployment on a single VPS (DigitalOcean, Linode, Hetzner, etc.)

#### Step 1: Provision Server

```bash
# Recommended specs:
# - 4 CPU cores
# - 8GB RAM
# - 80GB SSD
# - Ubuntu 24.04 LTS

# SSH into server
ssh root@your-server-ip

# Update system
apt update && apt upgrade -y
```

#### Step 2: Install Docker

```bash
# Install Docker
curl -fsSL https://get.docker.com -o get-docker.sh
sh get-docker.sh

# Install Docker Compose
apt install docker-compose-plugin -y

# Verify
docker --version
docker compose version
```

#### Step 3: Deploy Application

```bash
# Create application directory
mkdir -p /opt/vectorscrub
cd /opt/vectorscrub

# Clone repository (or upload via rsync/scp)
git clone https://github.com/yourusername/VectorScrub.git .

# Create production .env file
cd infra/docker
nano .env  # Paste your production secrets

# Update docker-compose.yml with production values
# Change REDIS_PASSWORD and POSTGRES_PASSWORD

# Start services
docker compose up -d

# Check logs
docker compose logs -f
```

#### Step 4: Set Up Reverse Proxy (Nginx)

```bash
# Install Nginx
apt install nginx certbot python3-certbot-nginx -y

# Create Nginx config
cat > /etc/nginx/sites-available/vectorscrub <<'EOF'
server {
    listen 80;
    server_name yourdomain.com www.yourdomain.com;

    location / {
        proxy_pass http://127.0.0.1:3000;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }

    location /api/ {
        proxy_pass http://127.0.0.1:8080;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }
}
EOF

# Enable site
ln -s /etc/nginx/sites-available/vectorscrub /etc/nginx/sites-enabled/
nginx -t
systemctl reload nginx

# Get SSL certificate
certbot --nginx -d yourdomain.com -d www.yourdomain.com
```

---

## Option 2: Cloud Provider (Recommended)

### AWS Deployment

#### Architecture

```
Internet → ALB → ECS Fargate → RDS + ElastiCache
                      ↓
                CloudWatch Logs
```

#### Infrastructure Setup

```bash
# Install AWS CLI
brew install awscli
aws configure

# Install Terraform (optional but recommended)
brew install terraform
```

#### Step 1: Create VPC and Networking

```bash
# Create VPC with public and private subnets
aws ec2 create-vpc --cidr-block 10.0.0.0/16 --tag-specifications 'ResourceType=vpc,Tags=[{Key=Name,Value=vectorscrub-vpc}]'

# Note the VpcId from output
export VPC_ID=<vpc-id>

# Create public subnets (2 AZs for high availability)
aws ec2 create-subnet --vpc-id $VPC_ID --cidr-block 10.0.1.0/24 --availability-zone us-east-1a
aws ec2 create-subnet --vpc-id $VPC_ID --cidr-block 10.0.2.0/24 --availability-zone us-east-1b

# Create private subnets
aws ec2 create-subnet --vpc-id $VPC_ID --cidr-block 10.0.10.0/24 --availability-zone us-east-1a
aws ec2 create-subnet --vpc-id $VPC_ID --cidr-block 10.0.11.0/24 --availability-zone us-east-1b
```

#### Step 2: Create RDS PostgreSQL

```bash
# Create DB subnet group
aws rds create-db-subnet-group \
  --db-subnet-group-name vectorscrub-db-subnet \
  --db-subnet-group-description "VectorScrub DB subnet group" \
  --subnet-ids subnet-xxx subnet-yyy

# Create RDS instance
aws rds create-db-instance \
  --db-instance-identifier vectorscrub-postgres \
  --db-instance-class db.t4g.medium \
  --engine postgres \
  --engine-version 16.1 \
  --master-username vectorscrub \
  --master-user-password <POSTGRES_PASSWORD> \
  --allocated-storage 100 \
  --storage-type gp3 \
  --storage-encrypted \
  --backup-retention-period 7 \
  --db-subnet-group-name vectorscrub-db-subnet \
  --vpc-security-group-ids <security-group-id> \
  --no-publicly-accessible
```

#### Step 3: Create ElastiCache Redis

```bash
# Create Redis cluster
aws elasticache create-cache-cluster \
  --cache-cluster-id vectorscrub-redis \
  --engine redis \
  --engine-version 7.1 \
  --cache-node-type cache.t4g.medium \
  --num-cache-nodes 1 \
  --security-group-ids <security-group-id> \
  --cache-subnet-group-name vectorscrub-redis-subnet
```

#### Step 4: Create ECR Repositories

```bash
# Create repositories for each service
aws ecr create-repository --repository-name vectorscrub/backend
aws ecr create-repository --repository-name vectorscrub/agents
aws ecr create-repository --repository-name vectorscrub/frontend

# Login to ECR
aws ecr get-login-password --region us-east-1 | docker login --username AWS --password-stdin <account-id>.dkr.ecr.us-east-1.amazonaws.com
```

#### Step 5: Build and Push Images

```bash
cd /Users/pavondunbar/VectorScrub

# Build and push backend
docker build -f infra/docker/Dockerfile.backend -t vectorscrub/backend backend/
docker tag vectorscrub/backend:latest <account-id>.dkr.ecr.us-east-1.amazonaws.com/vectorscrub/backend:latest
docker push <account-id>.dkr.ecr.us-east-1.amazonaws.com/vectorscrub/backend:latest

# Build and push agents
docker build -f infra/docker/Dockerfile.agents -t vectorscrub/agents agents/
docker tag vectorscrub/agents:latest <account-id>.dkr.ecr.us-east-1.amazonaws.com/vectorscrub/agents:latest
docker push <account-id>.dkr.ecr.us-east-1.amazonaws.com/vectorscrub/agents:latest

# Build and push frontend
docker build -f infra/docker/Dockerfile.frontend -t vectorscrub/frontend frontend/
docker tag vectorscrub/frontend:latest <account-id>.dkr.ecr.us-east-1.amazonaws.com/vectorscrub/frontend:latest
docker push <account-id>.dkr.ecr.us-east-1.amazonaws.com/vectorscrub/frontend:latest
```

#### Step 6: Create ECS Cluster and Services

```bash
# Create ECS cluster
aws ecs create-cluster --cluster-name vectorscrub-production

# Create task definitions (see example below)
# Create services for backend, agents, frontend

# Register task definition for backend
aws ecs register-task-definition --cli-input-json file://backend-task-def.json

# Create service
aws ecs create-service \
  --cluster vectorscrub-production \
  --service-name vectorscrub-backend \
  --task-definition vectorscrub-backend:1 \
  --desired-count 2 \
  --launch-type FARGATE \
  --network-configuration "awsvpcConfiguration={subnets=[subnet-xxx,subnet-yyy],securityGroups=[sg-xxx],assignPublicIp=ENABLED}"
```

#### Step 7: Create Application Load Balancer

```bash
# Create ALB
aws elbv2 create-load-balancer \
  --name vectorscrub-alb \
  --subnets subnet-xxx subnet-yyy \
  --security-groups sg-xxx

# Create target groups
aws elbv2 create-target-group \
  --name vectorscrub-backend-tg \
  --protocol HTTP \
  --port 8080 \
  --vpc-id $VPC_ID \
  --target-type ip \
  --health-check-path /api/v1/health/live

# Create listeners with SSL certificate
aws elbv2 create-listener \
  --load-balancer-arn <alb-arn> \
  --protocol HTTPS \
  --port 443 \
  --certificates CertificateArn=<cert-arn> \
  --default-actions Type=forward,TargetGroupArn=<tg-arn>
```

#### Step 8: Store Secrets in AWS Secrets Manager

```bash
# Store database password
aws secretsmanager create-secret \
  --name vectorscrub/production/postgres-password \
  --secret-string <POSTGRES_PASSWORD>

# Store all environment variables
aws secretsmanager create-secret \
  --name vectorscrub/production/env \
  --secret-string file://production-env.json
```

---

## Option 3: Kubernetes (Enterprise)

### For high-scale deployments with auto-scaling, multi-region, etc.

```bash
# Install kubectl and helm
brew install kubectl helm

# Create namespace
kubectl create namespace vectorscrub-production

# Add secrets
kubectl create secret generic vectorscrub-secrets \
  --from-literal=database-url=$DATABASE_URL \
  --from-literal=redis-url=$REDIS_URL \
  --from-literal=jwt-secret=$JWT_SECRET \
  --namespace vectorscrub-production

# Deploy using Helm (chart not included - needs creation)
# See infra/kubernetes/ directory for manifests
```

---

## Post-Deployment

### 1. Run Database Migrations

```bash
# Connect to backend container
docker exec -it vectorscrub-backend sh

# Migrations run automatically on startup via sqlx
# Verify migrations applied:
psql $DATABASE_URL -c "SELECT * FROM _sqlx_migrations;"
```

### 2. Verify Health Checks

```bash
# Liveness
curl https://yourdomain.com/api/v1/health/live

# Readiness
curl https://yourdomain.com/api/v1/health/ready

# Expected: {"status": "healthy"}
```

### 3. Test Authentication Flow

```bash
# Register test user
curl -X POST https://yourdomain.com/api/v1/auth/register \
  -H "Content-Type: application/json" \
  -d '{"email":"test@example.com","password":"TestPassword123!"}'

# Login
curl -X POST https://yourdomain.com/api/v1/auth/login \
  -H "Content-Type: application/json" \
  -d '{"email":"test@example.com","password":"TestPassword123!"}'
```

### 4. Monitor Initial Logs

```bash
# Docker Compose
docker compose logs -f

# AWS ECS
aws logs tail /ecs/vectorscrub-backend --follow

# Kubernetes
kubectl logs -f deployment/vectorscrub-backend -n vectorscrub-production
```

---

## Monitoring & Maintenance

### Prometheus Metrics

```bash
# Metrics endpoint
curl https://yourdomain.com/api/v1/metrics

# Set up Prometheus scraping
# Add to prometheus.yml:
scrape_configs:
  - job_name: 'vectorscrub'
    static_configs:
      - targets: ['backend:8080']
```

### Grafana Dashboards

```bash
# Import recommended dashboard
# See infra/monitoring/grafana-dashboard.json
```

### Automated Backups

```bash
# PostgreSQL daily backups
# Create backup script
cat > /opt/vectorscrub/backup.sh <<'EOF'
#!/bin/bash
set -euo pipefail
BACKUP_DIR=/var/backups/vectorscrub
DATE=$(date +%Y%m%d-%H%M%S)
mkdir -p $BACKUP_DIR

# Backup database
docker exec vectorscrub-postgres pg_dump -U vectorscrub vectorscrub | gzip > $BACKUP_DIR/db-$DATE.sql.gz

# Upload to S3 (optional)
aws s3 cp $BACKUP_DIR/db-$DATE.sql.gz s3://vectorscrub-backups/

# Keep only last 30 days
find $BACKUP_DIR -name "db-*.sql.gz" -mtime +30 -delete
EOF

chmod +x /opt/vectorscrub/backup.sh

# Add to crontab
echo "0 2 * * * /opt/vectorscrub/backup.sh" | crontab -
```

### Log Rotation

```bash
# Configure Docker log rotation
cat > /etc/docker/daemon.json <<EOF
{
  "log-driver": "json-file",
  "log-opts": {
    "max-size": "10m",
    "max-file": "3"
  }
}
EOF

systemctl restart docker
```

---

## Disaster Recovery

### Backup Strategy

1. **Database**: Automated daily backups + point-in-time recovery
2. **Redis**: Not critical (cache/queue only) - can rebuild
3. **Application**: Container images in ECR/registry
4. **Secrets**: Stored in Secrets Manager with versioning

### Recovery Procedures

#### Database Restore

```bash
# From backup
gunzip < db-20261109.sql.gz | docker exec -i vectorscrub-postgres psql -U vectorscrub vectorscrub

# AWS RDS point-in-time restore
aws rds restore-db-instance-to-point-in-time \
  --source-db-instance-identifier vectorscrub-postgres \
  --target-db-instance-identifier vectorscrub-postgres-restored \
  --restore-time 2026-11-09T12:00:00Z
```

#### Full System Restore

1. Provision new infrastructure (VPC, RDS, ElastiCache)
2. Restore database from backup
3. Deploy latest container images
4. Update DNS to point to new load balancer
5. Verify health checks
6. Monitor error logs

---

## Cost Optimization

### AWS Estimated Monthly Costs (Medium Scale)

| Service | Configuration | Monthly Cost |
|---------|---------------|--------------|
| RDS PostgreSQL | db.t4g.medium, 100GB | ~$100 |
| ElastiCache Redis | cache.t4g.medium | ~$50 |
| ECS Fargate | 2 vCPU, 4GB RAM, 3 tasks | ~$150 |
| ALB | Standard | ~$25 |
| Data Transfer | 500GB/month | ~$45 |
| **Total** | | **~$370/month** |

### Optimization Tips

- Use Reserved Instances (40% savings)
- Enable auto-scaling (scale down during low traffic)
- Use S3 Intelligent-Tiering for backups
- Compress logs before storage
- Use CloudFront for static assets

---

## Security Checklist

- [ ] All default passwords changed
- [ ] Secrets stored in secrets manager (not environment variables)
- [ ] HTTPS enforced (HSTS enabled)
- [ ] Database encryption at rest enabled
- [ ] Database connections use SSL/TLS
- [ ] Firewall rules restrict access (only 80, 443 public)
- [ ] SSH key-only authentication (no passwords)
- [ ] Fail2ban installed and configured
- [ ] Regular security updates automated
- [ ] Audit logs enabled and monitored
- [ ] Backup restoration tested monthly
- [ ] Incident response plan documented

---

## Support & Troubleshooting

### Common Issues

**Issue: Database connection timeout**
- Check security group rules
- Verify database is in same VPC
- Confirm DATABASE_URL is correct

**Issue: Redis authentication failed**
- Verify REDIS_PASSWORD matches docker-compose.yml
- Check Redis is running: `redis-cli ping`

**Issue: Agent can't reach backend**
- Verify BACKEND_API_URL is correct
- Check AGENT_API_KEY matches both services
- Ensure network connectivity

### Getting Help

1. Check logs: `docker compose logs -f <service>`
2. Review health endpoints: `/api/v1/health/ready`
3. Check metrics: `/api/v1/metrics`
4. Review audit logs for security events

---

**Next Steps:**
1. Choose deployment option
2. Run `infra/scripts/gen_keys.sh`
3. Store secrets in secrets manager
4. Follow deployment steps for chosen option
5. Verify health checks
6. Set up monitoring
7. Test disaster recovery procedures
