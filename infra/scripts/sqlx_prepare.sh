#!/usr/bin/env bash
# Regenerate, or verify, backend/.sqlx/ — the offline metadata for sqlx's compile-time checked
# queries (sqlx::query!). With it checked in, the backend builds with NO database available
# (SQLX_OFFLINE=true), which is what the Docker image and CI rely on.
#
#   infra/scripts/sqlx_prepare.sh           rewrite backend/.sqlx/ (commit the result)
#   infra/scripts/sqlx_prepare.sh --check   fail if backend/.sqlx/ is out of date (used in CI)
#
# Needs DATABASE_URL pointing at a PostgreSQL database with ALL migrations applied, e.g.:
#   createdb vectorscrub_prepare
#   for f in backend/migrations/*.sql; do psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -q -1 -f "$f"; done
set -euo pipefail

: "${DATABASE_URL:?DATABASE_URL must point at a database with every migration applied}"
MODE="${1:-write}"
cd "$(dirname "$0")/../../backend"

if [ "$MODE" = "--check" ]; then
  OUT="$(mktemp -d)"
  trap 'rm -rf "$OUT"' EXIT
else
  OUT="$PWD/.sqlx"
  rm -rf "$OUT" && mkdir "$OUT"
fi

# Touch main.rs so the query macros re-run even if nothing else changed, then let them write
# their metadata into $OUT while they check each query against the live database.
touch src/main.rs
SQLX_OFFLINE=false SQLX_OFFLINE_DIR="$OUT" cargo check --locked --all-targets

if [ "$MODE" = "--check" ]; then
  if diff -r .sqlx "$OUT" >/dev/null 2>&1; then
    echo "OK: backend/.sqlx is up to date"
  else
    echo "STALE: backend/.sqlx does not match the queries in src/. Run infra/scripts/sqlx_prepare.sh and commit backend/.sqlx" >&2
    diff -r .sqlx "$OUT" >&2 || true
    exit 1
  fi
else
  echo "Wrote $(ls "$OUT" | wc -l | tr -d ' ') query file(s) to backend/.sqlx — commit them."
fi
