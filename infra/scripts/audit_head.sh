#!/usr/bin/env bash
# The daily audit-head job (docs/runbooks/audit-chain.md).
#
#   infra/scripts/audit_head.sh
#
# 1. `vectorscrub audit verify`: the whole hash chain holds.
# 2. `vectorscrub audit check-head` with the head stored by the previous run: that head is still in
#    the chain. This is what catches an owner who rewrote or cut the chain consistently, which a
#    chain that verifies on its own cannot show.
# 3. `vectorscrub audit head`, stored as a new object in the off-site bucket, under a prefix the
#    backup host can only write (Object Lock in compliance mode keeps it for 35 days; see
#    infra/backup/aws-offsite-setup.sh). A copy is kept locally too, to compare against next time.
# 4. Ping the heartbeat URL, only after all of the above succeeded. A missing ping is the alert.
#
# Any failure exits non-zero, skips the ping, and prints AUDIT CHAIN BROKEN or the reason. Treat
# a broken chain as an incident (docs/runbooks/incident-response.md, "Audit trail altered").
#
# Settings (environment):
#   VECTORSCRUB_CMD       how to run the backend binary (default: `docker compose exec -T backend
#                         ./vectorscrub`, run from infra/docker). On a plain host: the binary path.
#                         Only reads the database; the runtime role is enough.
#   AUDIT_HEAD_DIR        local copies (default /var/lib/vectorscrub/audit-heads, mode 700)
#   AUDIT_HEAD_S3_URI     e.g. s3://<bucket>/audit-heads. Strongly recommended: a head kept only on
#                         the database host proves nothing against someone who controls that host.
#   AUDIT_HEAD_S3_SSE     server-side encryption (default AES256)
#   AUDIT_HEAD_HEARTBEAT_URL   pinged on success (a dead-man's switch alerts when it stops)
#   AUDIT_HEAD_TRUST_LOCAL=1   compare with the newest LOCAL copy even when S3 is set (default: the
#                         newest copy in S3 is the reference, because the local one is on the host)
set -euo pipefail
umask 077

die() { echo "audit-head: $*" >&2; exit 1; }
say() { echo "audit-head: $*"; }

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DIR="${AUDIT_HEAD_DIR:-/var/lib/vectorscrub/audit-heads}"
mkdir -p "$DIR"; chmod 700 "$DIR"

if [[ -n "${VECTORSCRUB_CMD:-}" ]]; then
  read -r -a vs <<<"$VECTORSCRUB_CMD"
else
  vs=(docker compose --project-directory "$here/../docker" exec -T backend ./vectorscrub)
fi
run() { "${vs[@]}" "$@" 2>&1 | grep -v '"level":' ; return "${PIPESTATUS[0]}"; }
[[ -z "${AUDIT_HEAD_S3_URI:-}" ]] || command -v aws >/dev/null || die "the aws CLI is needed for AUDIT_HEAD_S3_URI"

# 1. The chain holds.
out="$(run audit verify)" || { echo "$out" >&2; die "AUDIT CHAIN BROKEN (verify failed)"; }
say "$(tail -1 <<<"$out")"

# 2. The previous head is still in it.
previous=""
if [[ -n "${AUDIT_HEAD_S3_URI:-}" && "${AUDIT_HEAD_TRUST_LOCAL:-0}" != "1" ]]; then
  name="$(aws s3 ls "${AUDIT_HEAD_S3_URI%/}/" | awk '{print $4}' | grep -E '^audit-head-.*\.txt$' | sort | tail -1 || true)"
  if [[ -n "$name" ]]; then
    previous="$(aws s3 cp --only-show-errors "${AUDIT_HEAD_S3_URI%/}/$name" -)" || die "could not read the previous head from S3"
  fi
else
  last="$(ls -1 "$DIR"/audit-head-*.txt 2>/dev/null | sort | tail -1 || true)"
  [[ -z "$last" ]] || previous="$(cat "$last")"
fi
if [[ -n "$previous" ]]; then
  read -r _date pseq phash <<<"$previous"
  [[ "$pseq" =~ ^[0-9]+$ && "$phash" =~ ^[0-9a-f]{64}$ ]] || die "the previous head record is malformed: $previous"
  if ! out="$(run audit check-head "$pseq" "$phash")"; then
    echo "$out" >&2
    grep -q 'usage: vectorscrub audit verify | head$' <<<"$out" && die "the backend is too old for this job (no 'audit check-head'); deploy the current version"
    die "AUDIT CHAIN BROKEN (the head stored on $_date is gone)"
  fi
  say "$(tail -1 <<<"$out")"
else
  say "no previous head stored yet: this run starts the record"
fi

# 3. Store today's head.
head_line="$(run audit head | tail -1)" || die "could not read the head"
read -r seq hash <<<"$head_line"
stamp="$(date -u +%Y%m%dT%H%M%SZ)"
record="$(date -u +%FT%TZ) $seq $hash"
if [[ "$seq" == 0 ]]; then
  say "the chain is still empty; nothing to store"
else
  [[ "$seq" =~ ^[0-9]+$ && "$hash" =~ ^[0-9a-f]{64}$ ]] || die "unexpected head output: $head_line"
  file="$DIR/audit-head-$stamp.txt"
  printf '%s\n' "$record" > "$file"
  if [[ -n "${AUDIT_HEAD_S3_URI:-}" ]]; then
    aws s3 cp --only-show-errors --sse "${AUDIT_HEAD_S3_SSE:-AES256}" "$file" "${AUDIT_HEAD_S3_URI%/}/" || die "upload failed"
    say "stored head $seq off the host (${AUDIT_HEAD_S3_URI%/}/$(basename "$file"))"
  else
    say "WARNING: stored head $seq ONLY on this host. Set AUDIT_HEAD_S3_URI: a local copy proves nothing against whoever controls this host." >&2
  fi
fi

# 4. Heartbeat.
if [[ -n "${AUDIT_HEAD_HEARTBEAT_URL:-}" ]]; then
  curl -fsS -m 10 --retry 3 -o /dev/null "$AUDIT_HEAD_HEARTBEAT_URL" || say "WARNING: could not reach the heartbeat URL" >&2
fi
say "OK"
