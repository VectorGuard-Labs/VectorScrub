#!/usr/bin/env bash
# Back up the PyHSM keystore: the file that holds the key-encryption key.
#
#   PYHSM_KEYSTORE=/var/lib/pyhsm/keystore.enc BACKUP_GPG_RECIPIENT=keystore@example.com \
#   infra/scripts/backup_keystore.sh
#
# Why this matters more than the database backup: every customer's data key in the database is
# wrapped by a key that exists ONLY in this keystore. Lose the keystore (and every copy of it) and
# the whole database is unreadable, forever, whatever state its backups are in. Take this backup
#   * on a schedule (daily is fine: the file changes only when keys are created or rotated), and
#   * immediately after every `vectorscrub kms rotate`, which adds a key version to the file.
#
# The keystore file is already encrypted under the PyHSM master password, and this script encrypts
# it again to a GPG key, so a copy that leaks is two locks deep. Keep these three things apart, so
# no single person or system can open the data: (1) database backups, (2) keystore backups,
# (3) the PyHSM master password and IPC secret. Anyone holding all three can read everything.
#
# It copies the file (PyHSM writes it atomically by renaming, so a copy is never torn), checks the
# copy is the same bytes, encrypts, checksums, optionally uploads, prunes old local copies, and
# pings a heartbeat URL on success.
#
# Settings (environment):
#   PYHSM_KEYSTORE          path to the keystore file. Required.
#   BACKUP_GPG_RECIPIENT    public key to encrypt to. Required (BACKUP_ALLOW_UNENCRYPTED=1 only
#                           for a throwaway copy). Use a DIFFERENT key from the database backups.
#   KEYSTORE_BACKUP_DIR     where copies go (default ./keystore-backups, mode 700).
#   BACKUP_KEEP_DAYS        local retention in days (default 90, minimum 30). Longer than the
#                           database's on purpose: an old database backup needs the key version
#                           that was current when it was made, and the keystore keeps old versions.
#   BACKUP_S3_URI, BACKUP_S3_SSE, BACKUP_HEARTBEAT_URL   as in backup.sh.
set -euo pipefail
umask 077

die() { echo "backup-keystore: $*" >&2; exit 1; }

[[ -n "${PYHSM_KEYSTORE:-}" ]] || die "set PYHSM_KEYSTORE"
[[ -f "$PYHSM_KEYSTORE" ]] || die "no such file: $PYHSM_KEYSTORE"
[[ -s "$PYHSM_KEYSTORE" ]] || die "the keystore file is empty: $PYHSM_KEYSTORE"
KEEP_DAYS="${BACKUP_KEEP_DAYS:-90}"
[[ "$KEEP_DAYS" =~ ^[0-9]+$ ]] && (( KEEP_DAYS >= 30 )) || die "BACKUP_KEEP_DAYS must be a whole number of at least 30"
if [[ -z "${BACKUP_GPG_RECIPIENT:-}" && "${BACKUP_ALLOW_UNENCRYPTED:-0}" != "1" ]]; then
  die "BACKUP_GPG_RECIPIENT is required (BACKUP_ALLOW_UNENCRYPTED=1 overrides, for throwaway copies only)"
fi
[[ -z "${BACKUP_GPG_RECIPIENT:-}" ]] || command -v gpg >/dev/null || die "gpg is needed"
[[ -z "${BACKUP_S3_URI:-}" ]] || command -v aws >/dev/null || die "the aws CLI is needed for BACKUP_S3_URI"

sha256_of() {
  if command -v sha256sum >/dev/null; then sha256sum "$1" | awk '{print $1}'; else shasum -a 256 "$1" | awk '{print $1}'; fi
}

dir="${KEYSTORE_BACKUP_DIR:-./keystore-backups}"
mkdir -p "$dir"; chmod 700 "$dir"
stamp="$(date -u +%Y%m%dT%H%M%SZ)"
final="$dir/pyhsm-keystore-$stamp.enc"
partial="$dir/.pyhsm-keystore-$stamp.partial"
trap 'rm -f "$partial" "$partial.gpg"' EXIT

cp "$PYHSM_KEYSTORE" "$partial"
[[ "$(sha256_of "$PYHSM_KEYSTORE")" == "$(sha256_of "$partial")" ]] \
  || die "the keystore changed while it was being copied; run it again"

if [[ -n "${BACKUP_GPG_RECIPIENT:-}" ]]; then
  gpg --batch --yes --quiet --trust-model always --recipient "$BACKUP_GPG_RECIPIENT" \
      --output "$partial.gpg" --encrypt "$partial" || die "encryption failed"
  rm -f "$partial"
  final="$final.gpg"
  mv "$partial.gpg" "$final"
else
  echo "backup-keystore: WARNING: writing an UNENCRYPTED copy (it is still encrypted by the PyHSM password)" >&2
  mv "$partial" "$final"
fi

sum="$(sha256_of "$final")"
printf '%s  %s\n' "$sum" "$(basename "$final")" > "$final.sha256"

if [[ -n "${BACKUP_S3_URI:-}" ]]; then
  aws s3 cp --only-show-errors --sse "${BACKUP_S3_SSE:-AES256}" "$final" "${BACKUP_S3_URI%/}/" || die "upload failed"
  aws s3 cp --only-show-errors --sse "${BACKUP_S3_SSE:-AES256}" "$final.sha256" "${BACKUP_S3_URI%/}/" || die "upload of the checksum failed"
fi

find "$dir" -maxdepth 1 -type f \( -name 'pyhsm-keystore-*.enc' -o -name 'pyhsm-keystore-*.enc.gpg' -o -name 'pyhsm-keystore-*.sha256' \) \
  -mtime "+$KEEP_DAYS" -delete

echo "backup-keystore: OK $final (sha256 $sum)"
if [[ -n "${BACKUP_HEARTBEAT_URL:-}" ]]; then
  curl -fsS -m 10 --retry 3 -o /dev/null "$BACKUP_HEARTBEAT_URL" || echo "backup-keystore: WARNING: could not reach the heartbeat URL" >&2
fi
