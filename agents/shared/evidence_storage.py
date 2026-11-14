"""
Evidence Storage System

Securely stores screenshot evidence from removal submissions.
Screenshots are encrypted at rest and only retrievable by the customer who owns them.

Architecture:
- Screenshots stored in filesystem (or S3 in production)
- Encrypted using customer-specific keys
- Indexed by evidence_hash (SHA-256)
- Access controlled via backend API
"""
import hashlib
import os
from dataclasses import dataclass
from pathlib import Path
from typing import Optional

import structlog
from cryptography.fernet import Fernet, InvalidToken

from shared.config import get_config

log = structlog.get_logger(__name__)


@dataclass
class EvidenceMetadata:
    """Metadata for a stored evidence screenshot."""
    evidence_hash: str
    customer_id: str
    removal_request_id: str
    broker: str
    timestamp: float
    file_size_bytes: int
    encrypted: bool


class EvidenceStorage:
    """
    Secure storage for removal submission screenshots.

    Screenshots are:
    1. Hashed (SHA-256) for content-addressable storage
    2. Encrypted with customer-specific key
    3. Stored with metadata for access control
    4. Retrievable only by owning customer
    """

    def __init__(self, base_path: Optional[str] = None):
        """
        Initialize evidence storage.

        Args:
            base_path: Base directory for evidence storage.
                      Defaults to $VECTORSCRUB_EVIDENCE_DIR or ./evidence_storage
        """
        self.config = get_config()
        self.base_path = Path(base_path or os.getenv(
            "VECTORSCRUB_EVIDENCE_DIR",
            "./evidence_storage"
        ))
        self.base_path.mkdir(parents=True, exist_ok=True)

        # Create subdirectories by hash prefix for performance
        # e.g., abc123... -> ab/c1/abc123...
        log.info("evidence_storage_initialized", base_path=str(self.base_path))

    def _get_storage_path(self, evidence_hash: str) -> Path:
        """
        Get filesystem path for an evidence file.

        Uses content-addressable storage with 2-level sharding:
        evidence_hash "abc123..." -> base_path/ab/c1/abc123.enc
        """
        if len(evidence_hash) < 4:
            raise ValueError("Evidence hash too short")

        # 2-level sharding: first 2 chars, next 2 chars
        shard1 = evidence_hash[:2]
        shard2 = evidence_hash[2:4]

        shard_path = self.base_path / shard1 / shard2
        shard_path.mkdir(parents=True, exist_ok=True)

        return shard_path / f"{evidence_hash}.enc"

    def _get_metadata_path(self, evidence_hash: str) -> Path:
        """Get path for metadata file."""
        storage_path = self._get_storage_path(evidence_hash)
        return storage_path.with_suffix(".meta")

    def _generate_encryption_key(self, customer_id: str, evidence_hash: str) -> bytes:
        """
        Generate deterministic encryption key for a screenshot.

        Combines:
        - Master evidence encryption key (from environment)
        - Customer ID
        - Evidence hash

        This ensures:
        1. Only customer can decrypt their evidence
        2. Key is deterministic (no separate key storage needed)
        3. Master key rotation is possible
        """
        master_key = os.getenv("VECTORSCRUB_EVIDENCE_KEY")
        if not master_key:
            raise ValueError("VECTORSCRUB_EVIDENCE_KEY not set")

        # Combine components
        key_material = f"{master_key}:{customer_id}:{evidence_hash}".encode()

        # Derive Fernet-compatible key (32 bytes, URL-safe base64)
        derived = hashlib.sha256(key_material).digest()
        # Fernet wants 32 bytes, urlsafe base64 encoded
        # We'll use the hash directly and encode it
        import base64
        return base64.urlsafe_b64encode(derived)

    async def store_evidence(
        self,
        screenshot_bytes: bytes,
        customer_id: str,
        removal_request_id: str,
        broker: str,
        timestamp: float,
    ) -> str:
        """
        Store screenshot evidence securely.

        Args:
            screenshot_bytes: Raw screenshot data (PNG/JPEG)
            customer_id: UUID of customer
            removal_request_id: UUID of removal request
            broker: Broker domain name
            timestamp: Unix timestamp of capture

        Returns:
            evidence_hash: SHA-256 hash of the screenshot (for reference)
        """
        # Generate evidence hash
        evidence_hash = hashlib.sha256(screenshot_bytes).hexdigest()

        # Check if already stored
        storage_path = self._get_storage_path(evidence_hash)
        if storage_path.exists():
            log.debug("evidence_already_stored", evidence_hash=evidence_hash)
            return evidence_hash

        try:
            # Encrypt screenshot
            encryption_key = self._generate_encryption_key(customer_id, evidence_hash)
            fernet = Fernet(encryption_key)
            encrypted_data = fernet.encrypt(screenshot_bytes)

            # Write encrypted data
            storage_path.write_bytes(encrypted_data)

            # Write metadata (unencrypted - used for access control)
            metadata = {
                "evidence_hash": evidence_hash,
                "customer_id": customer_id,
                "removal_request_id": removal_request_id,
                "broker": broker,
                "timestamp": timestamp,
                "file_size_bytes": len(screenshot_bytes),
                "encrypted": True,
            }

            import json
            self._get_metadata_path(evidence_hash).write_text(
                json.dumps(metadata, indent=2)
            )

            log.info(
                "evidence_stored",
                evidence_hash=evidence_hash,
                broker=broker,
                size_bytes=len(screenshot_bytes),
            )

            return evidence_hash

        except Exception as exc:
            log.error(
                "evidence_storage_failed",
                evidence_hash=evidence_hash,
                error_type=type(exc).__name__,
            )
            raise

    async def retrieve_evidence(
        self,
        evidence_hash: str,
        customer_id: str,
    ) -> Optional[bytes]:
        """
        Retrieve and decrypt screenshot evidence.

        Args:
            evidence_hash: SHA-256 hash of the screenshot
            customer_id: UUID of customer (for access control)

        Returns:
            Decrypted screenshot bytes, or None if not found

        Raises:
            PermissionError: If customer_id doesn't match owner
            InvalidToken: If decryption fails (wrong customer)
        """
        storage_path = self._get_storage_path(evidence_hash)
        metadata_path = self._get_metadata_path(evidence_hash)

        if not storage_path.exists():
            log.warning("evidence_not_found", evidence_hash=evidence_hash)
            return None

        try:
            # Check access control via metadata
            import json
            metadata = json.loads(metadata_path.read_text())

            if metadata["customer_id"] != customer_id:
                log.warning(
                    "evidence_access_denied",
                    evidence_hash=evidence_hash,
                    requested_by=customer_id,
                    owner=metadata["customer_id"],
                )
                raise PermissionError("Access denied: evidence belongs to different customer")

            # Decrypt
            encrypted_data = storage_path.read_bytes()
            encryption_key = self._generate_encryption_key(customer_id, evidence_hash)
            fernet = Fernet(encryption_key)
            screenshot_bytes = fernet.decrypt(encrypted_data)

            log.info(
                "evidence_retrieved",
                evidence_hash=evidence_hash,
                customer_id=customer_id,
            )

            return screenshot_bytes

        except InvalidToken:
            log.error(
                "evidence_decryption_failed",
                evidence_hash=evidence_hash,
                customer_id=customer_id,
            )
            raise

        except Exception as exc:
            log.error(
                "evidence_retrieval_failed",
                evidence_hash=evidence_hash,
                error_type=type(exc).__name__,
            )
            raise

    async def get_metadata(
        self,
        evidence_hash: str,
        customer_id: Optional[str] = None,
    ) -> Optional[EvidenceMetadata]:
        """
        Get metadata for evidence without retrieving the screenshot.

        Args:
            evidence_hash: SHA-256 hash of the screenshot
            customer_id: Optional customer ID for access control

        Returns:
            EvidenceMetadata or None if not found
        """
        metadata_path = self._get_metadata_path(evidence_hash)

        if not metadata_path.exists():
            return None

        try:
            import json
            data = json.loads(metadata_path.read_text())

            # Access control if customer_id provided
            if customer_id and data["customer_id"] != customer_id:
                return None

            return EvidenceMetadata(
                evidence_hash=data["evidence_hash"],
                customer_id=data["customer_id"],
                removal_request_id=data["removal_request_id"],
                broker=data["broker"],
                timestamp=data["timestamp"],
                file_size_bytes=data["file_size_bytes"],
                encrypted=data["encrypted"],
            )

        except Exception as exc:
            log.error(
                "metadata_retrieval_failed",
                evidence_hash=evidence_hash,
                error_type=type(exc).__name__,
            )
            return None

    async def delete_evidence(
        self,
        evidence_hash: str,
        customer_id: str,
    ) -> bool:
        """
        Delete evidence and metadata.

        Args:
            evidence_hash: SHA-256 hash of the screenshot
            customer_id: UUID of customer (for access control)

        Returns:
            True if deleted, False if not found or access denied
        """
        # Verify ownership first
        metadata = await self.get_metadata(evidence_hash, customer_id)
        if not metadata:
            log.warning("evidence_delete_denied", evidence_hash=evidence_hash)
            return False

        try:
            storage_path = self._get_storage_path(evidence_hash)
            metadata_path = self._get_metadata_path(evidence_hash)

            if storage_path.exists():
                storage_path.unlink()
            if metadata_path.exists():
                metadata_path.unlink()

            log.info(
                "evidence_deleted",
                evidence_hash=evidence_hash,
                customer_id=customer_id,
            )
            return True

        except Exception as exc:
            log.error(
                "evidence_deletion_failed",
                evidence_hash=evidence_hash,
                error_type=type(exc).__name__,
            )
            return False

    def get_storage_stats(self) -> dict:
        """Get storage statistics."""
        total_files = 0
        total_size = 0

        for evidence_file in self.base_path.rglob("*.enc"):
            total_files += 1
            total_size += evidence_file.stat().st_size

        return {
            "total_evidence_files": total_files,
            "total_size_bytes": total_size,
            "total_size_mb": round(total_size / 1024 / 1024, 2),
            "base_path": str(self.base_path),
        }


# Singleton instance
_global_storage: Optional[EvidenceStorage] = None


def get_evidence_storage() -> EvidenceStorage:
    """Get or create the global evidence storage instance."""
    global _global_storage
    if _global_storage is None:
        _global_storage = EvidenceStorage()
    return _global_storage
