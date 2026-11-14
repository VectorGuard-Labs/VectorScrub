# Runbook: CAPTCHA Detection on Broker

## Symptoms
- Removal requests failing for specific broker
- Error message: "CAPTCHA challenge on the opt-out page"
- Health check reports CAPTCHA detected
- Success rate drops suddenly for broker

## Impact
- **Customer Impact:** Cannot complete automated removals
- **Scope:** Single broker affected
- **Workaround:** Manual opt-out instructions available
- **Severity:** Medium (P2)

## Diagnosis

### 1. Verify CAPTCHA Presence

```bash
# Run health check for specific broker
cd agents
python -m monitoring.health_checker --browser

# Check logs
tail -f logs/health-check-*.log | grep "captcha"

# Query Redis for recent results
redis-cli HGETALL health_check:broker:spokeo.com
```

### 2. Manual Verification

Visit the broker's opt-out URL in your browser:
```
Open: https://www.spokeo.com/optout
Look for: reCAPTCHA widget, hCaptcha, Cloudflare challenge
```

### 3. Check Metrics

```bash
# View broker removal metrics
curl http://localhost:9090/health | grep "spokeo.com"

# Check CAPTCHA rate trend
redis-cli GET "metrics:removal:spokeo.com:captcha_count"
```

## Resolution

### Scenario A: Temporary CAPTCHA (Bot Protection)

**If CAPTCHA is intermittent or only appears during high-volume periods:**

1. **Adjust rate limits** (more conservative)
```python
# Edit agents/shared/rate_limiter.py
RATE_LIMITS = {
    "spokeo.com": (5, 3600),  # Reduce from 10 to 5 requests/hour
}
```

2. **Add delays between requests**
```python
# In executor.py, add configurable delay
await asyncio.sleep(5)  # 5-second delay before submission
```

3. **Rotate User-Agents**
```python
# Use browser-like headers
headers = {
    "User-Agent": random.choice(BROWSER_USER_AGENTS),
}
```

4. **Monitor for 24 hours**
```bash
# Check if CAPTCHA rate decreases
watch -n 300 'curl -s http://localhost:9090/health | grep spokeo'
```

### Scenario B: Permanent CAPTCHA (Site Change)

**If CAPTCHA is present on every request:**

1. **Mark broker as manual-only**
```python
# Edit agents/discovery/agent.py
{
    "name": "spokeo.com",
    "url": "https://www.spokeo.com/search?q={name}",
    "tier": "free",
    "manual": True,  # Add this flag
}
```

2. **Update removal instructions**
```python
# Edit agents/removal/agent.py
OPT_OUT_REGISTRY["spokeo.com"] = RemovalProcedure(
    source_name="spokeo.com",
    procedure_type="web_form",
    opt_out_url="https://www.spokeo.com/optout",
    instructions="""
    IMPORTANT: This broker now uses CAPTCHA verification.

    Steps for manual opt-out:
    1. Visit https://www.spokeo.com/optout
    2. Complete the CAPTCHA challenge
    3. Enter your profile URL (get it from your dashboard)
    4. Enter your email address
    5. Click Submit
    6. Check email for verification link
    7. Click verification link to complete removal

    Expected removal time: 7 days
    """,
    requires_id_verification=False,
    estimated_days=7,
)
```

3. **Regenerate frontend**
```bash
cd agents
python -m removal.frontend_export
git add ../frontend/src/lib/brokers.generated.ts
```

4. **Commit changes**
```bash
git commit -m "Mark spokeo.com as manual-only due to CAPTCHA"
```

5. **Notify affected customers**
```sql
-- Find customers with pending Spokeo removals
SELECT DISTINCT customer_id, email
FROM removal_requests
WHERE broker = 'spokeo.com'
AND status = 'failed'
AND error_message LIKE '%CAPTCHA%';
```

Send notification:
```
Subject: Action Required: Manual Opt-Out for Spokeo.com

Spokeo.com has added CAPTCHA verification to their opt-out process,
which means automated removal is no longer possible.

We've updated your dashboard with step-by-step manual instructions.

What to do:
1. Log in to your dashboard
2. Navigate to "Spokeo.com" removal request
3. Click "Manual Instructions"
4. Follow the steps to complete opt-out yourself

This change affects all privacy services, not just VectorScrub.
We're working with Spokeo to explore API partnership options.

Questions? Reply to this email or contact support.
```

## Prevention

### For Temporary CAPTCHA
1. **Proactive rate limit tuning** - Start conservative, increase gradually
2. **Distributed requests** - Spread across multiple workers/IPs
3. **Browser fingerprinting** - Use playwright-stealth to appear more human

### For Permanent CAPTCHA
1. **Broker relationship program** - Build API partnerships
2. **Health monitoring** - Daily automated checks catch changes early
3. **Customer expectations** - Communicate that some brokers may require manual opt-out

## Escalation

### When to Escalate

**Escalate to Engineering Lead if:**
- Multiple brokers start showing CAPTCHA simultaneously (DDoS protection)
- CAPTCHA appears on tier-free brokers (impacts acquisition)
- Customer complaints exceed 10/day for single broker

**Escalate to Business Dev if:**
- High-value broker (>1000 removals/month) goes manual-only
- Opportunity to negotiate API partnership

### Escalation Template

```
Subject: [P2] CAPTCHA Detected on {broker_name}

Impact:
- Broker: {broker_name}
- Tier: {free/personal/premium}
- Removal volume: {N} requests/month
- CAPTCHA type: {reCAPTCHA/hCaptcha/Cloudflare}
- First detected: {date}
- Customer impact: {N} pending removals affected

Actions Taken:
- [x] Verified CAPTCHA is persistent
- [x] Marked broker as manual-only
- [x] Updated customer instructions
- [ ] Notified affected customers

Recommendations:
1. {API partnership opportunity?}
2. {Explore CAPTCHA solving services?}
3. {Accept manual-only status?}

Context:
{link to health check logs}
{link to metrics dashboard}
```

## Metrics to Track

After resolution, monitor:

```sql
-- Daily CAPTCHA detection count
SELECT
    DATE(created_at),
    COUNT(*) as captcha_detections
FROM removal_requests
WHERE error_message LIKE '%CAPTCHA%'
GROUP BY DATE(created_at)
ORDER BY DATE(created_at) DESC
LIMIT 30;

-- Broker health trends
SELECT * FROM health_check_summary
ORDER BY timestamp DESC
LIMIT 7;
```

## Related Runbooks
- [Rate Limit Breach](./rate-limit-breach.md) - Often precedes CAPTCHA
- [Broker Outage](./broker-outage.md) - May be mistaken for CAPTCHA
- [High Error Rates](./high-error-rates.md) - System-wide issues
