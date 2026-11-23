# Runbook: the audit hash chain

## What it is
Every row written to `audit_log` or `removal_audit_log` after migration 0025 gets a link in `audit_chain`. Each link's hash is
`sha256(previous link's hash || the row's fixed fields)`: what happened (`action`, `resource_type`, `resource_id`, `status`, or
`action` and `actor` for removal rows), the row id, and its timestamp. No personal data is hashed and account erasure does not
touch these fields, so erasing a customer never breaks the chain.

Editing a covered row, deleting one, deleting or editing a link, or inserting a row with the chain trigger off all make
`vectorscrub audit verify` fail and name the first break. The triggers on the audit tables already stop the application; this is for the
case they do not cover, a database **owner** or superuser who switches the triggers off, edits, and switches them on again.

Rows written before migration 0025 are not covered. The chain starts at the first row after it.

## Commands
```bash
docker compose exec backend ./vectorscrub audit verify    # exit 0: "the audit chain holds"; non-zero: AUDIT CHAIN BROKEN: ...
docker compose exec backend ./vectorscrub audit head      # prints: <sequence> <hash>
```
Both only read, so the runtime role can run them.

## The part that makes it worth having: store the head somewhere else
An owner can recompute every link after an edit, so a chain that verifies proves the owner did not tamper **unless** you also
hold an earlier head. Once a day (the same schedule as the database backup is natural):
1. Run `audit verify`; if it fails, treat it as an incident (`incident-response.md`, "Audit trail altered").
2. Run `audit head` and append the line, with the date, to a place the database host cannot write: a bucket with object lock, a
   different provider's storage, an append-only mailbox, a ticket. Not the database host's own disk.

To check an old head later: the head you stored must still be the hash of a link in the current chain
(`SELECT seq, encode(link_hash,'hex') FROM audit_chain WHERE seq = <stored seq>`) and `audit verify` must hold. A cut-off tail or a
rewritten chain shows up as a stored head that is no longer there or no longer matches.

## Who can write the chain
Only the trigger, which runs as the owner (`SECURITY DEFINER`). The runtime role has read access and nothing else
(`infra/scripts/db_roles.sh` keeps it that way after a restore). The chain is also append-only: UPDATE, DELETE and TRUNCATE are rejected.

## Cost and limits
- Audit writes are serialized by one advisory lock held to the end of the transaction. That is fine at pilot volume; if audit writes
  ever become a bottleneck, shorten the transactions that write them first.
- It is not an external log. It makes tampering by the owner detectable only to the extent you keep heads off the host.
- The backup check (`vectorscrub verify-backup`) reports a missing chain trigger. `audit verify` is a separate step after a restore.
- Restoring an older backup rolls the chain back too; stored heads newer than the backup will then not be found. That is expected
  after a restore and should be noted in the incident log.
