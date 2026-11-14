# Broker Health Checker Setup Guide

## Overview

The broker health checker automatically verifies all broker opt-out pages are accessible and functioning correctly. It detects:

- ✅ Site downtime (5xx errors)
- ✅ URL changes (404/410 errors)
- ✅ New CAPTCHA implementations
- ✅ SSL/certificate issues
- ✅ Unexpected domain redirects
- ✅ Slow response times

## Quick Start

### Manual Run

```bash
cd agents
python -m monitoring.health_checker

# Or with browser automation (slower but more thorough):
python -m monitoring.health_checker --browser
```

### Automated Nightly Checks

#### 1. Install Cron Job

```bash
# Edit crontab
crontab -e

# Add this line (runs at 2 AM daily):
0 2 * * * /path/to/vectorscrub/scripts/health-check-nightly.sh
```

#### 2. Verify Cron Installation

```bash
# List cron jobs
crontab -l

# Check logs after first run
tail -f logs/health-check-$(date +%Y%m%d).log
```

## Health Check Methods

### HTTP Mode (Default)
- Fast (~30 seconds for all brokers)
- Makes simple GET requests
- Checks HTTP status, redirects, CAPTCHA in HTML

```bash
python -m monitoring.health_checker
```

### Browser Mode (Thorough)
- Slower (~2-3 minutes for all brokers)
- Uses Playwright automation
- More accurate CAPTCHA detection
- Mimics actual removal submissions

```bash
python -m monitoring.health_checker --browser
```

## Understanding Results

### Sample Output

```
================================================================================
BROKER HEALTH CHECK REPORT
Timestamp: 2026-11-13T02:00:15.123456
================================================================================

SUMMARY:
  Total Brokers: 19
  Healthy: 16
  Unhealthy: 3

UNHEALTHY BROKERS:

  ❌ spokeo.com
     URL: https://www.spokeo.com/optout
     Status: 200
     Error: N/A
     ⚠️  CAPTCHA detected

  ❌ mylife.com
     URL: https://www.mylife.com/optout
     Status: 503
     Error: HTTP 503

  ❌ oldbroker.com
     URL: https://www.oldbroker.com/removal
     Status: 404
     Error: HTTP 404

CAPTCHA DETECTED:

  ⚠️  spokeo.com - https://www.spokeo.com/optout

================================================================================
```

### Health Status Meanings

| Status | Meaning | Action Required |
|--------|---------|-----------------|
| ✅ Healthy | Page loads, no CAPTCHA, correct domain | None |
| ❌ CAPTCHA | CAPTCHA detected on page | Mark broker as manual-only |
| ❌ 404/410 | URL not found | Update opt-out URL in registry |
| ❌ 5xx | Server error | Monitor; may be temporary |
| ❌ SSL Error | Certificate issue | Contact broker or update URL |
| ❌ Redirect | Domain changed | Update URL; verify legitimacy |

## Integration with Monitoring

### Redis Storage

Results are automatically stored in Redis:

```bash
# View summary
redis-cli HGETALL health_check:summary

# View specific broker
redis-cli HGETALL health_check:broker:spokeo.com
```

### Metrics Dashboard

Health check results appear in the metrics dashboard:

```
http://localhost:9090/health
```

Look for the "Broker Health Status" section.

### Prometheus Alerts

Add to `prometheus/alerts.yml`:

```yaml
groups:
  - name: vectorscrub_broker_health
    rules:
      - alert: BrokerUnhealthy
        expr: vectorscrub_broker_health_status == 0
        for: 24h
        labels:
          severity: warning
        annotations:
          summary: "Broker {{ $labels.broker }} is unhealthy"

      - alert: BrokerCaptchaDetected
        expr: vectorscrub_broker_has_captcha == 1
        for: 1h
        labels:
          severity: info
        annotations:
          summary: "CAPTCHA detected on {{ $labels.broker }}"
```

## Handling Unhealthy Brokers

### CAPTCHA Detected

1. Verify manually by visiting the URL
2. If CAPTCHA is permanent, mark as manual-only:

```python
# In agents/discovery/agent.py
{
    "name": "spokeo.com",
    "url": "https://www.spokeo.com/search?q={name}",
    "tier": "free",
    "manual": True,  # Add this flag
}
```

3. Document in removal instructions that manual opt-out is required

### 404 Errors

1. Search for new opt-out URL on broker's website
2. Update in `agents/removal/agent.py`:

```python
OPT_OUT_REGISTRY["broker.com"] = RemovalProcedure(
    source_name="broker.com",
    procedure_type="web_form",
    opt_out_url="https://www.broker.com/NEW-URL-HERE",  # Update
    # ...
)
```

3. Regenerate frontend: `cd agents && python -m removal.frontend_export`

### Server Errors (5xx)

1. Monitor for 24-48 hours (may be temporary maintenance)
2. If persistent, contact broker's support
3. Consider marking as unavailable temporarily

### SSL Errors

1. Check certificate expiration: `echo | openssl s_client -connect broker.com:443 -servername broker.com 2>/dev/null | openssl x509 -noout -dates`
2. If expired, contact broker
3. If self-signed/invalid, decide if acceptable (usually not)

### Redirects

1. Verify the redirect is legitimate (not phishing)
2. Update URL to final destination
3. If redirect is to a different company, investigate merger/acquisition

## Alerting Integration

### Slack Notifications

Add to `scripts/health-check-nightly.sh`:

```bash
# After health check runs
if [ $EXIT_CODE -ne 0 ]; then
    curl -X POST -H 'Content-type: application/json' \
        --data '{"text":"VectorScrub health check failed. Check logs."}' \
        "$SLACK_WEBHOOK_URL"
fi
```

### Email Alerts

```bash
# Install mail utility
sudo apt-get install mailutils

# Add to script
if [ $EXIT_CODE -ne 0 ]; then
    cat "$LOG_FILE" | mail -s "VectorScrub Health Check Failed" ops@yourdomain.com
fi
```

### PagerDuty

```bash
# Install pdutil
pip install pdpyras

# Add to script
if [ $EXIT_CODE -ne 0 ]; then
    python - <<EOF
from pdpyras import EventsAPISession
session = EventsAPISession('$PAGERDUTY_API_KEY')
session.trigger('VectorScrub broker health check failed', 'vectorscrub')
EOF
fi
```

## Advanced Configuration

### Custom Concurrency

Edit `monitoring/health_checker.py`:

```python
await checker.check_all_brokers(
    use_browser=False,
    max_concurrent=20,  # Increase for faster checks
)
```

### Custom Timeout

```python
# In health_checker.py
async with httpx.AsyncClient(
    timeout=60.0,  # Increase for slow brokers
) as client:
```

### Selective Checks

Run health check for specific brokers:

```python
# Create custom script
from monitoring.health_checker import BrokerHealthChecker

checker = BrokerHealthChecker()
result = await checker._check_broker_http(
    "spokeo.com",
    "https://www.spokeo.com/optout"
)
print(f"Healthy: {result.is_healthy}")
```

## Troubleshooting

### Health Check Hangs

- Reduce `max_concurrent` parameter
- Increase timeouts
- Check network connectivity to broker domains

### False Positive CAPTCHAs

- Use `--browser` mode for more accurate detection
- Manually verify by visiting URL
- Some brokers show CAPTCHA intermittently (rate limiting)

### Redis Connection Errors

```bash
# Verify Redis is running
redis-cli ping

# Check connection URL
echo $REDIS_URL
```

## Log Retention

Logs are automatically kept for 30 days:

```bash
# View recent logs
ls -lh logs/health-check-*.log

# Manual cleanup
find logs/ -name "health-check-*.log" -mtime +30 -delete
```

## Best Practices

1. **Run nightly** - Catch issues before customers do
2. **Monitor trends** - Increasing failures may indicate systemic issues
3. **Document changes** - Keep notes on broker URL changes
4. **Alert immediately** - Critical brokers should page on-call
5. **Review weekly** - Manual review of health trends
6. **Test after changes** - Run health check after updating broker configs

## API Integration

Programmatic access to health check:

```python
from monitoring.health_checker import run_health_check

# Run check
checker = await run_health_check(use_browser=False)

# Get unhealthy brokers
unhealthy = checker.get_unhealthy_brokers()
for broker in unhealthy:
    print(f"{broker.broker}: {broker.error_message}")

# Get CAPTCHA brokers
captcha = checker.get_captcha_brokers()
```

## Support

For health check issues:
- Check logs in `logs/health-check-*.log`
- Review Redis data: `redis-cli HGETALL health_check:summary`
- Run manual check: `python -m monitoring.health_checker`
