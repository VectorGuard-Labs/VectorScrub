# Runbook: SMTP Email Delivery Failures

## Symptoms
- Email-based removals failing
- Error messages: "Email send failed", "SMTP connection error"
- Metrics show SMTP delivery rate drop
- Customer complaints about not receiving notifications

## Impact
- **Customer Impact:** Email-based opt-outs not sent, notifications not delivered
- **Scope:** Affects email removals (MyLife, etc.) + all notifications
- **Workaround:** Manual email opt-outs
- **Severity:** High (P1) - Notifications critical

## Diagnosis

### 1. Check SMTP Metrics

```bash
# View SMTP dashboard
curl http://localhost:9090/health | grep -A 10 "SMTP"

# Check Redis metrics
redis-cli HGETALL metrics:smtp:deliveries
```

### 2. Test SMTP Connection

```bash
cd agents
python3 << 'EOF'
import smtplib
import ssl
from shared.config import get_config

config = get_config()
context = ssl.create_default_context()

try:
    server = smtplib.SMTP(config.smtp_host, config.smtp_port, timeout=10)
    server.starttls(context=context)
    server.login(config.smtp_username, config.smtp_password)
    print("✓ SMTP connection successful")
    server.quit()
except Exception as e:
    print(f"✗ SMTP connection failed: {type(e).__name__}: {e}")
EOF
```

### 3. Check Logs

```bash
# Agent SMTP logs
tail -f agents/logs/agent.log | grep -i "smtp\|email"

# System mail logs
sudo tail -f /var/log/mail.log
```

## Resolution

### Scenario A: Authentication Failure

**Symptoms:** `SMTPAuthenticationError`, "auth_failures" in metrics

1. **Verify credentials**
```bash
# Check environment variables
echo $SMTP_USERNAME
echo $SMTP_PASSWORD  # Careful - don't log this
```

2. **Test with provider directly**
```bash
# For Gmail
echo "Test" | mail -s "Test" -a "From: $SMTP_USERNAME" test@example.com

# Check if 2FA is enabled
# Check if app-specific password needed
```

3. **Rotate credentials if compromised**
```bash
# Update .env
SMTP_PASSWORD="new_app_specific_password"

# Restart agents
sudo systemctl restart vectorscrub-agents
```

### Scenario B: Connection Failures

**Symptoms:** `SMTPConnectError`, "connection_failures" in metrics

1. **Check network connectivity**
```bash
# Test DNS resolution
nslookup smtp.gmail.com

# Test port connectivity
telnet smtp.gmail.com 587
nc -zv smtp.gmail.com 587
```

2. **Check firewall rules**
```bash
# Outbound SMTP ports (25, 465, 587)
sudo iptables -L OUTPUT -n | grep -E "25|465|587"

# If blocked, allow SMTP
sudo iptables -A OUTPUT -p tcp --dport 587 -j ACCEPT
```

3. **Verify TLS version**
```bash
# Test TLS handshake
openssl s_client -connect smtp.gmail.com:587 -starttls smtp
```

### Scenario C: Rate Limiting

**Symptoms:** 421 errors, "quota exceeded" messages

1. **Check send rate**
```bash
# Emails sent in last hour
redis-cli GET "smtp:hourly_count:$(date +%Y%m%d%H)"
```

2. **Review provider limits**
- Gmail: 500 emails/day, 100 recipients/email
- AWS SES: Depends on sending limits
- SendGrid: Depends on plan

3. **Implement backoff**
```python
# In executor.py
if smtp_error.code == 421:
    await asyncio.sleep(3600)  # Wait 1 hour
    retry
```

### Scenario D: Bounces / Blocklist

**Symptoms:** 550 errors, bounces, "undeliverable"

1. **Check sender reputation**
```bash
# Check if IP is blacklisted
curl "https://mxtoolbox.com/api/v1/lookup/blacklist/$IP_ADDRESS"

# Check SPF/DKIM/DMARC
dig TXT _dmarc.yourdomain.com
dig TXT default._domainkey.yourdomain.com
```

2. **Review bounce rate**
```sql
-- Check bounce patterns
SELECT
    DATE(created_at),
    COUNT(*) FILTER (WHERE bounced = true) as bounces,
    COUNT(*) as total
FROM email_log
GROUP BY DATE(created_at)
ORDER BY DATE(created_at) DESC
LIMIT 30;
```

3. **Clean recipient list**
- Remove hard bounces permanently
- Retry soft bounces with backoff
- Validate email addresses before sending

## Prevention

### Best Practices

1. **Credential Management**
```bash
# Use app-specific passwords
# Rotate every 90 days
# Store in secure secrets manager
```

2. **Monitoring**
```bash
# Alert on high failure rate
if [ $(redis-cli HGET metrics:smtp:deliveries failed) -gt 100 ]; then
    alert "SMTP failure rate critical"
fi
```

3. **Rate Limiting**
```python
# Implement sending rate limiter
class SMTPRateLimiter:
    def __init__(self, max_per_hour=100):
        self.max_per_hour = max_per_hour

    async def wait_if_needed(self):
        # Check hourly quota, sleep if exceeded
        pass
```

4. **Failover Provider**
```python
# Configure backup SMTP provider
SMTP_PROVIDERS = [
    {"host": "smtp.primary.com", "priority": 1},
    {"host": "smtp.backup.com", "priority": 2},
]
```

## Escalation

### When to Escalate

**Escalate to Infrastructure Team if:**
- Network/firewall issues
- IP blocklist issues
- Provider account suspended

**Escalate to Security Team if:**
- Credentials compromised
- Suspicious send patterns
- Spam complaints

### Quick Fix (Emergency)

If SMTP completely down:

```python
# Disable email removals temporarily
# Edit agents/removal/executor.py
async def submit_email_request(...):
    log.error("email_removals_disabled_emergency")
    return {
        "status": "failed",
        "error": "Email removals temporarily unavailable. Try again later."
    }
```

Update status page:
```
Email-based removals are temporarily unavailable due to SMTP provider issues.
Manual opt-out instructions are still available in your dashboard.
We're working to restore service. ETA: 2 hours.
```

## Testing After Resolution

```bash
# Send test email
cd agents
python3 << 'EOF'
from removal.executor import RemovalExecutor
import asyncio

async def test():
    executor = RemovalExecutor()
    # Send test email
    print("Sending test email...")
    # Implementation here

asyncio.run(test())
EOF

# Verify delivery
# Check recipient inbox
# Check SMTP logs for success
```

## Metrics to Monitor

```bash
# SMTP success rate
watch -n 60 'curl -s http://localhost:9090/metrics | grep smtp'

# Email queue depth
redis-cli LLEN "queue:emails:pending"
```
