"""
The email sent to a company asking it to delete a customer's data.

DRAFT WORDING: not reviewed by a lawyer, and it must be before this is used for real customers.
What the email does and does not contain is the part that is fixed by design:

  * It names the person, their email address and, only if they agreed, their address. It never
    contains phone numbers, a date of birth, or anything about other people.
  * It says who is writing (VectorScrub, as the person's authorized agent), what law is relied on,
    and quotes the authorization the person accepted, with the date and version, so the company can
    see what it is being asked to rely on.
  * Replies go to the CUSTOMER (Reply-To), who is also free to verify their identity with the company.
    VectorScrub does not read replies.

Every value that came from outside is flattened to one line, so a name or address cannot add a
header or a line to the message.
"""
from __future__ import annotations

import re
from datetime import datetime, timezone

from shared.models import IdentityContext, QueuedDeletion

_CONTROL = re.compile(r"[\x00-\x1f\x7f\u2028\u2029]+")

REQUEST_LINES = {
    "delete": "Delete the personal information you hold about them.",
    "opt_out_sale": "Stop selling or sharing their personal information.",
}

# A single plain email address: no spaces, commas, angle brackets or second address.
_PLAIN_ADDRESS = re.compile(r"^[a-z0-9._%+\-]+@[a-z0-9\-]+(\.[a-z0-9\-]+)+$")


def one_line(value: str | None, limit: int = 300) -> str:
    """Control characters and runs of whitespace become single spaces; capped in length."""
    return " ".join(_CONTROL.sub(" ", value or "").split())[:limit]


def is_plain_address(address: str) -> bool:
    return bool(_PLAIN_ADDRESS.match(address or "")) and address == address.strip()


def authorized_on(iso: str) -> str:
    """"November 5, 2026 at 09:05 UTC": the day without a leading zero, the time with one."""
    try:
        when = datetime.fromisoformat(iso.replace("Z", "+00:00"))
    except ValueError:
        return one_line(iso, 40)
    if when.tzinfo is not None:
        when = when.astimezone(timezone.utc)
    return f"{when:%B} {when.day}, {when.year} at {when:%H:%M} UTC"


def render(request: QueuedDeletion, identity: IdentityContext) -> tuple[str, str]:
    """(subject, plain-text body) for one request."""
    name = one_line(identity.full_name or f"{identity.first_name} {identity.last_name}", 200)
    email = one_line(identity.email or "", 200)
    company = one_line(request.company, 200)
    basis = one_line(request.legal_basis, 300)
    address = one_line(identity.current_address, 300) if request.include_address else ""

    asks = [REQUEST_LINES[r] for r in request.requests if r in REQUEST_LINES]
    asks_text = "\n".join(f"  {i}. {line}" for i, line in enumerate(asks, 1))

    details = [f"  Name: {name}", f"  Email: {email}"]
    if address:
        details.append(f"  Address: {address}")

    # The authorization is quoted line by line, so nothing in it can break out of the quote.
    quoted = "\n".join("  > " + one_line(line, 400) if line.strip() else "  >" for line in request.authorization_text.splitlines())

    body = f"""To the privacy team at {company},

I am writing as the authorized agent of {name}, through VectorScrub (a product of VectorGuard Labs). This request is made under {basis}.

On their behalf, please:
{asks_text}

So that you can find their records:
{chr(10).join(details)}

{name} authorized this request on {authorized_on(request.authorized_at)} (authorization version {one_line(request.authorization_version, 40)}). The authorization they accepted reads:

{quoted}

You may contact {name} directly at {email} to verify their identity or to confirm that they authorized this request. Please send your answer to that address.

Thank you,
VectorScrub, for {name}
"""
    if request.subject and one_line(request.subject, 100):
        subject = one_line(request.subject, 100)       # the company asked for this exact subject line
    elif "opt_out_sale" in request.requests:
        subject = f"Privacy request from {name}: deletion and opt-out of sale"
    else:
        subject = f"Privacy request from {name}: deletion"
    return one_line(subject, 200), body
