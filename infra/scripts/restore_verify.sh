#!/usr/bin/env bash
# Restore a backup into a scratch database and prove it is usable.
#
#   RESTORE_ADMIN_URL=postgres://postgres:pw@scratch-host:5432/postgres \
#   MASTER_ENCRYPTION_KEY=... \
#   infra/scripts/restore_verify.sh backups/vectorscrub-20261109T020000Z.dump.gpg
#
# A backup you have never restored is a hope, not a backup. This script is the drill: it checks
# the file, restores it into a NEW database it creates (it never touches an existing one, so it is
# safe to point at any server where you may create databases, but use a scratch server anyway),
# then runs `vectorscrub verify-backup` against the copy. That last step is the one that
# matters: it opens a sample of encrypted rows with the master key you give it, so it proves
# that this backup and this key belong together.
#
# It reports how long the restore took (your real recovery time, to compare with the target) and
# the newest audit entry in the backup (how much you would have lost), then drops the copy.
#
# Settings (environment):
#   RESTORE_ADMIN_URL         libpq URL of a Postgres server where a database may be created.
#                             Its database part is only used to connect (normally "postgres").
#   MASTER_ENCRYPTION_KEY     the master key the backup was made under. Required, unless
#                             RESTORE_SKIP_DECRYPT_CHECK=1, in which case the result is only
#                             "restores", NOT "usable", and the script says so.
#   VECTORSCRUB_BIN           the backend binary (default: `vectorscrub` on PATH, then
#                             backend/target/release/vectorscrub, then .../debug/vectorscrub).
#   PG_BIN_DIR                directory holding psql/pg_restore, if PATH has older ones.
#   GNUPGHOME                 keyring holding the PRIVATE key, for .gpg backups.
#   KEEP_RESTORED=1           leave the restored database in place (its name is printed).
set -euo pipefail
umask 077

die() { echo "restore-verify: $*" >&2; exit 1; }

[[ $# -eq 1 ]] || die "usage: restore_verify.sh BACKUP_FILE"
backup="$1"
[[ -f "$backup" ]] || die "no such file: $backup"
[[ -n "${RESTORE_ADMIN_URL:-}" ]] || die "set RESTORE_ADMIN_URL (a server where a scratch database may be created)"
if [[ "${RESTORE_SKIP_DECRYPT_CHECK:-0}" != "1" && -z "${MASTER_ENCRYPTION_KEY:-}" ]]; then
  die "set MASTER_ENCRYPTION_KEY (or RESTORE_SKIP_DECRYPT_CHECK=1 to only test that the file restores)"
fi

PSQL="${PG_BIN_DIR:+$PG_BIN_DIR/}psql"
PG_RESTORE="${PG_BIN_DIR:+$PG_BIN_DIR/}pg_restore"
command -v "$PSQL" >/dev/null || die "psql not found (set PG_BIN_DIR)"
command -v "$PG_RESTORE" >/dev/null || die "pg_restore not found (set PG_BIN_DIR)"

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
bin="${VECTORSCRUB_BIN:-}"
if [[ -z "$bin" ]]; then
  for candidate in "$(command -v vectorscrub || true)" \
                   "$script_dir/../../backend/target/release/vectorscrub" \
                   "$script_dir/../../backend/target/debug/vectorscrub"; do
    if [[ -n "$candidate" && -x "$candidate" ]]; then bin="$candidate"; break; fi
  done
fi
if [[ "${RESTORE_SKIP_DECRYPT_CHECK:-0}" != "1" ]]; then
  [[ -n "$bin" && -x "$bin" ]] || die "the vectorscrub binary was not found (set VECTORSCRUB_BIN, or build it with: cd backend && cargo build --release)"
fi

sha256_of() {
  if command -v sha256sum >/dev/null; then sha256sum "$1" | awk '{print $1}'; else shasum -a 256 "$1" | awk '{print $1}'; fi
}

work="$(mktemp -d)"
dbname="vectorscrub_restore_$(date -u +%Y%m%d%H%M%S)_$RANDOM"
created=0
cleanup() {
  rm -rf "$work"
  if [[ $created -eq 1 && "${KEEP_RESTORED:-0}" != "1" ]]; then
    "$PSQL" "$RESTORE_ADMIN_URL" -qAt -c "DROP DATABASE IF EXISTS \"$dbname\" WITH (FORCE)" >/dev/null 2>&1 || \
      echo "restore-verify: WARNING: could not drop $dbname; drop it by hand" >&2
  fi
}
trap cleanup EXIT

echo "restore-verify: file     $backup"

# 1. The file is what was written.
if [[ -f "$backup.sha256" ]]; then
  want="$(awk '{print $1}' "$backup.sha256")"
  [[ "$(sha256_of "$backup")" == "$want" ]] || die "CHECKSUM MISMATCH: the file is not what was backed up"
  echo "restore-verify: checksum matches"
else
  echo "restore-verify: WARNING: no $backup.sha256 next to the file; integrity not checked" >&2
fi

# 2. Decrypt, if it is encrypted.
dump="$backup"
if [[ "$backup" == *.gpg ]]; then
  command -v gpg >/dev/null || die "gpg is needed to decrypt the backup"
  dump="$work/backup.dump"
  gpg --batch --yes --quiet --output "$dump" --decrypt "$backup" || die "decryption failed (is the private key in GNUPGHOME?)"
  echo "restore-verify: decrypted"
fi

# 3. Restore into a new, uniquely named database.
base="${RESTORE_ADMIN_URL%%\?*}"
query="${RESTORE_ADMIN_URL#"$base"}"
restored_url="${base%/*}/$dbname$query"
"$PSQL" "$RESTORE_ADMIN_URL" -qAt -c "CREATE DATABASE \"$dbname\"" >/dev/null || die "could not create the scratch database"
created=1

started="$(date +%s)"
"$PG_RESTORE" --exit-on-error --no-owner --no-privileges --dbname="$restored_url" "$dump" || die "pg_restore failed"
restore_secs=$(( $(date +%s) - started ))
echo "restore-verify: restored into $dbname in ${restore_secs}s"

# 4. Is it usable?
if [[ "${RESTORE_SKIP_DECRYPT_CHECK:-0}" == "1" ]]; then
  echo "restore-verify: RESULT: restores. NOT verified usable: the master key was not checked against it." >&2
  exit 0
fi
echo "restore-verify: verifying..."
if DATABASE_URL="$restored_url" "$bin" verify-backup | sed 's/^/restore-verify:   /'; then
  echo "restore-verify: OK (restore took ${restore_secs}s)"
  echo "restore-verify: for the drill log: | $(date -u +%Y-%m-%d) | $(basename "$backup") | ${restore_secs}s | usable |"
else
  die "NOT USABLE: see the problems above"
fi
