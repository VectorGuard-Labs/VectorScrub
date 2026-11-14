"""
Monitoring and alerting infrastructure for VectorScrub agents.

Provides:
- Broker health tracking
- Success rate metrics per broker
- Failed removal notifications
- SMTP delivery monitoring
- Prometheus-compatible metrics

Metrics are stored in Redis for distributed access and can be exported
to Prometheus or other monitoring systems.
"""
import time
from dataclasses import dataclass, field
from typing import Dict, List, Optional
from enum import Enum

import redis.asyncio as redis
import structlog

from shared.config import get_config

log = structlog.get_logger(__name__)


class OperationType(str, Enum):
    """Types of operations we track."""
    DISCOVERY = "discovery"
    DETECTION = "detection"
    REMOVAL = "removal"
    MONITORING = "monitoring"


class OperationStatus(str, Enum):
    """Status of an operation."""
    SUCCESS = "success"
    FAILURE = "failure"
    RATE_LIMITED = "rate_limited"
    BLOCKED = "blocked"
    TIMEOUT = "timeout"
    CAPTCHA = "captcha"


@dataclass
class BrokerHealth:
    """Health metrics for a single broker."""
    broker: str
    total_requests: int = 0
    successful_requests: int = 0
    failed_requests: int = 0
    rate_limited_count: int = 0
    blocked_count: int = 0
    timeout_count: int = 0
    captcha_count: int = 0
    last_success_at: Optional[float] = None
    last_failure_at: Optional[float] = None
    avg_response_time_ms: float = 0.0
    success_rate: float = 0.0

    def calculate_success_rate(self) -> float:
        """Calculate success rate as percentage."""
        if self.total_requests == 0:
            return 0.0
        return (self.successful_requests / self.total_requests) * 100.0


@dataclass
class RemovalMetrics:
    """Metrics for removal operations."""
    broker: str
    total_attempts: int = 0
    successful_removals: int = 0
    failed_removals: int = 0
    captcha_blocked: int = 0
    verification_required: int = 0
    avg_completion_time_s: float = 0.0
    success_rate: float = 0.0


@dataclass
class SMTPMetrics:
    """Metrics for SMTP email deliveries."""
    total_sent: int = 0
    successful: int = 0
    failed: int = 0
    auth_failures: int = 0
    connection_failures: int = 0
    timeout_failures: int = 0
    last_success_at: Optional[float] = None
    last_failure_at: Optional[float] = None


class MetricsCollector:
    """
    Collects and stores metrics in Redis.

    Metrics are stored with TTL for automatic cleanup and aggregated
    in time windows (hourly, daily) for trend analysis.
    """

    def __init__(self, redis_client: Optional[redis.Redis] = None):
        self.redis = redis_client
        self._local_redis: Optional[redis.Redis] = None
        self.config = get_config()

    async def _get_redis(self) -> redis.Redis:
        """Get or create Redis connection."""
        if self.redis:
            return self.redis
        if not self._local_redis:
            self._local_redis = redis.Redis.from_url(
                self.config.redis_url,
                decode_responses=True,
            )
        return self._local_redis

    async def record_operation(
        self,
        broker: str,
        operation: OperationType,
        status: OperationStatus,
        duration_ms: Optional[float] = None,
        metadata: Optional[Dict] = None,
    ) -> None:
        """
        Record an operation metric.

        Args:
            broker: Broker domain (e.g., "spokeo.com")
            operation: Type of operation
            status: Operation status
            duration_ms: Operation duration in milliseconds
            metadata: Additional metadata (error type, etc.)
        """
        r = await self._get_redis()
        now = time.time()
        pipe = r.pipeline()

        # Base key for this broker and operation
        base_key = f"metrics:{operation.value}:{broker}"

        # Increment counters
        pipe.hincrby(f"{base_key}:counts", "total", 1)
        pipe.hincrby(f"{base_key}:counts", status.value, 1)

        # Track timestamps
        if status == OperationStatus.SUCCESS:
            pipe.hset(f"{base_key}:timestamps", "last_success", now)
        else:
            pipe.hset(f"{base_key}:timestamps", "last_failure", now)

        # Track duration if provided
        if duration_ms is not None:
            pipe.lpush(f"{base_key}:durations", duration_ms)
            pipe.ltrim(f"{base_key}:durations", 0, 99)  # Keep last 100

        # Set TTL on keys (30 days)
        pipe.expire(f"{base_key}:counts", 30 * 24 * 3600)
        pipe.expire(f"{base_key}:timestamps", 30 * 24 * 3600)
        if duration_ms is not None:
            pipe.expire(f"{base_key}:durations", 30 * 24 * 3600)

        await pipe.execute()

        # Log significant events
        if status in (OperationStatus.FAILURE, OperationStatus.BLOCKED, OperationStatus.CAPTCHA):
            log.warning(
                "operation_failure_recorded",
                broker=broker,
                operation=operation.value,
                status=status.value,
                metadata=metadata,
            )

    async def get_broker_health(self, broker: str, operation: OperationType) -> BrokerHealth:
        """Get health metrics for a broker and operation type."""
        r = await self._get_redis()
        base_key = f"metrics:{operation.value}:{broker}"

        # Get counts
        counts = await r.hgetall(f"{base_key}:counts")
        timestamps = await r.hgetall(f"{base_key}:timestamps")
        durations = await r.lrange(f"{base_key}:durations", 0, -1)

        total = int(counts.get("total", 0))
        successful = int(counts.get("success", 0))
        failed = int(counts.get("failure", 0))
        rate_limited = int(counts.get("rate_limited", 0))
        blocked = int(counts.get("blocked", 0))
        timeout = int(counts.get("timeout", 0))
        captcha = int(counts.get("captcha", 0))

        # Calculate average duration
        avg_duration = 0.0
        if durations:
            numeric_durations = [float(d) for d in durations]
            avg_duration = sum(numeric_durations) / len(numeric_durations)

        health = BrokerHealth(
            broker=broker,
            total_requests=total,
            successful_requests=successful,
            failed_requests=failed,
            rate_limited_count=rate_limited,
            blocked_count=blocked,
            timeout_count=timeout,
            captcha_count=captcha,
            last_success_at=float(timestamps.get("last_success", 0)) or None,
            last_failure_at=float(timestamps.get("last_failure", 0)) or None,
            avg_response_time_ms=avg_duration,
        )
        health.success_rate = health.calculate_success_rate()
        return health

    async def get_all_broker_health(self, operation: OperationType) -> List[BrokerHealth]:
        """Get health metrics for all brokers for an operation type."""
        r = await self._get_redis()
        # Scan for all broker keys
        pattern = f"metrics:{operation.value}:*:counts"
        brokers = set()

        async for key in r.scan_iter(pattern):
            # Extract broker name from key
            # Format: metrics:operation:broker.com:counts
            parts = key.split(":")
            if len(parts) >= 3:
                brokers.add(parts[2])

        health_list = []
        for broker in brokers:
            health = await self.get_broker_health(broker, operation)
            health_list.append(health)

        # Sort by total requests descending
        health_list.sort(key=lambda h: h.total_requests, reverse=True)
        return health_list

    async def record_removal(
        self,
        broker: str,
        success: bool,
        duration_s: Optional[float] = None,
        captcha_blocked: bool = False,
        verification_required: bool = False,
    ) -> None:
        """Record a removal attempt."""
        status = OperationStatus.SUCCESS if success else OperationStatus.FAILURE
        if captcha_blocked:
            status = OperationStatus.CAPTCHA

        await self.record_operation(
            broker=broker,
            operation=OperationType.REMOVAL,
            status=status,
            duration_ms=duration_s * 1000 if duration_s else None,
            metadata={
                "captcha_blocked": captcha_blocked,
                "verification_required": verification_required,
            },
        )

    async def record_smtp_delivery(
        self,
        success: bool,
        error_type: Optional[str] = None,
    ) -> None:
        """Record SMTP email delivery."""
        r = await self._get_redis()
        now = time.time()
        pipe = r.pipeline()

        key = "metrics:smtp:deliveries"

        # Increment counters
        pipe.hincrby(key, "total", 1)
        if success:
            pipe.hincrby(key, "successful", 1)
            pipe.hset(key, "last_success", now)
        else:
            pipe.hincrby(key, "failed", 1)
            pipe.hset(key, "last_failure", now)

            # Track error types
            if error_type:
                if "auth" in error_type.lower() or "login" in error_type.lower():
                    pipe.hincrby(key, "auth_failures", 1)
                elif "connect" in error_type.lower() or "connection" in error_type.lower():
                    pipe.hincrby(key, "connection_failures", 1)
                elif "timeout" in error_type.lower():
                    pipe.hincrby(key, "timeout_failures", 1)

        pipe.expire(key, 30 * 24 * 3600)  # 30 days TTL
        await pipe.execute()

        if not success:
            log.warning("smtp_delivery_failed", error_type=error_type)

    async def get_smtp_metrics(self) -> SMTPMetrics:
        """Get SMTP delivery metrics."""
        r = await self._get_redis()
        data = await r.hgetall("metrics:smtp:deliveries")

        return SMTPMetrics(
            total_sent=int(data.get("total", 0)),
            successful=int(data.get("successful", 0)),
            failed=int(data.get("failed", 0)),
            auth_failures=int(data.get("auth_failures", 0)),
            connection_failures=int(data.get("connection_failures", 0)),
            timeout_failures=int(data.get("timeout_failures", 0)),
            last_success_at=float(data.get("last_success", 0)) or None,
            last_failure_at=float(data.get("last_failure", 0)) or None,
        )

    async def export_prometheus(self) -> str:
        """
        Export all metrics in Prometheus text format.

        Returns:
            Prometheus-formatted metrics string
        """
        lines = []

        # Broker health metrics for each operation type
        for operation in OperationType:
            health_list = await self.get_all_broker_health(operation)

            for health in health_list:
                broker_label = f'broker="{health.broker}",operation="{operation.value}"'

                lines.append(f"# HELP vectorscrub_requests_total Total requests to broker")
                lines.append(f"# TYPE vectorscrub_requests_total counter")
                lines.append(f"vectorscrub_requests_total{{{broker_label}}} {health.total_requests}")

                lines.append(f"# HELP vectorscrub_requests_successful Successful requests")
                lines.append(f"# TYPE vectorscrub_requests_successful counter")
                lines.append(f"vectorscrub_requests_successful{{{broker_label}}} {health.successful_requests}")

                lines.append(f"# HELP vectorscrub_requests_failed Failed requests")
                lines.append(f"# TYPE vectorscrub_requests_failed counter")
                lines.append(f"vectorscrub_requests_failed{{{broker_label}}} {health.failed_requests}")

                lines.append(f"# HELP vectorscrub_success_rate Success rate percentage")
                lines.append(f"# TYPE vectorscrub_success_rate gauge")
                lines.append(f"vectorscrub_success_rate{{{broker_label}}} {health.success_rate:.2f}")

                lines.append(f"# HELP vectorscrub_avg_response_time_ms Average response time")
                lines.append(f"# TYPE vectorscrub_avg_response_time_ms gauge")
                lines.append(f"vectorscrub_avg_response_time_ms{{{broker_label}}} {health.avg_response_time_ms:.2f}")

        # SMTP metrics
        smtp = await self.get_smtp_metrics()
        lines.append(f"# HELP vectorscrub_smtp_total Total SMTP emails sent")
        lines.append(f"# TYPE vectorscrub_smtp_total counter")
        lines.append(f"vectorscrub_smtp_total {smtp.total_sent}")

        lines.append(f"# HELP vectorscrub_smtp_successful Successful SMTP deliveries")
        lines.append(f"# TYPE vectorscrub_smtp_successful counter")
        lines.append(f"vectorscrub_smtp_successful {smtp.successful}")

        lines.append(f"# HELP vectorscrub_smtp_failed Failed SMTP deliveries")
        lines.append(f"# TYPE vectorscrub_smtp_failed counter")
        lines.append(f"vectorscrub_smtp_failed {smtp.failed}")

        return "\n".join(lines) + "\n"

    async def close(self) -> None:
        """Close Redis connection if we created it."""
        if self._local_redis:
            await self._local_redis.aclose()
            self._local_redis = None


# Singleton instance
_global_collector: Optional[MetricsCollector] = None


def get_metrics_collector() -> MetricsCollector:
    """Get or create the global metrics collector instance."""
    global _global_collector
    if _global_collector is None:
        _global_collector = MetricsCollector()
    return _global_collector
