# Observability & Monitoring

VectorScrub includes built-in observability features for production monitoring.

## Features

### 1. Prometheus Metrics

Exposes metrics in Prometheus format at `/api/v1/metrics`.

**Available Metrics:**

#### HTTP Metrics
- `http_requests_total` - Total HTTP requests by method, path, and status
- `http_request_duration_seconds` - Request latency histogram
- `http_requests_errors_total` - Total error responses (4xx, 5xx)

#### Database Metrics
- `db_connections_active` - Active database connections
- `db_query_duration_seconds` - Query latency histogram

#### Business Metrics
- `scans_initiated_total` - Total scans initiated
- `exposures_discovered_total` - Total exposures found
- `removal_requests_total` - Total removal requests

#### Job Queue Metrics
- `redis_queue_length` - Jobs in Redis queue
- `jobs_processed_total` - Total processed jobs
- `jobs_failed_total` - Total failed jobs

#### Agent Metrics
- `agent_api_calls_total` - Agent API calls
- `agent_callback_duration_seconds` - Agent callback duration

### 2. Distributed Tracing (Optional)

OpenTelemetry tracing with OTLP export to services like Jaeger, Honeycomb, or Datadog.

### 3. Structured Logging

JSON-formatted logs with trace IDs for correlation.

## Setup

### Prometheus

1. **Start Prometheus:**

```bash
docker run -d \
  --name prometheus \
  -p 9090:9090 \
  -v $(pwd)/prometheus.yml:/etc/prometheus/prometheus.yml \
  prom/prometheus
```

2. **Create prometheus.yml:**

```yaml
global:
  scrape_interval: 15s

scrape_configs:
  - job_name: 'vectorscrub'
    static_configs:
      - targets: ['host.docker.internal:8080']
    metrics_path: '/api/v1/metrics'
```

3. **Access Prometheus UI:**
   - Open http://localhost:9090
   - Query metrics: `http_requests_total`, `http_request_duration_seconds`, etc.

### Grafana (Optional)

1. **Start Grafana:**

```bash
docker run -d \
  --name grafana \
  -p 3001:3000 \
  grafana/grafana
```

2. **Configure:**
   - Open http://localhost:3001
   - Login: admin / admin
   - Add Prometheus data source: http://host.docker.internal:9090
   - Import VectorScrub dashboard (see grafana-dashboard.json)

### Distributed Tracing (Optional)

#### Using Jaeger

1. **Start Jaeger:**

```bash
docker run -d \
  --name jaeger \
  -p 4317:4317 \
  -p 16686:16686 \
  jaegertracing/all-in-one:latest
```

2. **Configure VectorScrub:**

```bash
# In .env
OTLP_ENDPOINT=http://localhost:4317
```

3. **View traces:**
   - Open http://localhost:16686
   - Select "vectorscrub-backend" service
   - Explore request traces

#### Using Honeycomb

```bash
# In .env
OTLP_ENDPOINT=https://api.honeycomb.io
HONEYCOMB_API_KEY=your-api-key
```

## Monitoring Dashboards

### Key Metrics to Monitor

1. **Request Rate & Latency:**
   - `rate(http_requests_total[5m])` - Requests per second
   - `histogram_quantile(0.95, http_request_duration_seconds)` - 95th percentile latency

2. **Error Rate:**
   - `rate(http_requests_errors_total[5m])` - Errors per second
   - `rate(http_requests_errors_total[5m]) / rate(http_requests_total[5m])` - Error percentage

3. **Scan Performance:**
   - `rate(scans_initiated_total[1h])` - Scans per hour
   - `rate(exposures_discovered_total[1h])` - Exposures found per hour

4. **Queue Health:**
   - `redis_queue_length` - Backlog size
   - `rate(jobs_failed_total[5m])` - Job failure rate

### Alerting Rules (Prometheus)

```yaml
groups:
  - name: vectorscrub
    rules:
      # High error rate
      - alert: HighErrorRate
        expr: rate(http_requests_errors_total[5m]) / rate(http_requests_total[5m]) > 0.05
        for: 5m
        annotations:
          summary: "High error rate (>5%)"

      # High latency
      - alert: HighLatency
        expr: histogram_quantile(0.95, http_request_duration_seconds) > 2
        for: 5m
        annotations:
          summary: "95th percentile latency >2s"

      # Queue backlog
      - alert: QueueBacklog
        expr: redis_queue_length > 100
        for: 10m
        annotations:
          summary: "Redis queue has >100 jobs"

      # High job failure rate
      - alert: JobFailures
        expr: rate(jobs_failed_total[5m]) > 0.1
        for: 5m
        annotations:
          summary: "Jobs failing at >0.1/sec"
```

## Log Levels

Set log level via `RUST_LOG` environment variable:

```bash
# Debug logging for VectorScrub
RUST_LOG=vectorscrub_backend=debug

# Info logging with HTTP traces
RUST_LOG=vectorscrub_backend=info,tower_http=debug

# Production (errors only)
RUST_LOG=vectorscrub_backend=error
```

## Production Checklist

- [ ] Prometheus scraping VectorScrub metrics endpoint
- [ ] Grafana dashboards configured with alerts
- [ ] Log aggregation (e.g., Loki, Elasticsearch) capturing JSON logs
- [ ] Distributed tracing (optional but recommended for debugging)
- [ ] Alert thresholds tuned to your SLA
- [ ] On-call rotation configured for critical alerts

## Troubleshooting

### Metrics endpoint returns 404

Ensure backend is running and route is registered:
```bash
curl http://localhost:8080/api/v1/metrics
```

### No metrics appear in Prometheus

1. Check Prometheus scrape targets: http://localhost:9090/targets
2. Verify `host.docker.internal` resolves (or use actual IP)
3. Check backend logs for errors

### OTLP tracing not working

1. Verify OTLP endpoint is reachable
2. Check backend logs for "OpenTelemetry tracing initialized"
3. Ensure firewall allows gRPC traffic on port 4317

## Cost Optimization

**Metric Retention:**
- Prometheus: 15 days default (adjust `--storage.tsdb.retention.time`)
- Grafana: Query data, doesn't store (relies on Prometheus)

**Trace Sampling:**
- Configure sampling in OpenTelemetry SDK to reduce trace volume
- Example: Sample 10% of requests in high-traffic deployments

**Log Volume:**
- Use `RUST_LOG=error` in production unless debugging
- Enable log filtering by endpoint (exclude health checks)

## Further Reading

- [Prometheus Documentation](https://prometheus.io/docs/)
- [Grafana Documentation](https://grafana.com/docs/)
- [OpenTelemetry Documentation](https://opentelemetry.io/docs/)
- [Jaeger Documentation](https://www.jaegertracing.io/docs/)

---

**Last Updated:** 2026-11-09
