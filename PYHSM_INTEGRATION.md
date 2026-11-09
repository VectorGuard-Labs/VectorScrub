# PyHSM Integration Guide

VectorScrub integrates with [PyHSM](https://github.com/VectorGuard-Labs/PyHSM) for secure key management in production environments.

## Why PyHSM?

Instead of storing the master encryption key in environment variables, PyHSM provides:

✅ **Double-encrypted key storage** - Keys are wrapped with AES-KWP and stored in encrypted keystore
✅ **Automatic key rotation** - Keys rotate every 90 days by default
✅ **Audit logging** - HMAC-chained tamper-evident audit trail
✅ **Memory-hard KDF** - Argon2id with 64 MB memory requirement
✅ **Per-key policies** - Expiry, rate limiting, operation restrictions
✅ **Zero-knowledge security** - Key material is zeroized after use

## Installation

PyHSM is already included in `agents/requirements.txt`:

```bash
cd agents
pip install -r requirements.txt
```

## Configuration

### Option 1: Environment Variables (Recommended)

Set these in your `.env` file:

```bash
# PyHSM configuration
HSM_MASTER_PASSWORD=your-strong-password-here
HSM_STORAGE_PATH=./keystore.enc  # Optional, defaults to ./keystore.enc
```

**Important:** Remove or comment out `MASTER_ENCRYPTION_KEY` when using PyHSM.

### Option 2: Manual Initialization

For advanced use cases, you can manually initialize PyHSM:

```python
from orchestrator.hsm_service import HSMService

hsm = HSMService(
    storage_path="./keystore.enc",
    master_password="your-password"
)

# Get master key for backend
master_key_b64 = hsm.get_master_key_base64()

# Close when done
hsm.close()
```

## First-Time Setup

1. **Generate a strong master password:**

```bash
openssl rand -base64 32
```

2. **Update `.env`:**

```bash
HSM_MASTER_PASSWORD=<generated-password>
```

3. **Initialize the keystore:**

The first time you run the orchestrator, PyHSM will automatically:
- Create the keystore file at `HSM_STORAGE_PATH`
- Generate the master encryption key
- Set up automatic rotation (every 90 days)

4. **Verify initialization:**

```bash
python -m orchestrator.orchestrator
```

You should see log output indicating PyHSM initialization.

## Key Operations

### Generate Customer-Specific Keys

```python
from orchestrator.hsm_service import get_hsm

hsm = get_hsm()

# Generate derived key for customer
customer_key_name = hsm.generate_derived_key("customer-uuid-here")

# Use the key
ciphertext = hsm.encrypt(customer_key_name, b"sensitive data")
plaintext = hsm.decrypt(customer_key_name, ciphertext)
```

### Manual Key Rotation

```python
from orchestrator.hsm_service import get_hsm

hsm = get_hsm()

# Rotate master key immediately (emergency rotation)
hsm.rotate_master_key()
```

### Backup Keystore

```bash
python -c "from orchestrator.hsm_service import get_hsm; get_hsm().backup_keystore('keystore-backup.enc')"
```

Store backups in a secure location (e.g., encrypted S3 bucket, offline storage).

## Production Deployment

### Docker

1. Mount keystore as a persistent volume:

```yaml
services:
  orchestrator:
    volumes:
      - keystore_data:/app/keystore.enc
    environment:
      - HSM_MASTER_PASSWORD=${HSM_MASTER_PASSWORD}
      - HSM_STORAGE_PATH=/app/keystore.enc

volumes:
  keystore_data:
```

2. **Never commit the keystore** to version control. Add to `.gitignore`:

```gitignore
keystore.enc
keystore-*.enc
*.keystore
```

### Key Rotation Schedule

PyHSM automatically rotates keys every 90 days. To change this:

```python
hsm.hsm.update_key_policy("vectorscrub-master-key", {
    "rotate_every_days": 60  # Rotate every 60 days
})
```

### Monitoring

PyHSM exports metrics to Prometheus:

- `hsm_key_operations_total` - Total key operations
- `hsm_key_rotation_timestamp` - Last rotation timestamp
- `hsm_audit_log_entries_total` - Audit log entries

### Security Checklist

- [ ] Master password is 32+ characters from `openssl rand`
- [ ] Keystore file has `chmod 600` permissions
- [ ] Backups are encrypted and stored securely
- [ ] Automatic rotation is enabled (90 days)
- [ ] Audit logs are monitored for suspicious activity
- [ ] Non-production keystores are never used in production

## Troubleshooting

### "HSM_MASTER_PASSWORD environment variable must be set"

Ensure `.env` file contains `HSM_MASTER_PASSWORD`:

```bash
grep HSM_MASTER_PASSWORD .env
```

### "Invalid master password"

The keystore exists but the password is incorrect. If you lost the password:
1. Delete the keystore file: `rm keystore.enc`
2. Restart the orchestrator to create a new keystore
3. **Warning:** This will invalidate all existing encrypted data

### Keystore corrupted

Restore from backup:

```bash
cp keystore-backup.enc keystore.enc
```

If no backup exists, you'll need to re-encrypt all customer data with a new master key.

## Migration from Environment Variable

If you're currently using `MASTER_ENCRYPTION_KEY`:

1. **Don't delete the old key yet** - you need it for migration
2. Set `HSM_MASTER_PASSWORD` in `.env`
3. Run migration script (coming soon)
4. Verify all data decrypts correctly
5. Remove `MASTER_ENCRYPTION_KEY` from `.env`

## Advanced: Process Isolation

For maximum security, run PyHSM as a separate process with Unix socket IPC:

```python
from hsm import PyHSMServer

# Start HSM server
server = PyHSMServer(
    storage_path="keystore.enc",
    master_password="your-password",
    socket_path="/tmp/pyhsm.sock",
    auth_token="your-auth-token"
)
server.start()

# In orchestrator, connect to server
from hsm import PyHSMClient

hsm = PyHSMClient(
    socket_path="/tmp/pyhsm.sock",
    auth_token="your-auth-token"
)
```

This isolates key material in a separate process with minimal attack surface.

## Further Reading

- [PyHSM GitHub Repository](https://github.com/VectorGuard-Labs/PyHSM)
- [PyHSM Documentation](https://github.com/VectorGuard-Labs/PyHSM/wiki)
- [OWASP Key Management Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/Key_Management_Cheat_Sheet.html)

---

**Last Updated:** 2026-11-09
