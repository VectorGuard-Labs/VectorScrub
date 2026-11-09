# Data Broker Detection System

VectorScrub now includes an extensible data broker detection system that searches multiple people search sites and data aggregators.

## Supported Data Brokers

| Broker | Domain | Removal Difficulty | Status |
|--------|--------|-------------------|--------|
| **Whitepages** | whitepages.com | Medium | ✅ Implemented |
| **Spokeo** | spokeo.com | Easy | ✅ Implemented |
| **BeenVerified** | beenverified.com | Medium | ✅ Implemented |
| **TruthFinder** | truthfinder.com | Hard | ✅ Implemented |
| **Intelius** | intelius.com | Medium | ✅ Implemented |
| **PeopleFinders** | peoplefinders.com | Easy | ✅ Implemented |

**Note:** All data brokers now have working web scraping implementations. Each detector:
- Makes HTTP requests to the broker's public search endpoints
- Parses HTML results using BeautifulSoup
- Extracts PII (names, addresses, phone numbers, ages)
- Returns structured DetectionResult objects with confidence scores

**Important:** Many data brokers have anti-bot protection and may require:
- User-Agent rotation and request headers
- Rate limiting to avoid blocking
- CAPTCHA solving for some sites
- Subscriptions or API keys for full access
- Proxy rotation for high-volume scanning

## Architecture

### Base Components

1. **DataBrokerDetector** (base class)
   - Abstract interface for all detectors
   - Confidence scoring based on identity matching
   - Standard detection result format

2. **DataBrokerRegistry**
   - Central registry for all detectors
   - Automatic registration via decorator
   - Easy retrieval by broker name

3. **DataBrokerAggregator**
   - Runs all detectors in parallel
   - Consolidates and deduplicates results
   - Filters by confidence threshold

## Usage

### Basic Search

```python
from orchestrator.data_brokers import DataBrokerAggregator

# Customer identity context
identity = {
    "full_name": "John Doe",
    "current_address": "123 Main St, San Francisco, CA 94102",
    "phone_numbers": ["+1-555-123-4567"],
    "email_addresses": ["john.doe@example.com"],
}

# Create aggregator
aggregator = DataBrokerAggregator(identity)

# Search all brokers
results = await aggregator.search_all_brokers(
    full_name="John Doe",
    city="San Francisco",
    state="CA",
    min_confidence=0.6,
)

# Print results
for result in results:
    print(f"Found on {result.broker_name}")
    print(f"Profile: {result.profile_url}")
    print(f"Confidence: {result.confidence:.2f}")
    print(f"Remove: {result.removal_url}")
    print()
```

### Get Removal Summary

```python
# Generate removal action summary
summary = aggregator.get_removal_summary(results)

print(f"Total exposures: {summary['total_exposures']}")
print(f"Brokers with exposures: {len(summary['brokers'])}")

# Group by removal difficulty
for difficulty, exposures in summary['by_difficulty'].items():
    if exposures:
        print(f"\n{difficulty.upper()} removals ({len(exposures)}):")
        for exp in exposures:
            print(f"  - {exp['broker']}: {exp['profile_url']}")
```

## Adding New Data Brokers

### 1. Create Detector Class

```python
from orchestrator.data_brokers import DataBrokerDetector, RemovalDifficulty, register_detector

@register_detector
class MyBrokerDetector(DataBrokerDetector):
    """Detector for MyBroker.com."""

    def get_broker_name(self) -> str:
        return "MyBroker"

    def get_broker_domain(self) -> str:
        return "mybroker.com"

    def get_removal_info(self) -> Dict:
        return {
            "removal_url": "https://mybroker.com/optout",
            "removal_difficulty": RemovalDifficulty.EASY,
            "removal_instructions": "Step-by-step removal instructions...",
        }

    async def search(self, full_name, city=None, state=None, age=None):
        results = []

        # Implement search logic
        # 1. Build search URL
        # 2. Make HTTP request
        # 3. Parse HTML results
        # 4. Extract person information
        # 5. Calculate confidence
        # 6. Create DetectionResult objects

        return results
```

### 2. Import in Package

Add to `/agents/orchestrator/data_brokers/__init__.py`:

```python
from .mybroker import MyBrokerDetector
```

The `@register_detector` decorator automatically registers it with the system.

### 3. Test

```python
from orchestrator.data_brokers import DataBrokerRegistry

# Verify registration
brokers = DataBrokerRegistry.list_brokers()
assert "MyBroker" in brokers

# Test detector
detector = DataBrokerRegistry.get_detector("MyBroker")
results = await detector.search("John Doe", state="CA")
```

## Confidence Scoring

Detectors calculate confidence based on:

1. **Format Validity** (base: 0.5-0.7)
   - Does the result match expected format?
   - Are required fields present?

2. **Identity Match** (boost: +0.0 to +0.6)
   - Exact name match: +0.3
   - Partial name match: +0.15
   - Location match: +0.3
   - Phone match: +0.2
   - Age match (±2 years): +0.2

3. **Context Quality** (adjustment: -0.2 to +0.2)
   - Quality of source data
   - Number of matching fields
   - Data freshness

**Threshold:** Results with confidence < 0.6 are filtered out by default.

## Removal Process

### Automated Removal (Future)

VectorScrub will eventually automate removal requests where possible:

- **Easy Removals:** Automatic form submission
- **Medium Removals:** Email/phone verification assistance
- **Hard Removals:** Manual process with guidance
- **Very Hard Removals:** Legal assistance recommendations

### Current Process

Users must manually remove listings using provided instructions:

1. Visit removal URL
2. Follow step-by-step instructions
3. Verify identity (if required)
4. Wait for confirmation

## Rate Limiting & Ethics

### Best Practices

- Respect robots.txt and terms of service
- Implement rate limiting (max 1 request/second per broker)
- Use proxy rotation for large-scale scans
- Cache results to avoid duplicate requests
- Provide opt-out for customers who don't want broker scanning

### Rate Limiting Example

```python
import asyncio
from datetime import datetime, timedelta

class RateLimitedDetector:
    def __init__(self):
        self.last_request = {}
        self.min_interval = timedelta(seconds=1)

    async def rate_limited_request(self, broker_name, request_func):
        # Check last request time
        if broker_name in self.last_request:
            elapsed = datetime.now() - self.last_request[broker_name]
            if elapsed < self.min_interval:
                wait_time = (self.min_interval - elapsed).total_seconds()
                await asyncio.sleep(wait_time)

        # Make request
        result = await request_func()

        # Update last request time
        self.last_request[broker_name] = datetime.now()

        return result
```

## Production Considerations

### Subscriptions Required

Most data brokers require paid subscriptions:

- **Whitepages Premium:** ~$20/month
- **Spokeo:** ~$15/month
- **BeenVerified:** ~$25/month
- **TruthFinder:** ~$28/month
- **Intelius:** ~$20/month

**Total monthly cost:** ~$110/month for 5 brokers

### API Access

Some brokers offer API access:

- Faster than web scraping
- More reliable
- Higher rate limits
- Official support

Contact broker directly for API pricing.

### Legal Compliance

- Review each broker's terms of service
- Ensure FCRA compliance for people search
- Don't use for credit, employment, or housing decisions
- Respect privacy laws (GDPR, CCPA)

## Monitoring

Track data broker detection metrics:

```python
# Prometheus metrics
from metrics import counter, histogram

counter(
    "data_broker_searches_total",
    "broker" => broker_name,
    "status" => "success|failure"
).increment(1)

histogram(
    "data_broker_search_duration_seconds",
    "broker" => broker_name
).record(duration)

counter(
    "data_broker_exposures_found_total",
    "broker" => broker_name
).increment(num_results)
```

## Troubleshooting

### No Results Found

- Verify identity information is accurate
- Try different name variations (middle name, maiden name)
- Try different locations (current and previous addresses)
- Check if broker requires subscription

### Low Confidence Scores

- Add more identity context (phone, email, age)
- Verify data accuracy in identity profile
- Adjust confidence threshold (default: 0.6)

### Rate Limiting Errors

- Implement exponential backoff
- Use proxy rotation
- Reduce concurrent requests
- Add delays between requests

## Future Enhancements

- [ ] API integrations for major brokers
- [ ] Automated removal request submission
- [ ] CAPTCHA solving integration
- [ ] Proxy rotation and IP management
- [ ] Result caching and deduplication
- [ ] Historical tracking of exposures
- [ ] Notification on new exposures

---

**Last Updated:** 2026-11-09
