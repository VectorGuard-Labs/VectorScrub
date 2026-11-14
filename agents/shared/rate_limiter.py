"""
Rate limiting for data broker requests.

Prevents hitting broker rate limits and IP bans by tracking requests per broker
per time window. Uses Redis for distributed rate limiting across multiple workers.

Architecture: Each broker has a (requests, window_seconds) limit. The rate limiter
uses a sliding window algorithm with Redis sorted sets to track request timestamps.
"""
import asyncio
import time
from typing import Optional

import redis.asyncio as redis
import structlog

from shared.config import get_config

log = structlog.get_logger(__name__)

# Rate limits per broker: (max_requests, window_seconds)
# Conservative limits based on typical broker behavior and TOS.
# Adjust based on observed 429 responses and broker documentation.
RATE_LIMITS = {
    # Free tier brokers
    "spokeo.com": (10, 3600),  # 10 requests per hour
    "whitepages.com": (15, 3600),  # 15 requests per hour
    "peoplefinder.com": (8, 3600),  # 8 requests per hour (shares Intelius infra)
    "truepeoplesearch.com": (20, 3600),  # 20 requests per hour
    # Personal tier brokers
    "fastpeoplesearch.com": (20, 3600),  # 20 requests per hour
    "familytreenow.com": (12, 3600),  # 12 requests per hour
    "nuwber.com": (10, 3600),  # 10 requests per hour (aggressive bot detection)
    "beenverified.com": (8, 3600),  # 8 requests per hour
    "intelius.com": (8, 3600),  # 8 requests per hour (PeopleConnect shared)
    "mylife.com": (15, 3600),  # 15 requests per hour (email-based, more lenient)
    "ussearch.com": (8, 3600),  # 8 requests per hour (PeopleConnect shared)
    "peoplelooker.com": (10, 3600),  # 10 requests per hour
    "ownerly.com": (12, 3600),  # 12 requests per hour (BeenVerified shared)
    "instantcheckmate.com": (8, 3600),  # 8 requests per hour (PeopleConnect shared)
    "radaris.com": (15, 3600),  # 15 requests per hour
    # Premium tier brokers
    "truthfinder.com": (8, 3600),  # 8 requests per hour (PeopleConnect shared)
    "thatsthem.com": (12, 3600),  # 12 requests per hour
    "smartbackgroundchecks.com": (10, 3600),  # 10 requests per hour
    "advancedbackgroundchecks.com": (10, 3600),  # 10 requests per hour
}

# Default rate limit for brokers not in the registry
DEFAULT_RATE_LIMIT = (5, 3600)  # Conservative: 5 requests per hour


class RateLimitExceeded(Exception):
    """Raised when a broker's rate limit would be exceeded."""

    def __init__(self, broker: str, retry_after_seconds: float):
        self.broker = broker
        self.retry_after_seconds = retry_after_seconds
        super().__init__(
            f"Rate limit exceeded for {broker}. Retry after {retry_after_seconds:.0f} seconds."
        )


class RateLimiter:
    """
    Distributed rate limiter using Redis sorted sets.

    Uses sliding window algorithm: stores request timestamps in a sorted set,
    removes expired entries, counts remaining entries, and allows request if
    under limit.
    """

    def __init__(self, redis_client: Optional[redis.Redis] = None):
        self.redis = redis_client
        self.config = get_config()
        self._local_redis: Optional[redis.Redis] = None

    async def _get_redis(self) -> redis.Redis:
        """Get or create Redis connection."""
        if self.redis:
            return self.redis
        if not self._local_redis:
            self._local_redis = redis.Redis.from_url(
                self.config.redis_url,
                decode_responses=False,  # We're using timestamps, not strings
            )
        return self._local_redis

    async def check_rate_limit(self, broker: str, operation: str = "request") -> None:
        """
        Check if a request to this broker would exceed the rate limit.

        Raises RateLimitExceeded if limit would be exceeded.

        Args:
            broker: Broker domain (e.g., "spokeo.com")
            operation: Operation type (e.g., "request", "removal", "scan")
        """
        max_requests, window_seconds = RATE_LIMITS.get(broker, DEFAULT_RATE_LIMIT)
        key = f"ratelimit:{broker}:{operation}"

        r = await self._get_redis()
        now = time.time()
        window_start = now - window_seconds

        # Use pipeline for atomic operations
        pipe = r.pipeline()
        # Remove expired entries
        pipe.zremrangebyscore(key, "-inf", window_start)
        # Count requests in current window
        pipe.zcount(key, window_start, "+inf")
        # Set expiration on the key itself (cleanup)
        pipe.expire(key, window_seconds)
        results = await pipe.execute()

        current_count = results[1]

        if current_count >= max_requests:
            # Find oldest timestamp in window to calculate retry_after
            oldest = await r.zrange(key, 0, 0, withscores=True)
            if oldest:
                oldest_timestamp = oldest[0][1]
                retry_after = oldest_timestamp + window_seconds - now
            else:
                retry_after = window_seconds

            log.warning(
                "rate_limit_exceeded",
                broker=broker,
                operation=operation,
                current_count=current_count,
                max_requests=max_requests,
                window_seconds=window_seconds,
                retry_after_seconds=retry_after,
            )
            raise RateLimitExceeded(broker, max(0, retry_after))

    async def record_request(self, broker: str, operation: str = "request") -> None:
        """
        Record a request to this broker.

        Call this after a successful request to update the rate limit counter.

        Args:
            broker: Broker domain (e.g., "spokeo.com")
            operation: Operation type (e.g., "request", "removal", "scan")
        """
        _, window_seconds = RATE_LIMITS.get(broker, DEFAULT_RATE_LIMIT)
        key = f"ratelimit:{broker}:{operation}"

        r = await self._get_redis()
        now = time.time()

        # Add current timestamp to sorted set
        pipe = r.pipeline()
        pipe.zadd(key, {str(now): now})
        pipe.expire(key, window_seconds)
        await pipe.execute()

        log.debug("rate_limit_recorded", broker=broker, operation=operation)

    async def get_current_usage(self, broker: str, operation: str = "request") -> dict:
        """
        Get current rate limit usage for a broker.

        Returns:
            {
                "current_count": int,
                "max_requests": int,
                "window_seconds": int,
                "remaining": int,
                "reset_at": float (unix timestamp)
            }
        """
        max_requests, window_seconds = RATE_LIMITS.get(broker, DEFAULT_RATE_LIMIT)
        key = f"ratelimit:{broker}:{operation}"

        r = await self._get_redis()
        now = time.time()
        window_start = now - window_seconds

        # Clean and count
        pipe = r.pipeline()
        pipe.zremrangebyscore(key, "-inf", window_start)
        pipe.zcount(key, window_start, "+inf")
        pipe.zrange(key, 0, 0, withscores=True)
        results = await pipe.execute()

        current_count = results[1]
        oldest = results[2]

        reset_at = now + window_seconds
        if oldest:
            reset_at = oldest[0][1] + window_seconds

        return {
            "current_count": current_count,
            "max_requests": max_requests,
            "window_seconds": window_seconds,
            "remaining": max(0, max_requests - current_count),
            "reset_at": reset_at,
        }

    async def wait_if_needed(
        self, broker: str, operation: str = "request", max_wait: float = 300
    ) -> float:
        """
        Wait if rate limit is exceeded, then proceed.

        Returns the time waited in seconds.

        Args:
            broker: Broker domain
            operation: Operation type
            max_wait: Maximum time to wait in seconds (default 5 minutes)

        Raises:
            RateLimitExceeded: If wait time would exceed max_wait
        """
        try:
            await self.check_rate_limit(broker, operation)
            return 0.0
        except RateLimitExceeded as exc:
            if exc.retry_after_seconds > max_wait:
                log.warning(
                    "rate_limit_wait_too_long",
                    broker=broker,
                    retry_after=exc.retry_after_seconds,
                    max_wait=max_wait,
                )
                raise

            log.info(
                "rate_limit_waiting",
                broker=broker,
                operation=operation,
                wait_seconds=exc.retry_after_seconds,
            )
            await asyncio.sleep(exc.retry_after_seconds)
            return exc.retry_after_seconds

    async def close(self) -> None:
        """Close Redis connection if we created it."""
        if self._local_redis:
            await self._local_redis.aclose()
            self._local_redis = None


# Singleton instance for convenience
_global_limiter: Optional[RateLimiter] = None


def get_rate_limiter() -> RateLimiter:
    """Get or create the global rate limiter instance."""
    global _global_limiter
    if _global_limiter is None:
        _global_limiter = RateLimiter()
    return _global_limiter
