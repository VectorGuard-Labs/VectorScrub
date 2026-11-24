#!/usr/bin/env bash
# The full recovery drill: restore BOTH the database and the PyHSM keystore from their off-site
# copies, start a key service on the restored keystore, and prove that the restored data opens.
#
#   infra/scripts/restore_drill.sh
#
# What it does, in order, timing each step (the total is your measured recovery time for the data):
#   1. Fetch the newest database backup and the newest keystore backup from off-site storage, with
#      their checksums. (Or take local files: DRILL_DB_BACKUP / DRILL_KEYSTORE_BACKUP.)
#   2. Check both checksums. Decrypt both with the private keys in GNUPGHOME.
#   3. Start a scratch PyHSM container on the restored keystore: no network, its own socket volume,
#      its own IPC secret made for this drill. It needs the keystore PASSWORD, which is the one
#      secret this drill cannot invent (it comes from your secrets manager).
#   4. Start a scratch Postgres 16 container on its own network and restore the dump into it.
#   5. Run `vectorscrub verify-backup` (data keys unwrap through the restored key service, a sample of
#      every encrypted kind of value opens, the audit triggers are present) and `vectorscrub audit
#      verify` (the audit hash chain holds) against the restored copy.
#   6. Print the result and append one line to the drill log. Remove every scratch container,
#      volume and decrypted file, whatever happened.
#
# It never touches the live stack: different container names, a different network, no shared volume.
#
# Settings (environment):
#   DRILL_S3_DB_URI          e.g. s3://bucket/db         newest *.dump.gpg there is used
#   DRILL_S3_KEYSTORE_URI    e.g. s3://bucket/keystore   newest pyhsm-keystore-*.enc.gpg there is used
#   DRILL_S3_ENDPOINT        optional S3-compatible endpoint (MinIO, R2, ...). AWS needs none.
#                            The aws CLI uses its usual credentials: give it the READ-ONLY restore key.
#   DRILL_DB_BACKUP, DRILL_KEYSTORE_BACKUP   local files instead of S3 (each with its .sha256 beside it)
#   DRILL_PYHSM_PASSWORD_FILE  file holding the keystore password (mode 600). Required.
#   PYHSM_IMAGE              the PyHSM image, by digest (the same one production runs). Required.
#   BACKEND_IMAGE            the backend image to verify with (default vectorscrub-backend:local)
#   PYHSM_KEY_ID             default vectorscrub-kek
#   GNUPGHOME                keyring with BOTH private keys (database and keystore). Required for .gpg.
#   DRILL_LOG                where the result line is appended (default docs/runbooks/drill-log.md)
#   DRILL_ENVIRONMENT        free text for the log line (default "unspecified"), e.g. "staging, laptop"
set -euo pipefail
umask 077

die() { echo "restore-drill: $*" >&2; exit 1; }
say() { echo "restore-drill: $*"; }

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
BACKEND_IMAGE="${BACKEND_IMAGE:-vectorscrub-backend:local}"
KEY_ID="${PYHSM_KEY_ID:-vectorscrub-kek}"
DRILL_LOG="${DRILL_LOG:-$repo/docs/runbooks/drill-log.md}"
PG_IMAGE="postgres:16-alpine@sha256:f2f98726820d0342055351c6e9d36e705f5bfffb3dd53ff556325114b08afb6d"

command -v docker >/dev/null || die "docker is required"
[[ -n "${PYHSM_IMAGE:-}" ]] || die "set PYHSM_IMAGE (the PyHSM image production runs, by digest)"
[[ "$PYHSM_IMAGE" == *@sha256:* ]] || die "PYHSM_IMAGE must be referenced by digest (…@sha256:…)"
[[ -n "${DRILL_PYHSM_PASSWORD_FILE:-}" && -f "$DRILL_PYHSM_PASSWORD_FILE" ]] || die "set DRILL_PYHSM_PASSWORD_FILE to the keystore password file"
docker image inspect "$BACKEND_IMAGE" >/dev/null 2>&1 || die "no image $BACKEND_IMAGE (build it, or set BACKEND_IMAGE)"

sha256_of() {
  if command -v sha256sum >/dev/null; then sha256sum "$1" | awk '{print $1}'; else shasum -a 256 "$1" | awk '{print $1}'; fi
}
now() { date +%s; }

id="drill$(date -u +%Y%m%d%H%M%S)$RANDOM"
work="$(mktemp -d)"
net="${id}_net"; pgc="${id}_pg"; hsmc="${id}_pyhsm"; datav="${id}_ksdata"; sockv="${id}_kssock"
cleanup() {
  docker rm -f "$pgc" "$hsmc" >/dev/null 2>&1 || true
  docker volume rm -f "$datav" "$sockv" >/dev/null 2>&1 || true
  docker network rm "$net" >/dev/null 2>&1 || true
  rm -rf "$work"
}
trap cleanup EXIT

started="$(now)"
step() { local t; t="$(now)"; say "[$((t - started))s] $*"; }

# ---- 1. fetch ------------------------------------------------------------------------------------
aws_s3() { aws ${DRILL_S3_ENDPOINT:+--endpoint-url "$DRILL_S3_ENDPOINT"} s3 "$@"; }
newest() { # newest object name under an s3 prefix matching a pattern
  aws_s3 ls "${1%/}/" | awk '{print $4}' | grep -E "$2" | sort | tail -1
}
fetch() { # fetch URI PATTERN -> local path
  local uri="$1" pattern="$2" name
  command -v aws >/dev/null || die "the aws CLI is required for S3 sources"
  name="$(newest "$uri" "$pattern")"
  [[ -n "$name" ]] || die "nothing matching $pattern in $uri"
  aws_s3 cp --only-show-errors "${uri%/}/$name" "$work/$name"
  aws_s3 cp --only-show-errors "${uri%/}/$name.sha256" "$work/$name.sha256" || die "no checksum next to $name in $uri"
  printf '%s' "$work/$name"
}
step "fetching backups"
if [[ -n "${DRILL_DB_BACKUP:-}" ]]; then db_file="$DRILL_DB_BACKUP"
else [[ -n "${DRILL_S3_DB_URI:-}" ]] || die "set DRILL_S3_DB_URI (or DRILL_DB_BACKUP)"; db_file="$(fetch "$DRILL_S3_DB_URI" '^vectorscrub-.*\.dump(\.gpg)?$')"; fi
if [[ -n "${DRILL_KEYSTORE_BACKUP:-}" ]]; then ks_file="$DRILL_KEYSTORE_BACKUP"
else [[ -n "${DRILL_S3_KEYSTORE_URI:-}" ]] || die "set DRILL_S3_KEYSTORE_URI (or DRILL_KEYSTORE_BACKUP)"; ks_file="$(fetch "$DRILL_S3_KEYSTORE_URI" '^pyhsm-keystore-.*\.enc(\.gpg)?$')"; fi
say "database backup: $(basename "$db_file")"
say "keystore backup: $(basename "$ks_file")"

# ---- 2. integrity and decryption -------------------------------------------------------------------
verify_sum() {
  [[ -f "$1.sha256" ]] || die "no checksum file next to $(basename "$1")"
  [[ "$(sha256_of "$1")" == "$(awk '{print $1}' "$1.sha256")" ]] || die "CHECKSUM MISMATCH: $(basename "$1") is not what was backed up"
}
verify_sum "$db_file"; verify_sum "$ks_file"
step "checksums match"
plain() { # decrypt if needed -> path
  if [[ "$1" == *.gpg ]]; then
    command -v gpg >/dev/null || die "gpg is required"
    gpg --batch --yes --quiet --output "$2" --decrypt "$1" || die "could not decrypt $(basename "$1") (is its private key in GNUPGHOME?)"
    printf '%s' "$2"
  else printf '%s' "$1"; fi
}
dump="$(plain "$db_file" "$work/db.dump")"
keystore="$(plain "$ks_file" "$work/keystore.enc")"
step "decrypted both"

# ---- 3. a key service on the restored keystore -------------------------------------------------------
ipc_secret="$(openssl rand -hex 32)"
docker volume create "$datav" >/dev/null; docker volume create "$sockv" >/dev/null
# Load the keystore and the password into the volume, owned by PyHSM's uid. The password is copied
# into the data volume only for the drill's lifetime (the volume is deleted on exit).
docker run --rm -i -v "$datav:/d" -v "$sockv:/s" alpine:3 sh -c '
  cat > /d/keystore.enc && chown 1000:1000 /d /s /d/keystore.enc && chmod 750 /d && chmod 700 /s && chmod 600 /d/keystore.enc' < "$keystore"
docker run --rm -i -v "$datav:/d" alpine:3 sh -c 'cat > /d/password && chown 1000:1000 /d/password && chmod 600 /d/password' < "$DRILL_PYHSM_PASSWORD_FILE"
docker run -d --name "$hsmc" --network none --user 1000:1000 --read-only --cap-drop ALL \
  --security-opt no-new-privileges:true --tmpfs /tmp:noexec,nosuid,nodev,size=64m \
  -v "$datav:/data" -v "$sockv:/run/pyhsm" -e PYHSM_IPC_SECRET="$ipc_secret" \
  "$PYHSM_IMAGE" vectorguard-pyhsm-server --store /data/keystore.enc --password-file /data/password \
  --socket /run/pyhsm/pyhsm.sock --rate-limit 100000 >/dev/null
for _ in $(seq 1 30); do
  docker run --rm -v "$sockv:/s" alpine:3 test -S /s/pyhsm.sock 2>/dev/null && break
  [[ "$(docker inspect -f '{{.State.Running}}' "$hsmc")" == true ]] || { docker logs "$hsmc" 2>&1 | tail -5 >&2; die "PyHSM did not start on the restored keystore (wrong password, or a damaged keystore)"; }
  sleep 1
done
step "key service running on the restored keystore"

# ---- 4. a scratch database -------------------------------------------------------------------------
docker network create --internal "$net" >/dev/null
pgpw="$(openssl rand -hex 16)"
docker run -d --name "$pgc" --network "$net" -e POSTGRES_USER=vectorscrub -e POSTGRES_DB=vectorscrub \
  -e POSTGRES_PASSWORD="$pgpw" "$PG_IMAGE" >/dev/null
for _ in $(seq 1 60); do docker exec "$pgc" pg_isready -U vectorscrub -d vectorscrub >/dev/null 2>&1 && break; sleep 1; done
docker exec "$pgc" pg_isready -U vectorscrub -d vectorscrub >/dev/null || die "the scratch database did not start"
sleep 2
restore_start="$(now)"
docker exec -i "$pgc" pg_restore --exit-on-error --no-owner --no-privileges -U vectorscrub -d vectorscrub < "$dump" \
  || die "pg_restore failed"
restore_secs=$(( $(now) - restore_start ))
customers="$(docker exec "$pgc" psql -U vectorscrub -d vectorscrub -Atc 'select count(*) from customers')"
step "database restored in ${restore_secs}s ($customers customer accounts)"

# ---- 5. does it open? ------------------------------------------------------------------------------
verify_env=(-e DATABASE_URL="postgres://vectorscrub:$pgpw@$pgc:5432/vectorscrub" -e KEY_PROVIDER=pyhsm
            -e PYHSM_SOCKET=/run/pyhsm/pyhsm.sock -e PYHSM_IPC_SECRET="$ipc_secret" -e PYHSM_KEY_ID="$KEY_ID")
vb() { docker run --rm --network "$net" --user 1000:1000 -v "$sockv:/run/pyhsm" "${verify_env[@]}" "$BACKEND_IMAGE" "$@"; }
result=usable
if vb verify-backup 2>&1 | grep -v '"level":"INFO"' | sed 's/^/restore-drill:   /'; then :; else result="NOT usable"; fi
# The exit status of verify-backup is what counts; re-run quietly to read it reliably.
vb verify-backup >/dev/null 2>&1 || result="NOT usable"
chain="$(vb audit verify 2>&1 | grep -v '"level":' | tail -1)"
say "audit chain: $chain"
[[ "$chain" == "the audit chain holds" ]] || result="NOT usable"
total=$(( $(now) - started ))
step "done"

# ---- 6. report -------------------------------------------------------------------------------------
line="| $(date -u +%Y-%m-%d) | ${DRILL_ENVIRONMENT:-unspecified} | $(basename "$db_file") + $(basename "$ks_file") | ${restore_secs}s database restore, ${total}s total | $result | $customers accounts; audit chain: $chain |"
if [[ ! -f "$DRILL_LOG" ]]; then
  printf '# Restore drill log\n\nAppended by `infra/scripts/restore_drill.sh`. One line per drill.\n\n| Date | Environment | Backups | Time | Result | Notes |\n|---|---|---|---|---|---|\n' > "$DRILL_LOG"
fi
echo "$line" >> "$DRILL_LOG"
say "logged to $DRILL_LOG"
if [[ "$result" == usable ]]; then
  say "RESULT: usable. The database and the keystore backups restore together, and the data opens. Measured: ${total}s."
else
  die "RESULT: NOT USABLE. See the problems above. This is an incident: you do not currently have a working backup."
fi
