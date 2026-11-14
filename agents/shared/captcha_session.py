"""
CAPTCHA Session Manager - Phase 3

Handles paused browser sessions for human-in-the-loop CAPTCHA solving.
When automation encounters a CAPTCHA, the browser session is persisted,
the customer is notified, and they can solve it in an embedded browser view.
"""
from __future__ import annotations

import asyncio
import uuid
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Optional

import structlog
from playwright.async_api import Browser, BrowserContext, Page

from shared.models import CaptchaSession, CaptchaSessionStatus

log = structlog.get_logger(__name__)


class CaptchaSessionManager:
    """
    Manages paused browser sessions for CAPTCHA intervention.

    Workflow:
    1. Agent detects CAPTCHA during automated removal
    2. Agent calls pause_for_captcha() to save browser state
    3. Customer is notified via dashboard/email
    4. Customer accesses embedded browser via session URL
    5. Customer solves CAPTCHA
    6. Customer marks session as solved
    7. Agent resumes automation from saved state
    """

    def __init__(self, storage_dir: str = "captcha_sessions"):
        self.storage_dir = Path(storage_dir)
        self.storage_dir.mkdir(parents=True, exist_ok=True)
        log.info("captcha_session_manager_initialized", storage_dir=str(self.storage_dir))

    async def pause_for_captcha(
        self,
        customer_id: uuid.UUID,
        removal_request_id: uuid.UUID,
        broker: str,
        context: BrowserContext,
        page: Page,
        screenshot_bytes: bytes,
    ) -> CaptchaSession:
        """
        Pause automation at CAPTCHA and create intervention session.

        Args:
            customer_id: Customer UUID
            removal_request_id: Removal request UUID
            broker: Broker name (e.g., "nuwber.com")
            context: Playwright browser context to persist
            page: Playwright page showing CAPTCHA
            screenshot_bytes: Screenshot of CAPTCHA page

        Returns:
            CaptchaSession with session_url for customer access
        """
        session_id = uuid.uuid4()
        now = datetime.now(timezone.utc)
        expires_at = now + timedelta(minutes=30)

        # Save browser context to disk
        context_path = self.storage_dir / f"{session_id}_context"
        await context.storage_state(path=str(context_path))
        log.info(
            "browser_context_saved",
            session_id=str(session_id),
            context_path=str(context_path),
        )

        # Save screenshot for evidence
        screenshot_path = self.storage_dir / f"{session_id}_captcha.png"
        screenshot_path.write_bytes(screenshot_bytes)

        # Generate screenshot hash
        import hashlib
        screenshot_hash = hashlib.sha256(screenshot_bytes).hexdigest()

        # Get current page URL
        page_url = page.url

        # Generate session URL for customer access
        # In production: this would be a secure dashboard URL
        session_url = f"https://app.vectorscrub.com/captcha-assist/{session_id}"

        session = CaptchaSession(
            session_id=session_id,
            customer_id=customer_id,
            removal_request_id=removal_request_id,
            broker=broker,
            status=CaptchaSessionStatus.WAITING_FOR_CUSTOMER,
            session_url=session_url,
            created_at=now.isoformat(),
            expires_at=expires_at.isoformat(),
            browser_context_path=str(context_path),
            page_url=page_url,
            screenshot_hash=screenshot_hash,
        )

        # Save session metadata
        session_metadata_path = self.storage_dir / f"{session_id}_session.json"
        session_metadata_path.write_text(session.model_dump_json(indent=2))

        log.info(
            "captcha_session_created",
            session_id=str(session_id),
            broker=broker,
            expires_at=expires_at.isoformat(),
        )

        return session

    async def resume_from_captcha(
        self,
        session_id: uuid.UUID,
        browser: Browser,
    ) -> tuple[BrowserContext, Page]:
        """
        Resume automation after customer solved CAPTCHA.

        Args:
            session_id: Session UUID
            browser: Playwright browser instance

        Returns:
            Tuple of (context, page) restored from saved state

        Raises:
            FileNotFoundError: If session doesn't exist
            ValueError: If session expired or not solved
        """
        # Load session metadata
        session_metadata_path = self.storage_dir / f"{session_id}_session.json"
        if not session_metadata_path.exists():
            raise FileNotFoundError(f"Session {session_id} not found")

        session_data = session_metadata_path.read_text()
        session = CaptchaSession.model_validate_json(session_data)

        # Check session status
        if session.status != CaptchaSessionStatus.SOLVED:
            raise ValueError(
                f"Session {session_id} not ready to resume (status: {session.status})"
            )

        # Check expiration
        expires_at = datetime.fromisoformat(session.expires_at)
        if datetime.now(timezone.utc) > expires_at:
            session.status = CaptchaSessionStatus.EXPIRED
            session_metadata_path.write_text(session.model_dump_json(indent=2))
            raise ValueError(f"Session {session_id} expired")

        # Restore browser context from saved state
        context_path = Path(session.browser_context_path)
        if not context_path.exists():
            raise FileNotFoundError(f"Browser context not found: {context_path}")

        context = await browser.new_context(storage_state=str(context_path))

        # Open page at the URL where CAPTCHA was encountered
        page = await context.new_page()
        await page.goto(session.page_url)

        log.info(
            "captcha_session_resumed",
            session_id=str(session_id),
            broker=session.broker,
        )

        return context, page

    async def mark_solved(self, session_id: uuid.UUID) -> None:
        """
        Mark session as solved by customer.

        Called by the dashboard when customer completes CAPTCHA.
        """
        session_metadata_path = self.storage_dir / f"{session_id}_session.json"
        if not session_metadata_path.exists():
            raise FileNotFoundError(f"Session {session_id} not found")

        session_data = session_metadata_path.read_text()
        session = CaptchaSession.model_validate_json(session_data)
        session.status = CaptchaSessionStatus.SOLVED
        session_metadata_path.write_text(session.model_dump_json(indent=2))

        log.info("captcha_session_marked_solved", session_id=str(session_id))

    async def mark_failed(self, session_id: uuid.UUID, reason: str = "") -> None:
        """
        Mark session as failed (customer couldn't solve).

        Called by the dashboard when customer reports they can't solve it.
        """
        session_metadata_path = self.storage_dir / f"{session_id}_session.json"
        if not session_metadata_path.exists():
            raise FileNotFoundError(f"Session {session_id} not found")

        session_data = session_metadata_path.read_text()
        session = CaptchaSession.model_validate_json(session_data)
        session.status = CaptchaSessionStatus.FAILED
        session_metadata_path.write_text(session.model_dump_json(indent=2))

        log.warning(
            "captcha_session_marked_failed",
            session_id=str(session_id),
            reason=reason,
        )

    async def cleanup_expired_sessions(self) -> int:
        """
        Clean up expired sessions (older than 30 minutes).

        Returns:
            Number of sessions cleaned up
        """
        cleaned = 0
        now = datetime.now(timezone.utc)

        for session_file in self.storage_dir.glob("*_session.json"):
            try:
                session_data = session_file.read_text()
                session = CaptchaSession.model_validate_json(session_data)

                expires_at = datetime.fromisoformat(session.expires_at)
                if now > expires_at:
                    # Mark as expired
                    session.status = CaptchaSessionStatus.EXPIRED
                    session_file.write_text(session.model_dump_json(indent=2))

                    # Delete browser context and screenshot
                    context_path = Path(session.browser_context_path)
                    if context_path.exists():
                        context_path.unlink()

                    screenshot_path = self.storage_dir / f"{session.session_id}_captcha.png"
                    if screenshot_path.exists():
                        screenshot_path.unlink()

                    cleaned += 1
                    log.info(
                        "expired_session_cleaned",
                        session_id=str(session.session_id),
                    )
            except Exception as exc:
                log.error(
                    "session_cleanup_error",
                    session_file=str(session_file),
                    error_type=type(exc).__name__,
                )

        return cleaned

    async def get_pending_sessions(self, customer_id: uuid.UUID) -> list[CaptchaSession]:
        """
        Get all pending CAPTCHA sessions for a customer.

        Used by dashboard to show active interventions.
        """
        sessions = []
        for session_file in self.storage_dir.glob("*_session.json"):
            try:
                session_data = session_file.read_text()
                session = CaptchaSession.model_validate_json(session_data)

                if (
                    session.customer_id == customer_id
                    and session.status in (
                        CaptchaSessionStatus.WAITING_FOR_CUSTOMER,
                        CaptchaSessionStatus.CUSTOMER_SOLVING,
                    )
                ):
                    sessions.append(session)
            except Exception as exc:
                log.error(
                    "session_load_error",
                    session_file=str(session_file),
                    error_type=type(exc).__name__,
                )

        return sessions


# Singleton instance
_captcha_session_manager: Optional[CaptchaSessionManager] = None


def get_captcha_session_manager() -> CaptchaSessionManager:
    """Get singleton instance of CaptchaSessionManager."""
    global _captcha_session_manager
    if _captcha_session_manager is None:
        _captcha_session_manager = CaptchaSessionManager()
    return _captcha_session_manager
