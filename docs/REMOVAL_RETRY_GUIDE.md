## Removal Retry System - User Guide

### Overview

The removal retry system allows customers to re-attempt failed removal requests without counting against their scan quota. This feature helps handle transient failures (network issues, temporary broker downtime) and gives customers control over their removal requests.

### How It Works

1. **Automatic Detection** - When a removal fails, it's marked as eligible for retry
2. **Customer Action** - Customer clicks "Retry" button in dashboard
3. **Eligibility Check** - System verifies retry is allowed (within limits)
4. **Fresh Attempt** - New browser session attempts the removal again
5. **Result Update** - Dashboard shows success/failure of retry

### Retry Limits

To prevent abuse and excessive load, the system enforces limits:

| Limit Type | Value | Reason |
|------------|-------|--------|
| **Per Request** | 3 retries max | Prevents infinite retry loops |
| **Cooldown** | 1 hour between retries | Gives brokers time to recover |
| **Daily Limit** | 10 retries/day/customer | Prevents system abuse |

### When to Retry

**Good Candidates for Retry:**
- ✅ "Request timed out" errors
- ✅ "Network connection failed" errors
- ✅ "Page layout may have changed" (temporary)
- ✅ Server errors (5xx) from broker
- ✅ "No submit button found" (page may have loaded slowly)

**Don't Retry (Won't Help):**
- ❌ CAPTCHA detected (needs manual opt-out)
- ❌ "Maximum retry limit reached"
- ❌ Already succeeded (shows "Submitted")
- ❌ Broker requires ID verification (see manual instructions)
- ❌ Form validation errors (wrong data format)

### Customer Dashboard UI

#### Failed Removal Card

```
┌─────────────────────────────────────────────┐
│ ❌ Spokeo.com Removal Failed                │
│                                             │
│ Status: Failed                              │
│ Error: Request timed out                    │
│ Last Attempted: Nov 13, 2026 2:30 PM       │
│ Retries: 1 of 3                            │
│                                             │
│ [🔄 Retry Removal]  [📖 Manual Instructions]│
└─────────────────────────────────────────────┘
```

#### Retry Button States

**Enabled** (green):
```
[🔄 Retry Removal]
```

**Cooldown** (disabled with tooltip):
```
[⏱️ Retry Available in 45 minutes]
```

**Limit Reached** (disabled with explanation):
```
[❌ Maximum Retries Reached]
  Use Manual Instructions below
```

**Daily Limit** (disabled):
```
[❌ Daily Retry Limit Reached]
  Try again tomorrow
```

### Customer Workflow

#### 1. View Failed Removal

Navigate to **Dashboard → Removal Requests → Failed**

#### 2. Check Retry Eligibility

Hover over the retry button to see:
- Current retry count (e.g., "1 of 3")
- Cooldown status
- Next available retry time

#### 3. Initiate Retry

Click **"Retry Removal"** button

System shows:
```
✓ Retry initiated
Your removal request will be re-attempted shortly.
You'll receive an email when it completes.
```

#### 4. Monitor Progress

Removal card updates in real-time:
```
⏳ Retrying...
Attempting removal submission (Retry 2 of 3)
```

#### 5. See Result

**Success:**
```
✓ Removal Submitted
Your opt-out request was successfully submitted to Spokeo.com
Confirmation: REF-ABC123
```

**Failed Again:**
```
❌ Retry Failed
The removal request failed again.
Error: CAPTCHA detected
→ Use Manual Instructions to complete opt-out yourself
```

### Email Notifications

Customers receive emails for retry events:

#### Retry Success Email

```
Subject: ✓ Removal Retry Successful - Spokeo.com

Your retry for Spokeo.com has been successfully submitted!

Confirmation Reference: REF-ABC123
Submitted: Nov 13, 2026 3:15 PM
Expected Removal: Within 7 days

We'll notify you once the broker confirms removal.

View Details: https://app.vectorscrub.com/removals/...
```

#### Retry Failed Email

```
Subject: Removal Retry Failed - Spokeo.com

Your retry for Spokeo.com was unsuccessful.

Error: CAPTCHA detected on opt-out page
Retries Used: 2 of 3
Next Retry Available: Nov 13, 2026 4:30 PM

What to do:
• Wait and try again later (CAPTCHA may be temporary)
• Use manual opt-out instructions in your dashboard
• Contact support if you need help

Manual Instructions: https://app.vectorscrub.com/removals/...
```

### Manual Opt-Out Fallback

If retries are exhausted or CAPTCHA is persistent:

1. Click **"Manual Instructions"** on removal card
2. Follow step-by-step guide for that specific broker
3. Mark as completed manually (optional)
4. Broker verification will still run to confirm removal

### Admin Dashboard

Admins see additional retry metrics:

#### Retry Statistics

```
Retry Success Rate: 72%
  ├─ Network Errors: 85% success
  ├─ Timeout Errors: 78% success
  ├─ Page Layout: 45% success
  └─ CAPTCHA: 5% success (mark manual)

Daily Retries: 127 today
  ├─ Successful: 92
  ├─ Failed: 35
  └─ Pending: 3

Top Reasons for Retry:
  1. Request timeout (45%)
  2. Page layout changed (22%)
  3. Network error (18%)
  4. Server error (10%)
  5. Other (5%)
```

#### Broker Retry Patterns

```
Spokeo.com
  ├─ Retry Success Rate: 45%
  ├─ Avg Retries to Success: 1.8
  └─ CAPTCHA Rate: 65% ⚠️
     → Consider marking manual

WhitePages.com
  ├─ Retry Success Rate: 90%
  ├─ Avg Retries to Success: 1.2
  └─ CAPTCHA Rate: 2% ✓
     → Retries working well
```

### Best Practices

**For Customers:**
1. Wait at least 1 hour before retrying (cooldown period)
2. If retry fails twice, try manual opt-out
3. Check email for retry results
4. Don't retry CAPTCHA errors (use manual instructions)

**For Support:**
1. Check retry history before suggesting manual opt-out
2. If 3 retries failed, broker likely has CAPTCHA
3. Update broker to manual-only if persistent CAPTCHA
4. Monitor retry success rates to detect broker changes

**For Developers:**
1. Analyze failed retry patterns weekly
2. Update site-specific handlers for low success rates
3. Mark brokers manual-only if retry success < 30%
4. Alert on sudden retry success rate drops

### Troubleshooting

#### "Retry button is disabled"

**Possible Reasons:**
1. On cooldown - Check "Available in X minutes" tooltip
2. Daily limit reached - Try tomorrow
3. Max retries reached - Use manual instructions
4. Already succeeded - No retry needed

#### "Retry keeps failing"

**Solutions:**
1. Check error message - may be CAPTCHA (use manual)
2. Wait longer between retries (broker may be down)
3. Try at different time of day (rate limiting)
4. Contact support - broker may have changed

#### "Can't see retry button"

**Check:**
1. Is removal actually failed? (not pending/submitted)
2. Is removal recent? (old requests may not be retryable)
3. Browser refresh - dashboard may be stale

### API Usage (Advanced)

For integrations via API:

```bash
# Check if retry is allowed
GET /api/v1/customers/{customer_id}/removal-requests/{request_id}/retry-eligibility

# Initiate retry
POST /api/v1/customers/{customer_id}/removal-requests/{request_id}/retry

# Get retry history
GET /api/v1/customers/{customer_id}/removal-requests/{request_id}/retry-history
```

See `backend/API_RETRY_ENDPOINTS.md` for full API documentation.

### Metrics & Analytics

Track retry performance:

- **Retry Success Rate** - % of retries that succeed
- **Retries to Success** - Average attempts before success
- **Cooldown Compliance** - % respecting 1-hour wait
- **Daily Limit Hits** - Customers hitting 10/day limit
- **Broker Patterns** - Which brokers have low retry success

These metrics help identify:
- Brokers needing manual-only flags
- Site-specific handlers needing updates
- Temporary vs. permanent broker issues
