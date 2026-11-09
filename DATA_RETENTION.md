# Data Retention Policy

VectorScrub implements automated data retention to comply with GDPR, CCPA, and SOC 2 requirements.

## Retention Periods

| Data Type | Retention Period | Justification |
|-----------|------------------|---------------|
| **Active Customer Accounts** | Indefinite | Until customer closes account |
| **Inactive Customer Accounts** | 2 years | GDPR requirement, then deleted |
| **Customer PII** | Account lifetime + 90 days | Grace period for reactivation |
| **Active Exposures** | Until removed + 90 days | Verification period |
| **Removed Exposures** | 90 days after removal | Compliance verification |
| **Scan History** | 1 year | Historical tracking |
| **Security Audit Logs** | 7 years | SOC 2 / compliance requirement |
| **Application Logs** | 90 days | Debugging and troubleshooting |
| **Backups (Daily)** | 30 days | Operational recovery |
| **Backups (Monthly)** | 7 years | Compliance requirement |

## Automated Cleanup

### Schedule

Automated cleanup runs **daily at 2:00 AM UTC**.

```rust
// In main.rs
use services::data_retention::{schedule_retention_cleanup, RetentionConfig};

// Start cleanup scheduler (runs every 24 hours)
tokio::spawn(schedule_retention_cleanup(
    pool.clone(),
    RetentionConfig::default(),
    24, // hours
));
```

### What Gets Deleted

#### 1. Inactive Customer Warning (2 years)

Customers who haven't logged in for 2 years receive a warning email:

```
Subject: VectorScrub Account Inactivity Notice

We noticed you haven't logged into your VectorScrub account in 2 years.

If you don't log in within 30 days, your account and all associated data
will be permanently deleted in accordance with our data retention policy.

To keep your account active, simply log in at: https://vectorscrub.com/login
```

#### 2. Inactive Customer Deletion (2 years + 1 month)

After 2 years and 1 month of inactivity, accounts are automatically deleted:

- Customer profile
- Identity information (PII)
- Scan history
- Exposures
- Removal requests
- Notifications

**Note:** Audit logs are preserved but pseudonymized (customer_id set to NULL).

#### 3. Old Exposures (90 days after removal)

Exposures that have been removed for 90+ days are deleted:

```rust
DELETE FROM exposures
WHERE is_active = false
  AND updated_at < NOW() - INTERVAL '90 days'
```

This allows time to verify successful removal.

#### 4. Old Scan Jobs (1 year)

Scan history older than 1 year is deleted:

```rust
DELETE FROM scan_jobs
WHERE created_at < NOW() - INTERVAL '1 year'
```

Recent scan history is preserved for trending and analytics.

#### 5. Application Logs (90 days)

Non-security logs older than 90 days are deleted:

```rust
DELETE FROM audit_log
WHERE timestamp < NOW() - INTERVAL '90 days'
  AND action NOT LIKE 'user.%'
  AND action NOT LIKE 'security.%'
```

Security-related logs are kept for 7 years.

## Manual Deletion (GDPR Right to Deletion)

### User-Initiated Deletion

Users can delete their account at any time:

```typescript
// Frontend: Account Settings
async function deleteAccount() {
  if (confirm('Are you sure? This cannot be undone.')) {
    await fetch('/api/v1/me', {
      method: 'DELETE',
      headers: { Authorization: `Bearer ${token}` },
    });

    // Logout and redirect
    localStorage.removeItem('vs_token');
    window.location.href = '/';
  }
}
```

### Admin-Initiated Deletion

Support can manually delete accounts upon request:

```rust
use crate::services::data_retention::DataRetentionService;

let service = DataRetentionService::new(pool, config);
service.delete_customer_data(customer_id).await?;
```

### Deletion Process

1. **Verification** (24 hours)
   - Email confirmation sent
   - User must click link to confirm
   - Grace period for accidental deletion

2. **Soft Delete**
   - Account marked inactive
   - Login disabled
   - Data not yet deleted

3. **Hard Delete** (30 days)
   - All customer data permanently deleted
   - Audit logs pseudonymized
   - Backups eventually age out

## Data Minimization

### PII Minimization

VectorScrub only stores necessary PII:

**Stored:**
- Name (for identity matching)
- Current address (for detection)
- Phone/email (for contact)
- Date of birth (optional, for matching)

**NOT Stored:**
- SSN
- Credit card numbers
- Government IDs
- Biometric data

### Encryption at Rest

All PII is encrypted using AES-256-GCM:

```rust
// Encrypted before storage
let encrypted_name = encrypt_data(
    &customer.full_name,
    &customer_key,
    &nonce
)?;

// Store encrypted
sqlx::query!(
    "INSERT INTO identity_profiles (full_name_enc) VALUES ($1)",
    encrypted_name
).execute(&pool).await?;
```

### Access Logging

All PII access is logged:

```sql
INSERT INTO audit_log (
    customer_id,
    action,
    user_email,
    timestamp
) VALUES (
    $1,
    'pii.access',
    $2,
    NOW()
);
```

## Backup Retention

### Short-Term Backups

**Daily Backups:**
- Frequency: Every 24 hours
- Retention: 30 days
- Location: Same AWS region
- Encryption: AES-256

**Purpose:** Operational recovery (accidental deletion, corruption)

### Long-Term Backups

**Monthly Backups:**
- Frequency: First of each month
- Retention: 7 years
- Location: Separate AWS region
- Encryption: AES-256

**Purpose:** Compliance and legal requirements

### Backup Deletion

Automated backup cleanup:

```bash
# Cron job (daily at 3:00 AM UTC)
0 3 * * * /usr/local/bin/cleanup-backups.sh

# Script
#!/bin/bash
# Delete daily backups older than 30 days
aws s3 rm s3://vectorscrub-backups/daily/ \
  --recursive \
  --exclude "*" \
  --include "*" \
  --older-than "30 days"

# Delete monthly backups older than 7 years
aws s3 rm s3://vectorscrub-backups/monthly/ \
  --recursive \
  --exclude "*" \
  --include "*" \
  --older-than "2555 days"
```

## Monitoring

### Metrics

Track retention cleanup in Prometheus:

```prometheus
# Cleanup runs
data_retention_cleanup_runs_total 142

# Customers deleted
data_retention_customers_deleted_last_run 5

# Exposures deleted
data_retention_exposures_deleted_last_run 1234

# Failures
data_retention_cleanup_failures_total 0
```

### Alerts

```yaml
groups:
  - name: data_retention
    rules:
      # Alert if cleanup hasn't run in 48 hours
      - alert: DataRetentionCleanupOverdue
        expr: time() - data_retention_last_run_timestamp > 172800
        annotations:
          summary: "Data retention cleanup hasn't run in 48 hours"

      # Alert on cleanup failures
      - alert: DataRetentionCleanupFailures
        expr: rate(data_retention_cleanup_failures_total[1h]) > 0
        annotations:
          summary: "Data retention cleanup is failing"
```

### Logs

All retention operations are logged:

```json
{
  "timestamp": "2026-11-09T02:00:00Z",
  "level": "info",
  "message": "Data retention cleanup complete",
  "customers_warned": 12,
  "customers_deleted": 5,
  "exposures_deleted": 234,
  "scans_deleted": 1523,
  "audit_logs_deleted": 0,
  "duration_ms": 2341
}
```

## Configuration

### Environment Variables

```bash
# Enable/disable automated cleanup
DATA_RETENTION_ENABLED=true

# Cleanup schedule (hours)
DATA_RETENTION_INTERVAL_HOURS=24

# Retention periods (days)
RETENTION_INACTIVE_WARNING_DAYS=730      # 2 years
RETENTION_INACTIVE_DELETION_DAYS=760     # 2 years + 1 month
RETENTION_REMOVED_EXPOSURE_DAYS=90       # 90 days
RETENTION_SCAN_HISTORY_DAYS=365          # 1 year
RETENTION_APPLICATION_LOG_DAYS=90        # 90 days
RETENTION_AUDIT_LOG_DAYS=2555            # 7 years
```

### Custom Configuration

```rust
use crate::services::data_retention::RetentionConfig;

let config = RetentionConfig {
    inactive_warning_days: 365,     // 1 year instead of 2
    inactive_deletion_days: 395,    // 1 year + 1 month
    removed_exposure_days: 60,      // 60 days instead of 90
    scan_history_days: 180,         // 6 months instead of 1 year
    application_log_days: 30,       // 30 days instead of 90
    audit_log_days: 2555,           // Keep 7 years (required)
};
```

## Testing

### Manual Test

```bash
# Run cleanup manually
curl -X POST http://localhost:8080/api/v1/internal/cleanup/run \
  -H "X-Agent-Key: $AGENT_API_KEY"

# Response
{
  "inactive_customers_warned": 12,
  "inactive_customers_deleted": 5,
  "exposures_deleted": 234,
  "scans_deleted": 1523,
  "audit_logs_deleted": 0
}
```

### Integration Test

```rust
#[tokio::test]
async fn test_data_retention_cleanup() {
    let pool = setup_test_db().await;
    let config = RetentionConfig {
        inactive_deletion_days: 1, // 1 day for testing
        ..Default::default()
    };

    let service = DataRetentionService::new(pool.clone(), config);

    // Create inactive customer
    let customer_id = create_test_customer(&pool).await;
    set_last_scan_date(&pool, customer_id, 2).await; // 2 days ago

    // Run cleanup
    let stats = service.run_cleanup().await.unwrap();

    // Verify customer was deleted
    assert_eq!(stats.inactive_customers_deleted, 1);

    let customer = get_customer(&pool, customer_id).await;
    assert!(customer.is_none());
}
```

## Compliance

### GDPR Compliance

✅ **Right to Deletion (Art. 17)**
- User-initiated deletion available
- Data deleted within 30 days
- Confirmation email sent

✅ **Data Minimization (Art. 5)**
- Only necessary PII stored
- Encrypted at rest
- Automatic deletion of old data

✅ **Storage Limitation (Art. 5)**
- Retention periods defined
- Automated cleanup enforced
- Audit trail maintained

### CCPA Compliance

✅ **Right to Delete (§1798.105)**
- Deletion request form available
- Confirmation within 10 days
- Deletion within 45 days

✅ **Data Inventory**
- All data categories documented
- Retention periods published
- Deletion process documented

### SOC 2 Compliance

✅ **Data Classification**
- PII clearly identified
- Retention periods defined
- Access controls enforced

✅ **Data Disposal**
- Automated deletion
- Secure erasure
- Audit logging

## FAQs

### Q: What happens to my data if I don't log in for 2 years?

A: After 2 years of inactivity:
1. You receive a warning email
2. If you don't log in within 30 days, your account and data are deleted
3. Audit logs are kept but anonymized

### Q: Can I recover deleted data?

A: No. Once data is deleted (hard delete after 30 days), it cannot be recovered.

Daily backups are kept for 30 days, so recent deletions within that window may be recoverable by support.

### Q: How long are my scan results kept?

A: Scan history is kept for 1 year. Older scans are automatically deleted.

### Q: Are audit logs ever deleted?

A: Security-related audit logs are kept for 7 years (compliance requirement).
Non-security logs are kept for 90 days.

### Q: Can I request my data be deleted immediately?

A: Yes. Go to Account Settings → Delete Account. Data will be deleted within 30 days.

---

**Last Updated:** 2026-11-09
**Policy Owner:** privacy@vectorscrub.com
