#!/usr/bin/env bash
# Generate cryptographic secrets for VectorScrub.
# Run once during initial setup. Store output in your secrets manager.
set -euo pipefail

echo "=== VectorScrub Key Generation ==="
echo ""
echo "JWT_SECRET (copy to .env):"
openssl rand -base64 48

echo ""
echo "MASTER_ENCRYPTION_KEY (copy to .env — 32 bytes base64):"
openssl rand -base64 32

echo ""
echo "POSTGRES_PASSWORD:"
openssl rand -base64 24

echo ""
echo "REDIS_PASSWORD:"
openssl rand -base64 24

echo ""
echo "WARNING: Store these values in a secrets manager (AWS Secrets Manager,"
echo "         HashiCorp Vault, etc.). Never commit them to version control."
