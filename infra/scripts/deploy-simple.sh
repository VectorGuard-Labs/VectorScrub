#!/usr/bin/env bash
# Simple deployment script for VectorScrub on a single VPS
# Usage: sudo ./deploy-simple.sh
#
# What it sets up: Docker Compose services bound to 127.0.0.1 only, behind nginx with a
# Let's Encrypt certificate. Secrets are written to infra/docker/.env (mode 600) and are never
# printed.
set -euo pipefail

echo "=== VectorScrub Simple Deployment ==="
echo ""
echo "This script will deploy VectorScrub using Docker Compose on this server."
echo "Prerequisites:"
echo "  - Docker with the Compose plugin installed"
echo "  - Domain name pointing to this server"
echo "  - Ports 80 and 443 available"
echo ""
read -r -p "Continue? (y/N) " -n 1 REPLY
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
docker compose version >/dev/null 2>&1 || { echo "ERROR: the Docker Compose plugin is not installed"; exit 1; }
command -v openssl >/dev/null 2>&1 || { echo "ERROR: openssl not installed"; exit 1; }

# ---------------------------------------------------------------------------
# Input. Everything typed here ends up in a .env file, an nginx config or a URL, so it is
# validated rather than trusted.
# ---------------------------------------------------------------------------

read -r -p "Enter your domain name (e.g., vectorscrub.com): " DOMAIN
if [[ ! $DOMAIN =~ ^([A-Za-z0-9-]+\.)+[A-Za-z]{2,}$ ]]; then
    echo "ERROR: '$DOMAIN' does not look like a domain name"; exit 1
fi
read -r -p "Enter your email for Let's Encrypt: " EMAIL
if [[ ! $EMAIL =~ ^[^[:space:]@\']+@[^[:space:]@\']+\.[^[:space:]@\']+$ ]]; then
    echo "ERROR: '$EMAIL' does not look like an email address"; exit 1
fi

# A password is embedded in a URL (DATABASE_URL, REDIS_URL), where characters such as / + = @ :
# break parsing. Generated passwords are hex; typed ones must be URL-safe.
URL_SAFE='^[A-Za-z0-9._~-]{16,}$'

echo ""
echo "Database credentials (press Enter to generate a random password; it is saved to the"
echo ".env file and never shown):"
read -r -s -p "PostgreSQL password (16+ characters of A-Z a-z 0-9 . _ ~ -): " POSTGRES_PASSWORD
echo
if [ -z "$POSTGRES_PASSWORD" ]; then
    POSTGRES_PASSWORD=$(openssl rand -hex 24)
    echo "Generated a random PostgreSQL password."
elif [[ ! $POSTGRES_PASSWORD =~ $URL_SAFE ]]; then
    echo "ERROR: the PostgreSQL password must be 16+ characters of A-Z a-z 0-9 . _ ~ -"; exit 1
fi

read -r -s -p "Redis password (16+ characters of A-Z a-z 0-9 . _ ~ -): " REDIS_PASSWORD
echo
if [ -z "$REDIS_PASSWORD" ]; then
    REDIS_PASSWORD=$(openssl rand -hex 24)
    echo "Generated a random Redis password."
elif [[ ! $REDIS_PASSWORD =~ $URL_SAFE ]]; then
    echo "ERROR: the Redis password must be 16+ characters of A-Z a-z 0-9 . _ ~ -"; exit 1
fi

# Generate secrets
echo ""
echo "Generating application secrets..."
JWT_SECRET=$(openssl rand -base64 48)
AGENT_API_KEY=$(openssl rand -base64 36)
# No MASTER_ENCRYPTION_KEY: customer data keys are wrapped by PyHSM (docs/runbooks/key-service.md).
PYHSM_IPC_SECRET=$(openssl rand -hex 32)
# Password of the runtime database role (hex: it is embedded in a URL).
DB_APP_PASSWORD=$(openssl rand -hex 24)

# The PyHSM image must be one you built from a reviewed PyHSM release and pinned by digest.
echo ""
echo "PyHSM holds the key that protects every customer's data. Build its image first (see"
echo "docs/runbooks/key-service.md) and give it here by digest, e.g. pyhsm@sha256:<64 hex digits>."
read -r -p "PyHSM image (by digest): " PYHSM_IMAGE
if [[ ! $PYHSM_IMAGE =~ ^[A-Za-z0-9._/:-]+@sha256:[0-9a-f]{64}$ ]]; then
    echo "ERROR: PYHSM_IMAGE must be a reference with a digest, like pyhsm@sha256:<64 hex digits>"; exit 1
fi

# Typed secrets are read without echo and written single-quoted into .env, so reject the
# characters that would break the quoting.
read_secret() {  # read_secret VARNAME "prompt"
    local value
    read -r -s -p "$2" value
    echo
    if [[ $value == *"'"* || $value == *$'\n'* ]]; then
        echo "ERROR: that value may not contain a single quote"; exit 1
    fi
    printf -v "$1" '%s' "$value"
}

echo ""
read_secret ANTHROPIC_API_KEY "Anthropic API key: "
read -r -p "SMTP host (default: smtp.gmail.com): " SMTP_HOST
SMTP_HOST=${SMTP_HOST:-smtp.gmail.com}
read -r -p "SMTP username: " SMTP_USERNAME
read_secret SMTP_PASSWORD "SMTP password: "
for v in "$SMTP_HOST" "$SMTP_USERNAME"; do
    if [[ $v == *"'"* ]]; then echo "ERROR: values may not contain a single quote"; exit 1; fi
done

# ---------------------------------------------------------------------------
# Application directory
# ---------------------------------------------------------------------------

APP_DIR="/opt/vectorscrub"
mkdir -p "$APP_DIR"
cd "$APP_DIR"

if [ -d ".git" ]; then
    echo "Updating existing repository..."
    git pull
else
    echo "Cloning repository..."
    read -r -p "Git repository URL: " REPO_URL
    git clone -- "$REPO_URL" .
fi

# Create the .env file. umask 077 makes it private to root from the moment it exists (not
# readable-then-chmod'ed), and chmod makes sure of it if the file was already there.
# Only variables docker-compose.yml actually reads are written; it builds DATABASE_URL and
# REDIS_URL from the two passwords itself.
(
    umask 077
    cat > infra/docker/.env <<EOF
# Generated by deploy-simple.sh. Contains secrets: keep mode 600, never commit.
COMPOSE_PROJECT_NAME=vectorscrub
POSTGRES_PASSWORD='$POSTGRES_PASSWORD'
DB_APP_PASSWORD='$DB_APP_PASSWORD'
REDIS_PASSWORD='$REDIS_PASSWORD'
PYHSM_IMAGE='$PYHSM_IMAGE'
PYHSM_IPC_SECRET='$PYHSM_IPC_SECRET'
PYHSM_PASSWORD_FILE='$APP_DIR/secrets/pyhsm-password'

# Application
ENVIRONMENT=production
ALLOWED_ORIGINS='https://$DOMAIN,https://www.$DOMAIN'
# nginx (below) overwrites X-Forwarded-For with the real client address, so the backend can
# rate-limit and audit per client. Without this every visitor shares nginx's address.
TRUST_PROXY_HEADERS=true

# Security
JWT_SECRET='$JWT_SECRET'
AGENT_API_KEY='$AGENT_API_KEY'

# LLM
ANTHROPIC_API_KEY='$ANTHROPIC_API_KEY'
DEFAULT_LLM_PROVIDER=anthropic

# SMTP (used for email opt-out requests)
SMTP_HOST='$SMTP_HOST'
SMTP_PORT=587
SMTP_USERNAME='$SMTP_USERNAME'
SMTP_PASSWORD='$SMTP_PASSWORD'
SMTP_FROM_EMAIL='noreply@$DOMAIN'
SMTP_USE_TLS=true
EOF
)
chmod 600 infra/docker/.env

# The PyHSM keystore password: a file, mode 600 (PyHSM refuses anything wider). Created once and
# never overwritten: replacing it would lock the keystore.
mkdir -p "$APP_DIR/secrets"
chmod 700 "$APP_DIR/secrets"
if [ ! -e "$APP_DIR/secrets/pyhsm-password" ]; then
    ( umask 077; openssl rand -base64 36 | tr -d '\n/+=' | cut -c1-40 > "$APP_DIR/secrets/pyhsm-password" )
    echo "Created $APP_DIR/secrets/pyhsm-password. Copy it to your password manager NOW: without it the keystore, and so all customer data, cannot be opened."
fi
chmod 600 "$APP_DIR/secrets/pyhsm-password"

# Start services. Only nginx (below) is reachable from the network: no container publishes a port
# except on 127.0.0.1. First start follows docs/runbooks/key-service.md: give the PyHSM volumes to
# uid 1000, start PyHSM, the database and Redis, create the key, then start everything.
echo ""
echo "Building and starting services (the first build takes several minutes)..."
cd infra/docker
docker compose down 2>/dev/null || true
docker volume create vectorscrub_pyhsm_data >/dev/null
docker volume create vectorscrub_pyhsm_socket >/dev/null
docker run --rm -v vectorscrub_pyhsm_data:/d -v vectorscrub_pyhsm_socket:/s alpine \
    sh -c 'chown 1000:1000 /d /s && chmod 750 /d && chmod 700 /s'
docker compose up -d --build pyhsm postgres redis migrate
docker compose run --rm --no-deps backend kms status >/dev/null 2>&1 || docker compose run --rm --no-deps backend kms init
docker compose up -d --build --wait --wait-timeout 600
docker compose ps
echo ""
echo "NEXT, before any customer signs up: back up the PyHSM keystore (docs/runbooks/key-service.md, step 4)."

# ---------------------------------------------------------------------------
# Nginx + TLS
# ---------------------------------------------------------------------------

echo ""
echo "Installing Nginx reverse proxy..."
apt-get update
apt-get install -y nginx certbot python3-certbot-nginx

# X-Forwarded-For and X-Real-IP are SET to the connecting address, not appended to: a client
# cannot smuggle its own value through, which is what makes TRUST_PROXY_HEADERS=true safe.
cat > /etc/nginx/sites-available/vectorscrub <<EOF
server {
    listen 80;
    server_name $DOMAIN www.$DOMAIN;
    client_max_body_size 1m;

    location / {
        proxy_pass http://127.0.0.1:3000;
        proxy_set_header Host \$host;
        proxy_set_header X-Real-IP \$remote_addr;
        proxy_set_header X-Forwarded-For \$remote_addr;
        proxy_set_header X-Forwarded-Proto \$scheme;
        proxy_buffering off;
    }

    location /api/ {
        proxy_pass http://127.0.0.1:8080;
        proxy_set_header Host \$host;
        proxy_set_header X-Real-IP \$remote_addr;
        proxy_set_header X-Forwarded-For \$remote_addr;
        proxy_set_header X-Forwarded-Proto \$scheme;
    }

    # Prometheus metrics are unauthenticated: not for the internet.
    location = /api/v1/metrics {
        allow 127.0.0.1;
        deny all;
        proxy_pass http://127.0.0.1:8080;
    }
}
EOF
chmod 644 /etc/nginx/sites-available/vectorscrub

ln -sf /etc/nginx/sites-available/vectorscrub /etc/nginx/sites-enabled/
rm -f /etc/nginx/sites-enabled/default
nginx -t && systemctl reload nginx

# Get SSL certificate
echo ""
echo "Obtaining SSL certificate from Let's Encrypt..."
certbot --nginx -d "$DOMAIN" -d "www.$DOMAIN" --non-interactive --agree-tos --email "$EMAIL"

# Set up auto-renewal
systemctl enable certbot.timer
systemctl start certbot.timer

# ---------------------------------------------------------------------------
# Backups
# ---------------------------------------------------------------------------

echo ""
echo "Setting up automated backups..."
install -d -m 700 /var/backups/vectorscrub
cat > "$APP_DIR/backup.sh" <<'BACKUP_EOF'
#!/bin/bash
set -euo pipefail
umask 077
BACKUP_DIR=/var/backups/vectorscrub
DATE=$(date +%Y%m%d-%H%M%S)
install -d -m 700 "$BACKUP_DIR"

# Backup database
docker exec vectorscrub-postgres pg_dump -U vectorscrub vectorscrub | gzip > "$BACKUP_DIR/db-$DATE.sql.gz"

# Keep only last 30 days
find "$BACKUP_DIR" -name "db-*.sql.gz" -mtime +30 -delete

echo "Backup completed: db-$DATE.sql.gz"
BACKUP_EOF

chmod 700 "$APP_DIR/backup.sh"

# Add to crontab
(crontab -l 2>/dev/null; echo "0 2 * * * $APP_DIR/backup.sh >> /var/log/vectorscrub-backup.log 2>&1") | crontab -

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
echo "Secrets are in $APP_DIR/infra/docker/.env (mode 600, readable by root only)."
echo "Backups: daily at 2 AM to /var/backups/vectorscrub (mode 700)."
echo "  NOTE: backups are plain database dumps on this same server. They are not encrypted"
echo "  and do not survive losing the server; copy them somewhere else."
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
