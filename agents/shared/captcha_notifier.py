"""
CAPTCHA Intervention Notifier - Phase 3

Notifies customers when CAPTCHA intervention is needed.
Sends real-time notifications via:
1. WebSocket (dashboard)
2. Email (fallback)
3. SMS (optional, for urgent cases)
"""
from __future__ import annotations

import asyncio
import smtplib
import ssl
from email.mime.multipart import MIMEMultipart
from email.mime.text import MIMEText
from typing import Optional

import structlog

from shared.models import CaptchaSession

log = structlog.get_logger(__name__)


class CaptchaNotifier:
    """
    Sends notifications when customer intervention is needed for CAPTCHA.

    Multi-channel approach ensures customer gets notified quickly:
    - WebSocket push to dashboard (instant if customer online)
    - Email notification (arrives within 1 minute)
    - SMS for Premium tier (optional, for urgent removals)
    """

    def __init__(
        self,
        smtp_host: str = "localhost",
        smtp_port: int = 587,
        smtp_user: Optional[str] = None,
        smtp_password: Optional[str] = None,
    ):
        self.smtp_host = smtp_host
        self.smtp_port = smtp_port
        self.smtp_user = smtp_user
        self.smtp_password = smtp_password

    async def notify_customer(
        self,
        session: CaptchaSession,
        customer_email: str,
        customer_name: str = "",
    ) -> dict[str, bool]:
        """
        Notify customer that CAPTCHA intervention is needed.

        Args:
            session: CaptchaSession with intervention details
            customer_email: Customer's email address
            customer_name: Customer's name (for personalization)

        Returns:
            Dict of channel -> success status
        """
        results = {
            "websocket": False,
            "email": False,
            "sms": False,
        }

        # Try WebSocket notification (instant if customer online)
        try:
            websocket_sent = await self._send_websocket_notification(session)
            results["websocket"] = websocket_sent
            if websocket_sent:
                log.info(
                    "captcha_notification_websocket_sent",
                    session_id=str(session.session_id),
                )
        except Exception as exc:
            log.warning(
                "captcha_notification_websocket_failed",
                session_id=str(session.session_id),
                error_type=type(exc).__name__,
            )

        # Always send email notification (guaranteed delivery)
        try:
            email_sent = await self._send_email_notification(
                session, customer_email, customer_name
            )
            results["email"] = email_sent
            if email_sent:
                log.info(
                    "captcha_notification_email_sent",
                    session_id=str(session.session_id),
                    customer_email=customer_email,
                )
        except Exception as exc:
            log.error(
                "captcha_notification_email_failed",
                session_id=str(session.session_id),
                error_type=type(exc).__name__,
            )

        return results

    async def _send_websocket_notification(self, session: CaptchaSession) -> bool:
        """
        Send real-time WebSocket notification to dashboard.

        In production: This would use a WebSocket server or pub/sub system
        (e.g., Redis Pub/Sub, Pusher, Socket.io) to push to connected clients.

        Returns:
            True if notification sent successfully
        """
        # TODO: Implement WebSocket notification
        # For now, log that it would be sent
        log.debug(
            "websocket_notification_placeholder",
            session_id=str(session.session_id),
            customer_id=str(session.customer_id),
            message="Real-time notification would be sent here",
        )
        return False  # Not implemented yet

    async def _send_email_notification(
        self,
        session: CaptchaSession,
        customer_email: str,
        customer_name: str = "",
    ) -> bool:
        """
        Send email notification with CAPTCHA assistance link.

        Returns:
            True if email sent successfully
        """
        # Create email message
        msg = MIMEMultipart("alternative")
        msg["Subject"] = f"Action Needed: Complete CAPTCHA for {session.broker}"
        msg["From"] = "VectorScrub <noreply@vectorscrub.com>"
        msg["To"] = customer_email

        # Plain text version
        text_body = f"""
Hi {customer_name or 'there'},

We encountered a CAPTCHA challenge while trying to remove your data from {session.broker}.

To complete the removal, please click the link below and solve the CAPTCHA:

{session.session_url}

This link will expire in 30 minutes. If you don't complete it in time, the removal
will be marked as requiring manual completion.

Why this happened:
Some data brokers use CAPTCHA challenges to prevent automated submissions. We pause
when we detect these and ask you to help us get past them. Once you solve the CAPTCHA,
we'll automatically continue with the removal.

Questions? Reply to this email or visit our help center.

Best regards,
The VectorScrub Team

---
This is an automated message from VectorScrub privacy protection service.
"""

        # HTML version
        html_body = f"""
<!DOCTYPE html>
<html>
<head>
    <style>
        body {{ font-family: Arial, sans-serif; line-height: 1.6; color: #333; }}
        .container {{ max-width: 600px; margin: 0 auto; padding: 20px; }}
        .header {{ background: #2563eb; color: white; padding: 20px; text-align: center; }}
        .content {{ padding: 20px; background: #f9fafb; }}
        .button {{ display: inline-block; padding: 12px 24px; background: #2563eb;
                   color: white; text-decoration: none; border-radius: 6px; margin: 20px 0; }}
        .footer {{ padding: 20px; font-size: 12px; color: #666; text-align: center; }}
        .alert {{ background: #fef3c7; border-left: 4px solid #f59e0b; padding: 12px; margin: 20px 0; }}
    </style>
</head>
<body>
    <div class="container">
        <div class="header">
            <h1>🔐 Action Needed: CAPTCHA Challenge</h1>
        </div>
        <div class="content">
            <p>Hi {customer_name or 'there'},</p>

            <p>We encountered a CAPTCHA challenge while trying to remove your data from <strong>{session.broker}</strong>.</p>

            <div class="alert">
                <strong>⏱️ Time sensitive:</strong> This link expires in 30 minutes
            </div>

            <p>To complete the removal, click the button below and solve the CAPTCHA:</p>

            <center>
                <a href="{session.session_url}" class="button">
                    Complete CAPTCHA Now →
                </a>
            </center>

            <h3>Why this happened</h3>
            <p>Some data brokers use CAPTCHA challenges to prevent automated submissions. We pause when we
            detect these and ask you to help us get past them. Once you solve the CAPTCHA, we'll automatically
            continue with the removal.</p>

            <h3>What to do</h3>
            <ol>
                <li>Click the button above</li>
                <li>Solve the CAPTCHA challenge on the page</li>
                <li>Click "Done" to let us know you completed it</li>
                <li>We'll resume the removal automatically</li>
            </ol>

            <p>If you have any questions, reply to this email or visit our help center.</p>

            <p>Best regards,<br>The VectorScrub Team</p>
        </div>
        <div class="footer">
            <p>This is an automated message from VectorScrub privacy protection service.</p>
            <p>Session ID: {session.session_id}</p>
        </div>
    </div>
</body>
</html>
"""

        msg.attach(MIMEText(text_body, "plain"))
        msg.attach(MIMEText(html_body, "html"))

        # Send email
        try:
            # Use local SMTP in development, external in production
            if self.smtp_host == "localhost":
                # Development: Log instead of sending
                log.debug(
                    "email_notification_logged",
                    to=customer_email,
                    subject=msg["Subject"],
                )
                return True
            else:
                # Production: Send via SMTP with TLS
                context = ssl.create_default_context()
                with smtplib.SMTP(self.smtp_host, self.smtp_port) as server:
                    server.starttls(context=context)
                    if self.smtp_user and self.smtp_password:
                        server.login(self.smtp_user, self.smtp_password)
                    server.send_message(msg)
                return True

        except Exception as exc:
            log.error(
                "email_send_failed",
                to=customer_email,
                error_type=type(exc).__name__,
            )
            return False


# Singleton instance
_captcha_notifier: Optional[CaptchaNotifier] = None


def get_captcha_notifier() -> CaptchaNotifier:
    """Get singleton instance of CaptchaNotifier."""
    global _captcha_notifier
    if _captcha_notifier is None:
        _captcha_notifier = CaptchaNotifier()
    return _captcha_notifier
