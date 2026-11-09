#!/usr/bin/env bash
# Simple deployment script for VectorScrub on a single VPS
# Usage: ./deploy-simple.sh
set -euo pipefail

echo "=== VectorScrub Simple Deployment ==="
echo ""
echo "This script will deploy VectorScrub using Docker Compose on this server."
echo "Prerequisites:"
echo "  - Docker and Docker Compose installed"
echo "  - Domain name pointing to this server"
echo "  - Ports 80 and 443 available"
echo ""
read -p "Continue? (y/N) " -n 1 -r
echo
if [[ ! $REPLY =~ ^[Yy]$ ]]; then
    echo "Deployment cancelled."
    exit 1
fi

# Check if running as root
if [ "$EUID" -ne 0 ]; then
    echo "ERROR: This script must be run as root (or with sudo)"
    exit 1
fi

# Check prerequisites
command -v docker >/dev/null 2>&1 || { echo "ERROR: docker not installed"; exit 1; }
command -v docker-compose >/dev/null 2>&1 || command -v docker compose version >/dev/null 2>&1 || { echo "ERROR: docker-compose not installed"; exit 1; }

# Get domain name
read -p "Enter your domain name (e.g., vectorscrub.com): " DOMAIN
read -p "Enter your email for Let's Encrypt: " EMAIL

# Get database credentials
echo ""
echo "Database credentials (or press Enter to generate random):"
read -sp "PostgreSQL password: " POSTGRES_PASSWORD
echo
if [ -z "$POSTGRES_PASSWORD" ]; then
    POSTGRES_PASSWORD=$(openssl rand -base64 24)
    echo "Generated PostgreSQL password: $POSTGRES_PASSWORD"
fi

read -sp "Redis password: " REDIS_PASSWORD
echo
if [ -z "$REDIS_PASSWORD" ]; then
    REDIS_PASSWORD=$(openssl rand -base64 24)
    echo "Generated Redis password: $REDIS_PASSWORD"
fi

# Generate secrets
echo ""
echo "Generating application secrets..."
JWT_SECRET=$(openssl rand -base64 48)
MASTER_ENCRYPTION_KEY=$(openssl rand -base64 32)
AGENT_API_KEY=$(openssl rand -base64 32)

# Get API keys
echo ""
read -p "Anthropic API key: " ANTHROPIC_API_KEY
read -p "SMTP host (default: smtp.gmail.com): " SMTP_HOST
SMTP_HOST=${SMTP_HOST:-smtp.gmail.com}
read -p "SMTP username: " SMTP_USERNAME
read -sp "SMTP password: " SMTP_PASSWORD
echo

# Create application directory
APP_DIR="/opt/vectorscrub"
mkdir -p $APP_DIR
cd $APP_DIR

# Clone or update repository
if [ -d ".git" ]; then
    echo "Updating existing repository..."
    git pull
else
    echo "Cloning repository..."
    read -p "Git repository URL: " REPO_URL
    git clone $REPO_URL .
fi

# Create .env file
cat > infra/docker/.env <<EOF
# Database
POSTGRES_PASSWORD=$POSTGRES_PASSWORD
DATABASE_URL=postgres://vectorscrub:$POSTGRES_PASSWORD@postgres:5432/vectorscrub

# Redis
REDIS_PASSWORD=$REDIS_PASSWORD

# Application
ENVIRONMENT=production
PORT=8080
ALLOWED_ORIGINS=https://$DOMAIN,https://www.$DOMAIN

# Security
JWT_SECRET=$JWT_SECRET
MASTER_ENCRYPTION_KEY=$MASTER_ENCRYPTION_KEY
AGENT_API_KEY=$AGENT_API_KEY

# LLM
ANTHROPIC_API_KEY=$ANTHROPIC_API_KEY
DEFAULT_LLM_PROVIDER=anthropic
DEFAULT_LLM_MODEL=claude-3-5-sonnet-20241022

# SMTP
SMTP_HOST=$SMTP_HOST
SMTP_PORT=587
SMTP_USERNAME=$SMTP_USERNAME
SMTP_PASSWORD=$SMTP_PASSWORD
SMTP_FROM_EMAIL=noreply@$DOMAIN
SMTP_USE_TLS=true

# Networking
BACKEND_API_URL=http://backend:8080
REDIS_URL=redis://:$REDIS_PASSWORD@redis:6379
NEXT_PUBLIC_API_URL=https://$DOMAIN
EOF

# Update docker-compose.yml with passwords
cd infra/docker
sed -i "s/changeme_in_production/$POSTGRES_PASSWORD/g" docker-compose.yml
sed -i "s/\${REDIS_PASSWORD:-changeme_in_production}/$REDIS_PASSWORD/g" docker-compose.yml

# Start services
echo ""
echo "Starting services..."
docker compose down 2>/dev/null || true
docker compose up -d

# Wait for services to be healthy
echo "Waiting for services to be healthy..."
sleep 10

# Check health
docker compose ps

# Install Nginx
echo ""
echo "Installing Nginx reverse proxy..."
apt update
apt install -y nginx certbot python3-certbot-nginx

# Create Nginx config
cat > /etc/nginx/sites-available/vectorscrub <<EOF
server {
    listen 80;
    server_name $DOMAIN www.$DOMAIN;

    location / {
        proxy_pass http://127.0.0.1:3000;
        proxy_set_header Host \$host;
        proxy_set_header X-Real-IP \$remote_addr;
        proxy_set_header X-Forwarded-For \$proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto \$scheme;
        proxy_buffering off;
    }

    location /api/ {
        proxy_pass http://127.0.0.1:8080;
        proxy_set_header Host \$host;
        proxy_set_header X-Real-IP \$remote_addr;
        proxy_set_header X-Forwarded-For \$proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto \$scheme;
    }

    location /api/v1/metrics {
        proxy_pass http://127.0.0.1:8080;
        allow 127.0.0.1;
        deny all;
    }
}
EOF

ln -sf /etc/nginx/sites-available/vectorscrub /etc/nginx/sites-enabled/
rm -f /etc/nginx/sites-enabled/default
nginx -t && systemctl reload nginx

# Get SSL certificate
echo ""
echo "Obtaining SSL certificate from Let's Encrypt..."
certbot --nginx -d $DOMAIN -d www.$DOMAIN --non-interactive --agree-tos --email $EMAIL

# Set up auto-renewal
systemctl enable certbot.timer
systemctl start certbot.timer

# Set up backups
echo ""
echo "Setting up automated backups..."
cat > /opt/vectorscrub/backup.sh <<'BACKUP_EOF'
#!/bin/bash
set -euo pipefail
BACKUP_DIR=/var/backups/vectorscrub
DATE=$(date +%Y%m%d-%H%M%S)
mkdir -p $BACKUP_DIR

# Backup database
docker exec vectorscrub-postgres pg_dump -U vectorscrub vectorscrub | gzip > $BACKUP_DIR/db-$DATE.sql.gz

# Keep only last 30 days
find $BACKUP_DIR -name "db-*.sql.gz" -mtime +30 -delete

echo "Backup completed: db-$DATE.sql.gz"
BACKUP_EOF

chmod +x /opt/vectorscrub/backup.sh

# Add to crontab
(crontab -l 2>/dev/null; echo "0 2 * * * /opt/vectorscrub/backup.sh >> /var/log/vectorscrub-backup.log 2>&1") | crontab -

# Print summary
echo ""
echo "=== Deployment Complete ==="
echo ""
echo "VectorScrub is now running at: https://$DOMAIN"
echo ""
echo "Services:"
echo "  Frontend: https://$DOMAIN"
echo "  Backend API: https://$DOMAIN/api"
echo "  Health Check: https://$DOMAIN/api/v1/health/live"
echo ""
echo "Credentials saved to: $APP_DIR/infra/docker/.env"
echo "Backups: Daily at 2 AM to /var/backups/vectorscrub"
echo ""
echo "Next steps:"
echo "  1. Test the deployment: curl https://$DOMAIN/api/v1/health/live"
echo "  2. Create your first user account"
echo "  3. Set up monitoring (see DEPLOYMENT.md)"
echo "  4. Review logs: docker compose logs -f"
echo ""
echo "To manage the application:"
echo "  cd $APP_DIR/infra/docker"
echo "  docker compose logs -f     # View logs"
echo "  docker compose restart     # Restart services"
echo "  docker compose down        # Stop services"
echo ""
