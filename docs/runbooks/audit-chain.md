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
docker compose exec backend ./vectorscrub audit check-head <seq> <hash>   # a stored head is still in the chain
```
Both only read, so the runtime role can run them.

## The part that makes it worth having: the head stored somewhere else
An owner can recompute every link after an edit, or cut links off the end, and the chain still verifies.
Only a head stored earlier, off the host, shows it. `infra/scripts/audit_head.sh` does this daily
(install: `infra/backup/README.md`, step 5):
1. `vectorscrub audit verify`: the chain holds.
2. `vectorscrub audit check-head <seq> <hash>` with the head stored by the previous run: it is still in the chain.
3. `vectorscrub audit head`, stored as a new object under `audit-heads/` in the off-site bucket (Object Lock keeps it).
4. A heartbeat ping, only if all of that succeeded.

A failure prints `AUDIT CHAIN BROKEN` and skips the ping: treat it as an incident (`incident-response.md`,
"Audit trail altered"). Tested on staging (2026-11-24): an owner disabled the triggers, deleted the newest audit row and its
link, and re-enabled them; `audit verify` alone still said the chain holds, and the job's next run failed with
"the head stored on ... is gone".

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
