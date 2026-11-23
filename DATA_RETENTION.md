# Data Retention

This document states what the system does today, what it does not, and the open design problem. An earlier version described a daily automated cleanup (inactive-account warnings and deletion, exposure and scan expiry, and so on). **None of that ran.** The module (`services/data_retention.rs`) was never started from `main.rs`, and was deleted because, as written, it could not have worked and an automatic deleter of customer data is too risky to ship unverified.

No screenshots of filled-in opt-out forms are taken or stored (they would show the customer's identity), so there is no evidence store to erase from. An earlier file-based store was removed.

## Account erasure (implemented, migration 0017)

`DELETE /api/v1/me` (password required, refused while a subscription would keep billing; one set to cancel at period end is allowed) and the operator command `vectorscrub erase-account <email>` call `services/account_deletion.rs`. In one transaction it pseudonymizes the customer's rows in `audit_log` (customer, email, IP, user agent cleared, including failed-login rows that carry only the email) and `removal_audit_log` (customer and request links cleared, free-text `details` reset to `{}`), then deletes the customer; everything the customer owns cascades. `stripe_subscriptions` keeps Stripe's identifiers with the customer link cut. The audit triggers still reject every other UPDATE, all DELETEs and TRUNCATE (tested).

Not covered: `audit_log.details` and `error_message` are not rewritten (they hold no email or name by convention, which is not enforced by the database); backups keep an erased customer until they age out (`BACKUP_KEEP_DAYS`, default 30), and restoring one brings the customer back, so erasures made after the backup must be repeated (procedure in `docs/runbooks/backup-and-restore.md`); inactive-account deletion still does not exist; an operator must confirm a written erasure request really comes from the account owner before running the command.

## What runs today

The scan recovery loop (`jobs/recovery.rs`) performs three housekeeping deletes:

| Data | Rule |
|------|------|
| `login_throttle` rows | Purged once their lockout window has expired |
| `privacy_score_history` | Rows older than 1 year are deleted |
| `sessions` | Rows that expired or were revoked more than 7 days ago |

Two-factor secrets, recovery codes and sessions belong to the customer and are removed with the account by the same cascade as everything else (tested).

| `audit_log` identifiers | After `AUDIT_IDENTIFIER_RETENTION_DAYS` (default 180) the email, IP address and user agent are cleared from the row. The row (what happened, when, result, and the customer link while the account exists) is kept. Failed logins for emails that never had an account stop being personal data this way. |

Nothing else is ever deleted automatically: customers, identity profiles, scan jobs, exposures, removal requests, alerts, `audit_log`, and `removal_audit_log` are kept indefinitely.

## What does not exist

- Inactive-account warnings or deletion.
- Expiry of scan jobs, exposures, or alerts.
- Application-log, backup, or Prometheus retention (these belong to the deployment platform).

Deciding how long an inactive account is kept is a policy choice, and the Privacy Policy must state it before any automatic deletion is built.

## How erasure and the append-only audit trail were reconciled (done, migration 0017)

Deleting a customer must remove their personal data, but the audit trail must not be editable. The schema now does both:

- `audit_log.customer_id` is `ON DELETE SET NULL`, and the immutability triggers permit exactly one kind of change: clearing the customer link, email, IP address and user agent (pseudonymization). Every other UPDATE, all DELETEs and TRUNCATE are rejected.
- `removal_audit_log` gained the same exception, and its references to customers and removal requests no longer block deleting them.
- `DELETE /me` and `vectorscrub erase-account` do the whole thing in one transaction (`services/account_deletion.rs`): pseudonymize the audit rows, then delete the customer, then everything the customer owns cascades.
- Free-text columns: `removal_audit_log.details` is reset to `{}` on erasure. `audit_log.details` and `error_message` are not rewritten; by convention they hold no names or emails, which the database does not enforce.

Still open: erasure from backups (they age out after `BACKUP_KEEP_DAYS`, and a restore resurrects accounts erased after the backup), and what to do about inactive accounts (below).

## Intended policy (a proposal, not behaviour)

| Data | Proposed retention |
|------|--------------------|
| Active account and identity profile | While the account exists |
| After account deletion | PII removed immediately; audit entries kept in anonymized form |
| Removed exposures and completed removal requests | To be decided by counsel (evidence value vs minimization) |
| Security audit log | Long (the earlier draft said 7 years); anonymized after account deletion |
| Privacy score history | 1 year (implemented) |

Retention periods and legal bases (GDPR, CCPA) are decisions for the business and counsel; none are encoded in the software yet.
