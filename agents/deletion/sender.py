"""
Sends one rendered deletion request by SMTP. Nothing here decides who to send to: the address is
the one the backend took from its evidence-checked catalog, and it is refused unless it is a
single plain email address.
"""
from __future__ import annotations

from email.mime.text import MIMEText
from email.utils import make_msgid

import structlog

from deletion.message import is_plain_address, one_line, render
from removal.executor import RemovalExecutor
from shared.models import IdentityContext, QueuedDeletion

log = structlog.get_logger(__name__)


class DeletionSender:
    def __init__(self, executor: RemovalExecutor | None = None) -> None:
        self._executor = executor or RemovalExecutor()
        self._config = self._executor.config

    async def send(self, request: QueuedDeletion, identity: IdentityContext) -> tuple[bool, str | None, str | None]:
        """Returns (sent, message_id, error). The error is a fixed phrase: never exception text,
        which can carry addresses."""
        if not is_plain_address(request.to_address):
            return False, None, "The company's address was not a single plain email address, so nothing was sent."
        if not identity.full_name.strip() or not (identity.email or "").strip():
            return False, None, "The customer's name or email address was missing, so nothing was sent."
        if not self._config.smtp_username or not self._config.smtp_password:
            return False, None, "Email sending is not configured."
        sender = self._config.smtp_from_email or self._config.smtp_username
        if not is_plain_address(sender):
            return False, None, "The sending address is not configured correctly."

        subject, body = render(request, identity)
        msg = MIMEText(body, "plain", "utf-8")
        msg["Subject"] = subject
        msg["From"] = sender
        msg["To"] = request.to_address
        msg["Reply-To"] = one_line(identity.email, 200)
        msg["Message-ID"] = make_msgid(domain=sender.rpartition("@")[2])

        try:
            message_id = await self._executor._send_smtp_email(msg, request.to_address)
        except Exception as exc:
            log.error("deletion_email_failed", channel=request.channel_id, error_type=type(exc).__name__)
            return False, None, f"The email could not be sent ({type(exc).__name__})."
        log.info("deletion_email_sent", channel=request.channel_id, to_domain=request.to_address.rpartition("@")[2])
        return True, message_id, None
