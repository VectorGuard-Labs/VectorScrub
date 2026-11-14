"""
Distributed locking using Redis.

Prevents concurrent operations on the same resource across multiple workers.
Uses Redis SET with NX (only set if not exists) and EX (expiry) for automatic
cleanup if a worker crashes.
"""
import asyncio
from typing import Optional

import redis.asyncio as redis
import structlog

from shared.config import get_config

log = structlog.get_logger(__name__)


class LockAcquisitionFailed(Exception):
    """Raised when a lock cannot be acquired within the timeout."""

    def __init__(self, key: str, timeout: float):
        self.key = key
        self.timeout = timeout
        super().__init__(f"Failed to acquire lock '{key}' within {timeout}s")


class DistributedLock:
    """
    Redis-based distributed lock with automatic expiration.

    Usage:
        async with DistributedLock("scan:customer_123", timeout=300):
            # ... do work ...
            pass

    The lock is automatically released when exiting the context manager.
    If the process crashes, the lock expires after `timeout` seconds.
    """

    def __init__(
        self,
        key: str,
        timeout: int = 300,
        wait_timeout: Optional[float] = None,
        redis_client: Optional[redis.Redis] = None,
    ):
        """
        Args:
            key: Lock key (e.g., "scan:customer_123")
            timeout: Lock expiration in seconds (default 300 = 5 minutes)
            wait_timeout: How long to wait for lock acquisition in seconds.
                         None = fail immediately if locked.
                         0 = wait indefinitely.
                         >0 = wait up to this many seconds.
            redis_client: Optional Redis client (creates one if not provided)
        """
        self.key = f"lock:{key}"
        self.timeout = timeout
        self.wait_timeout = wait_timeout
        self.redis = redis_client
        self._local_redis: Optional[redis.Redis] = None
        self._lock_value: Optional[str] = None
        self._acquired = False
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

    async def acquire(self) -> bool:
        """
        Try to acquire the lock.

        Returns:
            True if lock was acquired, False otherwise

        Raises:
            LockAcquisitionFailed: If wait_timeout is exceeded
        """
        r = await self._get_redis()
        # Unique value so we only release our own lock
        self._lock_value = f"lock_{id(self)}_{asyncio.current_task()}"

        # No wait timeout - try once
        if self.wait_timeout is None:
            acquired = await r.set(
                self.key,
                self._lock_value,
                nx=True,  # Only set if not exists
                ex=self.timeout,  # Expiry in seconds
            )
            if acquired:
                self._acquired = True
                log.debug("lock_acquired", key=self.key, timeout=self.timeout)
                return True
            else:
                log.debug("lock_already_held", key=self.key)
                return False

        # Wait indefinitely
        if self.wait_timeout == 0:
            while True:
                acquired = await r.set(self.key, self._lock_value, nx=True, ex=self.timeout)
                if acquired:
                    self._acquired = True
                    log.debug("lock_acquired", key=self.key, timeout=self.timeout)
                    return True
                await asyncio.sleep(0.5)

        # Wait with timeout
        start = asyncio.get_event_loop().time()
        poll_interval = min(0.5, self.wait_timeout / 10)
        while True:
            acquired = await r.set(self.key, self._lock_value, nx=True, ex=self.timeout)
            if acquired:
                self._acquired = True
                elapsed = asyncio.get_event_loop().time() - start
                log.debug(
                    "lock_acquired",
                    key=self.key,
                    timeout=self.timeout,
                    waited_seconds=round(elapsed, 2),
                )
                return True

            elapsed = asyncio.get_event_loop().time() - start
            if elapsed >= self.wait_timeout:
                log.warning(
                    "lock_acquisition_timeout",
                    key=self.key,
                    wait_timeout=self.wait_timeout,
                )
                raise LockAcquisitionFailed(self.key, self.wait_timeout)

            await asyncio.sleep(poll_interval)

    async def release(self) -> None:
        """Release the lock if we hold it."""
        if not self._acquired:
            return

        r = await self._get_redis()

        # Lua script to ensure we only delete our own lock
        # (Another worker could have acquired it after ours expired)
        lua_script = """
        if redis.call("get", KEYS[1]) == ARGV[1] then
            return redis.call("del", KEYS[1])
        else
            return 0
        end
        """
        deleted = await r.eval(lua_script, 1, self.key, self._lock_value)
        if deleted:
            log.debug("lock_released", key=self.key)
        else:
            log.warning("lock_already_released", key=self.key)

        self._acquired = False

    async def extend(self, additional_seconds: int) -> bool:
        """
        Extend the lock expiration by additional_seconds.

        Returns True if extended, False if we don't hold the lock.
        """
        if not self._acquired:
            return False

        r = await self._get_redis()

        # Lua script to extend only if we still hold the lock
        lua_script = """
        if redis.call("get", KEYS[1]) == ARGV[1] then
            return redis.call("expire", KEYS[1], ARGV[2])
        else
            return 0
        end
        """
        extended = await r.eval(
            lua_script, 1, self.key, self._lock_value, str(self.timeout + additional_seconds)
        )
        if extended:
            log.debug("lock_extended", key=self.key, additional_seconds=additional_seconds)
            return True
        else:
            log.warning("lock_extend_failed", key=self.key)
            return False

    async def __aenter__(self):
        """Context manager entry - acquire lock."""
        await self.acquire()
        return self

    async def __aexit__(self, exc_type, exc_val, exc_tb):
        """Context manager exit - release lock."""
        await self.release()
        if self._local_redis:
            await self._local_redis.aclose()
            self._local_redis = None
        return False  # Don't suppress exceptions
