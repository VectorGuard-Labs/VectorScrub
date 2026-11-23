# Runbook: SMTP failures (email sent to companies)

## What SMTP is used for
The agents send email **to data brokers and companies on the customer's behalf**: email-based opt-outs
(`agents/removal/executor.py`, `submit_email_request`) and, when switched on (they are off by default),
deletion requests (`agents/deletion/sender.py`). **No email is sent to customers.** There are no
notification, verification or password-reset emails today, so an SMTP outage never stops a customer from
signing in or seeing their data.

## Symptoms
- Email-type removal requests end `failed` with "Email send failed" or "SMTP connection error".
- The agents' logs show `smtp_not_configured`, an authentication error or a TLS error (error types only; addresses and message bodies are never logged).

## Impact
Only requests whose procedure is `email` (and deletion requests, if enabled). The customer sees the request as failed and the
manual opt-out guide. Web-form removals are unaffected. A request that may already have been sent is never retried
automatically (a deletion request that is claimed is never re-sent).

## Diagnosis
1. `docker compose logs agents | grep -i smtp` for the error type.
2. Is it configured? `SMTP_USERNAME` and `SMTP_PASSWORD` must be set in the agents' environment (`SMTP_HOST`, `SMTP_PORT`, `SMTP_FROM_EMAIL`, `SMTP_USE_TLS` default to `smtp.gmail.com`, 587, empty, true).
3. Test the credentials from the host with a mail client or `openssl s_client -starttls smtp -connect <host>:587`. Do not paste the password into a ticket or chat.

## Resolution
- Authentication error: the provider revoked or rotated the credential. Issue a new one, update the agents' environment, `docker compose up -d agents`.
- Connection or timeout: the agents' network (`web`) must reach the provider on its submission port; check the provider's status page and the host firewall.
- Once recorded, a failed request is not re-sent by the system. The customer can authorize it again from the dashboard.

## Before sending real mail at scale
Sender identity (a domain you control with SPF, DKIM and DMARC), the postal address, and the Terms and Privacy wording
must be in place (`docs/COMPLIANCE_CHECKLIST.md`). If mail to **customers** is ever added (verification, password reset,
alerts), it needs the same plus an unsubscribe for anything that is marketing, and this runbook must be extended.
