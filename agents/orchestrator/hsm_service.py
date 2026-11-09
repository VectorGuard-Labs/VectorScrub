"""
PyHSM integration for VectorScrub key management.

This module wraps PyHSM to provide secure key storage and encryption operations.
Replaces environment variable-based master key storage with HSM-backed encryption.
"""

import os
from pathlib import Path
from typing import Optional

from hsm import PyHSM


class HSMService:
    """
    HSM service wrapper for VectorScrub.

    Manages encryption keys using PyHSM instead of environment variables.
    Provides automatic key generation, rotation, and secure storage.
    """

    def __init__(
        self,
        storage_path: Optional[str] = None,
        master_password: Optional[str] = None,
    ):
        """
        Initialize HSM service.

        Args:
            storage_path: Path to encrypted keystore file (default: ./keystore.enc)
            master_password: Master password for keystore (from HSM_MASTER_PASSWORD env var)
        """
        self.storage_path = storage_path or os.getenv(
            "HSM_STORAGE_PATH", "./keystore.enc"
        )
        self.master_password = master_password or os.getenv("HSM_MASTER_PASSWORD")

        if not self.master_password:
            raise ValueError(
                "HSM_MASTER_PASSWORD environment variable must be set"
            )

        # Initialize PyHSM
        self.hsm = PyHSM(
            storage_path=self.storage_path,
            master_password=self.master_password,
        )

        # Ensure master encryption key exists
        self._ensure_master_key()

    def _ensure_master_key(self):
        """Ensure the master encryption key exists, creating it if necessary."""
        master_key_name = "vectorscrub-master-key"

        try:
            # Try to get the key to see if it exists
            self.hsm.get_key_info(master_key_name)
        except Exception:
            # Key doesn't exist, create it
            self.hsm.generate_key(
                master_key_name,
                key_type="aes-256",
                policy={
                    "description": "VectorScrub master encryption key for customer PII",
                    "rotate_every_days": 90,  # Automatic rotation every 90 days
                },
            )

    def get_master_key_base64(self) -> str:
        """
        Get the master encryption key in base64 format.

        This is used by the Rust backend for compatibility with existing code.
        The key is exported from HSM and should be zeroized after use.

        Returns:
            Base64-encoded 256-bit encryption key
        """
        master_key_name = "vectorscrub-master-key"

        # Export key material
        # Note: PyHSM's export_key returns the raw key bytes
        key_info = self.hsm.export_key(master_key_name)

        # Convert to base64 for compatibility with Rust backend
        import base64

        return base64.b64encode(key_info["key"]).decode("utf-8")

    def encrypt(self, key_name: str, plaintext: bytes) -> bytes:
        """
        Encrypt data using a named key.

        Args:
            key_name: Name of the key to use
            plaintext: Data to encrypt

        Returns:
            Ciphertext bytes
        """
        return self.hsm.encrypt(key_name, plaintext)

    def decrypt(self, key_name: str, ciphertext: bytes) -> bytes:
        """
        Decrypt data using a named key.

        Args:
            key_name: Name of the key to use
            ciphertext: Data to decrypt

        Returns:
            Plaintext bytes
        """
        return self.hsm.decrypt(key_name, ciphertext)

    def generate_derived_key(self, customer_id: str) -> str:
        """
        Generate a derived key for a specific customer.

        This creates a customer-specific encryption key derived from the master key.

        Args:
            customer_id: Customer UUID

        Returns:
            Key name for the customer's encryption key
        """
        key_name = f"customer-{customer_id}"

        try:
            self.hsm.get_key_info(key_name)
        except Exception:
            # Key doesn't exist, create it
            self.hsm.generate_key(
                key_name,
                key_type="aes-256",
                policy={
                    "description": f"Per-customer encryption key for {customer_id}",
                    "customer_id": customer_id,
                },
            )

        return key_name

    def rotate_master_key(self):
        """
        Manually trigger master key rotation.

        This is called automatically based on the rotate_every_days policy,
        but can be triggered manually for emergency rotation.
        """
        self.hsm.rotate_key("vectorscrub-master-key")

    def backup_keystore(self, backup_path: str):
        """
        Create an encrypted backup of the keystore.

        Args:
            backup_path: Path to write backup file
        """
        self.hsm.backup(backup_path)

    def close(self):
        """Close HSM session and zeroize key material."""
        if self.hsm:
            self.hsm.close_session()


# Global HSM instance
_hsm_instance: Optional[HSMService] = None


def get_hsm() -> HSMService:
    """
    Get or create the global HSM instance.

    Returns:
        Initialized HSM service
    """
    global _hsm_instance
    if _hsm_instance is None:
        _hsm_instance = HSMService()
    return _hsm_instance


def init_hsm() -> HSMService:
    """
    Initialize the global HSM instance.

    This should be called once at application startup.

    Returns:
        Initialized HSM service
    """
    global _hsm_instance
    _hsm_instance = HSMService()
    return _hsm_instance


def close_hsm():
    """Close and cleanup the global HSM instance."""
    global _hsm_instance
    if _hsm_instance:
        _hsm_instance.close()
        _hsm_instance = None
