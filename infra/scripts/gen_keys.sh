#!/usr/bin/env bash
# Generate cryptographic secrets for VectorScrub.
# Run once during initial setup. Store output in your secrets manager.
set -euo pipefail

echo "=== VectorScrub Key Generation ==="
echo ""
echo "JWT_SECRET (copy to .env — the backend refuses to start with fewer than 32 characters):"
openssl rand -base64 48

echo ""
echo "MASTER_ENCRYPTION_KEY (copy to .env — 32 bytes base64):"
openssl rand -base64 32

echo ""
echo "AGENT_API_KEY (copy to .env — shared secret between the backend and the agents, 32+ chars):"
openssl rand -base64 36

echo ""
echo "POSTGRES_PASSWORD (hex on purpose: it is embedded in a URL, where base64's / + = would break parsing):"
openssl rand -hex 24

echo ""
echo "REDIS_PASSWORD (hex, same reason):"
openssl rand -hex 24

echo ""
echo "WARNING: Store these values in a secrets manager (AWS Secrets Manager,"
echo "         HashiCorp Vault, etc.). Never commit them to version control."
