# Data Retention

This document states what the system does today, what it does not, and the open design problem. An earlier version described a daily automated cleanup (inactive-account warnings and deletion, exposure and scan expiry, and so on). **None of that ran.** The module (`services/data_retention.rs`) was never started from `main.rs`, and was deleted because, as written, it could not have worked and an automatic deleter of customer data is too risky to ship unverified.

## What runs today

The scan recovery loop (`jobs/recovery.rs`) performs two housekeeping deletes:

| Data | Rule |
|------|------|
| `login_throttle` rows | Purged once their lockout window has expired |
| `privacy_score_history` | Rows older than 1 year are deleted |

Nothing else is ever deleted automatically: customers, identity profiles, scan jobs, exposures, removal requests, alerts, `audit_log`, and `removal_audit_log` are kept indefinitely.

## What does not exist

- Account deletion (no endpoint, no job).
- Right-to-erasure / data-subject deletion workflow.
- Inactive-account warnings or deletion.
- Expiry of scan jobs, exposures, or alerts.
- Application-log, backup, or Prometheus retention (these belong to the deployment platform).

For a service whose purpose is data protection, this is a gap that has to be closed before launch.

## The open design problem

Deleting a customer must remove their PII, but the audit trail is meant to be immutable. They conflict in the current schema:

- `removal_audit_log` has triggers/rules that block UPDATE and DELETE, and its `customer_id` column references `customers(id)` without `ON DELETE CASCADE`/`SET NULL`. Deleting any customer who has a removal request would fail.
- `audit_log` (migration 0004) uses `ON DELETE SET NULL`, and the immutability triggers from migration 0007 deliberately allow exactly that change (clearing `customer_id` / `user_email`), so it can be anonymized without being rewritten.
- `removal_audit_log.removal_request_id` also references `removal_requests(id)` without cascade, so even cascading the customer's removal requests is blocked while their audit rows exist. The other customer-owned tables cascade from `customers`.

A workable approach, not yet built:

1. A new migration (never edit applied ones) changes `removal_audit_log.customer_id` to nullable with `ON DELETE SET NULL` and the `removal_request_id` reference so the request rows can go (or keeps request rows in anonymized form), and extends the immutability trigger to allow only that anonymizing change, as was done for `audit_log`.
2. Check what `removal_audit_log.details` or similar free-text columns may contain; anything identifying must be scrubbed or never written.
3. A `DELETE /me` endpoint (re-authenticated, audited) performs the deletion in one transaction in dependency order, and writes a final anonymized audit entry.
4. A tested erasure of backups follows the platform's backup policy.
5. Decide retention for inactive accounts and publish it in the privacy policy before implementing any automatic deletion. Automatic deletion should warn first and be tested against a copy of production data.

## Intended policy (a proposal, not behaviour)

| Data | Proposed retention |
|------|--------------------|
| Active account and identity profile | While the account exists |
| After account deletion | PII removed immediately; audit entries kept in anonymized form |
| Removed exposures and completed removal requests | To be decided by counsel (evidence value vs minimization) |
| Security audit log | Long (the earlier draft said 7 years); anonymized after account deletion |
| Privacy score history | 1 year (implemented) |

Retention periods and legal bases (GDPR, CCPA) are decisions for the business and counsel; none are encoded in the software yet.
