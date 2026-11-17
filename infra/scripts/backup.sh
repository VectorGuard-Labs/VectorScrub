#!/usr/bin/env bash
# Back up the VectorScrub database.
#
#   DATABASE_URL=postgres://... BACKUP_GPG_RECIPIENT=backups@example.com infra/scripts/backup.sh
#
# What it does, in order: dump (pg_dump custom format) -> check the dump is readable and holds the
# tables that matter -> encrypt to a public key -> checksum -> copy off the machine (optional) ->
# delete local copies older than BACKUP_KEEP_DAYS -> ping a dead-man's-switch URL (optional).
# It exits non-zero at the first thing that goes wrong and never leaves a half-written backup
# under a final name.
#
# The dump holds account emails, password hashes, client IP addresses, the audit trail and all
# customer data as ciphertext. The master key is NOT in it, and without that key the ciphertext
# is useless (see docs/runbooks/backup-and-restore.md: losing the key means losing the data).
# So the dump is encrypted again to a key whose private half is NOT kept on the database host:
# someone who takes over that host then cannot read the backups it made.
#
# Settings (environment):
#   DATABASE_URL            libpq connection URL of the database to back up. It is passed to
#                           pg_dump as an argument, so it is visible in the process list on this
#                           machine. Leave it unset and use PGHOST/PGUSER/PGDATABASE with a
#                           ~/.pgpass file or PGPASSWORD if that matters to you.
#   BACKUP_GPG_RECIPIENT    key id or email of the PUBLIC key to encrypt to. Required, unless
#                           BACKUP_ALLOW_UNENCRYPTED=1 (for a throwaway local copy only).
#   BACKUP_DIR              where backups go (default ./backups, created with mode 700).
#   BACKUP_KEEP_DAYS        local retention in days (default 30, minimum 7).
#   BACKUP_S3_URI           e.g. s3://my-bucket/vectorscrub : also copy the backup there.
#   BACKUP_S3_SSE           server-side encryption for that copy (default AES256; "aws:kms" for KMS).
#   BACKUP_HEARTBEAT_URL    fetched after a fully successful run. Point it at a monitor that alerts
#                           when the ping stops arriving (healthchecks.io or similar), which is how
#                           you find out that backups silently stopped.
#   PG_BIN_DIR              directory holding pg_dump/pg_restore, when the ones on PATH are older
#                           than the server (pg_dump refuses to dump a newer server).
set -euo pipefail
umask 077

die() { echo "backup: $*" >&2; exit 1; }

BACKUP_DIR="${BACKUP_DIR:-./backups}"
KEEP_DAYS="${BACKUP_KEEP_DAYS:-30}"
PG_DUMP="${PG_BIN_DIR:+$PG_BIN_DIR/}pg_dump"
PG_RESTORE="${PG_BIN_DIR:+$PG_BIN_DIR/}pg_restore"

command -v "$PG_DUMP" >/dev/null || die "pg_dump not found (set PG_BIN_DIR)"
command -v "$PG_RESTORE" >/dev/null || die "pg_restore not found (set PG_BIN_DIR)"
[[ "$KEEP_DAYS" =~ ^[0-9]+$ ]] && (( KEEP_DAYS >= 7 )) || die "BACKUP_KEEP_DAYS must be a whole number of at least 7"
[[ -n "${DATABASE_URL:-}" || -n "${PGDATABASE:-}" ]] || die "set DATABASE_URL (or PGDATABASE and the other PG* variables)"

if [[ -z "${BACKUP_GPG_RECIPIENT:-}" && "${BACKUP_ALLOW_UNENCRYPTED:-0}" != "1" ]]; then
  die "BACKUP_GPG_RECIPIENT is required: the dump holds personal data and must not be stored in the clear (BACKUP_ALLOW_UNENCRYPTED=1 overrides, for throwaway copies only)"
fi
[[ -z "${BACKUP_GPG_RECIPIENT:-}" ]] || command -v gpg >/dev/null || die "gpg is needed to encrypt the backup"
[[ -z "${BACKUP_S3_URI:-}" ]] || command -v aws >/dev/null || die "the aws CLI is needed for BACKUP_S3_URI"

sha256_of() {
  if command -v sha256sum >/dev/null; then sha256sum "$1" | awk '{print $1}'; else shasum -a 256 "$1" | awk '{print $1}'; fi
}

mkdir -p "$BACKUP_DIR"
chmod 700 "$BACKUP_DIR"
stamp="$(date -u +%Y%m%dT%H%M%SZ)"
final="$BACKUP_DIR/vectorscrub-$stamp.dump"
partial="$BACKUP_DIR/.vectorscrub-$stamp.partial"
listing="$(mktemp)"
trap 'rm -f "$partial" "$partial.gpg" "$listing"' EXIT

echo "backup: dumping..."
dump_args=(--format=custom --compress=6 --no-owner --no-privileges --file="$partial")
[[ -z "${DATABASE_URL:-}" ]] || dump_args+=(--dbname="$DATABASE_URL")
"$PG_DUMP" "${dump_args[@]}" || die "pg_dump failed"

# A dump that cannot be read back, or that lacks the tables that matter, is not a backup.
"$PG_RESTORE" --list "$partial" > "$listing" || die "the dump cannot be read back"
for table in customers identity_profiles audit_log removal_audit_log; do
  grep -Eq "TABLE DATA public $table " "$listing" || die "the dump has no data section for $table"
done
grep -Eq "TABLE DATA public _sqlx_migrations " "$listing" || die "the dump has no migration history (is this a VectorScrub database?)"

if [[ -n "${BACKUP_GPG_RECIPIENT:-}" ]]; then
  echo "backup: encrypting..."
  gpg --batch --yes --quiet --trust-model always --recipient "$BACKUP_GPG_RECIPIENT" \
      --output "$partial.gpg" --encrypt "$partial" || die "encryption failed"
  rm -f "$partial"
  final="$final.gpg"
  mv "$partial.gpg" "$final"
else
  echo "backup: WARNING: writing an UNENCRYPTED backup" >&2
  mv "$partial" "$final"
fi

sum="$(sha256_of "$final")"
printf '%s  %s\n' "$sum" "$(basename "$final")" > "$final.sha256"
bytes="$(wc -c < "$final" | tr -d ' ')"

if [[ -n "${BACKUP_S3_URI:-}" ]]; then
  echo "backup: copying off the machine..."
  aws s3 cp --only-show-errors --sse "${BACKUP_S3_SSE:-AES256}" "$final" "${BACKUP_S3_URI%/}/" || die "upload failed"
  aws s3 cp --only-show-errors --sse "${BACKUP_S3_SSE:-AES256}" "$final.sha256" "${BACKUP_S3_URI%/}/" || die "upload of the checksum failed"
fi

# Local retention: only files this script names, only in this directory.
find "$BACKUP_DIR" -maxdepth 1 -type f \( -name 'vectorscrub-*.dump' -o -name 'vectorscrub-*.dump.gpg' -o -name 'vectorscrub-*.dump*.sha256' \) \
  -mtime "+$KEEP_DAYS" -delete

echo "backup: OK $final ($bytes bytes, sha256 $sum)"

if [[ -n "${BACKUP_HEARTBEAT_URL:-}" ]]; then
  curl -fsS -m 10 --retry 3 -o /dev/null "$BACKUP_HEARTBEAT_URL" || echo "backup: WARNING: could not reach the heartbeat URL" >&2
fi
