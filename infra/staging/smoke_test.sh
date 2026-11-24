#!/usr/bin/env bash
# The checks from docs/security/PENTEST_BRIEF.md ("Before you commission it") that a script can make,
# run against a staging stack. It uses only fake accounts and prints PASS or FAIL per check; the exit
# status is non-zero if any check failed.
#
# Not covered here (do them by hand, and the brief says how): OWASP ZAP baseline, testssl.sh, replaying
# a TOTP or recovery code, idle-timeout behaviour, and reading a day of logs for sensitive values.
# This test also makes no claim about a real reverse proxy: point STAGING_URL at the real staging
# hostname to include it.
#
# Settings: STAGING_URL, STAGING_ENV_FILE, STAGING_PROJECT, COMPOSE_DIR (set by staging.sh).
set -uo pipefail

URL="${STAGING_URL:-http://127.0.0.1:3000}"
ENV_FILE="${STAGING_ENV_FILE:?run this through staging.sh smoke}"
PROJECT="${STAGING_PROJECT:-vsstaging}"
COMPOSE_DIR="${COMPOSE_DIR:?}"
API="$URL/api/v1"
set -a; . "$ENV_FILE"; set +a

pass=0; fail=0
ok()   { printf 'PASS  %s\n' "$1"; pass=$((pass+1)); }
bad()  { printf 'FAIL  %s%s\n' "$1" "${2:+  ($2)}"; fail=$((fail+1)); }
check() { # check "name" expected actual
  if [[ "$2" == "$3" ]]; then ok "$1"; else bad "$1" "wanted $2, got $3"; fi
}
dc() { (cd "$COMPOSE_DIR" && docker compose -p "$PROJECT" --env-file "$ENV_FILE" "$@"); }

# --- a tiny HTTP client: one session per variable, cookies handled by hand so the checks work over
# http too (the session cookie is Secure, and curl will not send a Secure cookie over http).
H="$(mktemp)"; trap 'rm -f "$H"' EXIT
code() { curl -s -m 30 -o /dev/null -w '%{http_code}' "$@"; }
# req METHOD PATH SESSION_COOKIE [json]  -> prints "status<TAB>body"; sets $LAST_HEADERS
csrf=""
get_csrf() { curl -s -m 30 -D "$H" "$API/csrf-token" | sed -n 's/.*"csrf_token":"\([^"]*\)".*/\1/p'; }
req() {
  local method="$1" path="$2" sess="$3" body="${4:-}" tok; tok="$(get_csrf)"
  local args=(-s -m 30 -D "$H" -o /tmp/vs_body -w '%{http_code}' -X "$method" -H "x-csrf-token: $tok" -H "Cookie: csrf_token=$tok${sess:+; vs_session=$sess}")
  [[ -n "$body" ]] && args+=(-H 'content-type: application/json' -d "$body")
  local s; s="$(curl "${args[@]}" "$API$path")"
  printf '%s\t%s' "$s" "$(cat /tmp/vs_body)"
}
status_of() { printf '%s' "${1%%$'\t'*}"; }
body_of() { printf '%s' "${1#*$'\t'}"; }
cookie_of() { sed -n 's/^[Ss]et-[Cc]ookie: vs_session=\([^;]*\);.*/\1/p' "$H" | tail -1; }
# Auth endpoints are limited to 5 a minute per address. Wait out a 429 instead of failing: the limit is a
# production setting and is left as deployed.
authreq() {
  local out i
  for i in 1 2 3 4 5 6; do
    out="$(req "$@")"
    [[ "$(status_of "$out")" == 429 ]] || { printf '%s' "$out"; return; }
    sleep 13
  done
  printf '%s' "$out"
}

PW="staging smoke passphrase $RANDOM-$RANDOM"
terms="$(grep -o 'TERMS_VERSION[^"]*"[^"]*"' "$COMPOSE_DIR/../../frontend/src/lib/legal.ts" 2>/dev/null | head -1 | sed 's/.*"\(.*\)"/\1/')"
[[ -n "$terms" ]] || terms="$(sed -n 's/.*TERMS_VERSION: &str = "\(.*\)";/\1/p' "$COMPOSE_DIR/../../backend/src/models/customer.rs")"
register() { authreq POST /auth/register "" "{\"email\":\"$1\",\"password\":\"$PW\",\"accepted_terms_version\":\"$terms\",\"confirmed_adult\":true}"; }

echo "== staging smoke test against $URL"

# ---- the stack is up and healthy
check "readiness probe" 200 "$(code "$API/health/ready")"
ks="$(dc exec -T backend ./vectorscrub kms status 2>&1)"
if grep -q 'key service answering: true' <<<"$ks" && grep -q "exists: true" <<<"$ks"; then ok "key service answers and holds the key"; else bad "key service" "$(head -c 200 <<<"$ks")"; fi

# ---- the closed doors
check "agent API is not on the public port" 404 "$(code -X POST -H 'X-Agent-Key: x' "$API/internal/scan/status")"
# With METRICS_TOKEN configured the backend answers 401 without it (404 only when no token is set).
# The production nginx config answers 404 for this path before it reaches the app.
check "metrics refused without the token" 401 "$(code "$API/metrics")"
check "metrics refused with a wrong token" 401 "$(code -H 'Authorization: Bearer wrong-token-wrong-token-wrong-token' "$API/metrics")"
check "metrics served with the token" 200 "$(code -H "Authorization: Bearer $METRICS_TOKEN" "$API/metrics")"
check "API description is not served in production" 404 "$(code "$URL/api-docs/openapi.json")"
check "docs are not served in production" 404 "$(code "$URL/docs")"
hdr="$(curl -s -m 30 -I "$API/health/live")"
grep -qi '^strict-transport-security' <<<"$hdr" && ok "HSTS on API responses" || bad "HSTS on API responses"
grep -qi '^x-content-type-options: nosniff' <<<"$hdr" && ok "nosniff on API responses" || bad "nosniff on API responses"
csp="$(curl -s -m 30 -I "$URL/login" | tr -d '\r')"
grep -qi "^content-security-policy:.*script-src 'self' 'nonce-" <<<"$csp" && ok "page CSP carries a per-request nonce" || bad "page CSP"
grep -qi "unsafe-inline.*script-src\|script-src[^;]*unsafe-inline" <<<"$csp" && bad "CSP allows inline scripts" || ok "CSP has no unsafe-inline for scripts"

# ---- accounts, sessions, passwords
A="smoke-a-$RANDOM@example.com"; B="smoke-b-$RANDOM@example.com"
weak="$(authreq POST /auth/register "" "{\"email\":\"$A\",\"password\":\"Password123!!!\",\"accepted_terms_version\":\"$terms\",\"confirmed_adult\":true}")"
check "a common password is refused at registration" 422 "$(status_of "$weak")"
r="$(register "$A")"; check "account A registers" 201 "$(status_of "$r")"; SA="$(cookie_of)"
r="$(register "$B")"; check "account B registers" 201 "$(status_of "$r")"; SB="$(cookie_of)"
[[ -n "$SA" && -n "$SB" ]] || { bad "session cookies were issued"; echo "cannot continue"; exit 1; }
check "A reads their own profile" 200 "$(status_of "$(req GET /me "$SA")")"
check "no session, no profile" 401 "$(status_of "$(req GET /me "")")"
check "a forged session cookie is refused" 401 "$(status_of "$(req GET /me "eyJhbGciOiJIUzI1NiJ9.e30.x")")"
r="$(req POST /me/identity "$SA")"; [[ "$(status_of "$r")" =~ ^(400|415|422)$ ]] && ok "an empty identity body is rejected" || bad "empty identity body" "$(status_of "$r")"

# ---- cross-customer: B using A's identifiers
asess="$(req GET /me/sessions "$SA")"; aid="$(body_of "$asess" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p' | head -1)"
check "B cannot end A's session" 404 "$(status_of "$(req DELETE "/me/sessions/$aid" "$SB")")"
check "A's session survives that" 200 "$(status_of "$(req GET /me "$SA")")"
check "B cannot read a scan id of A's" 404 "$(status_of "$(req GET "/scans/$(cat /proc/sys/kernel/random/uuid 2>/dev/null || uuidgen)" "$SB")")"
check "B sees none of A's exposures" '{"exposures":[]}' "$(body_of "$(req GET /exposures "$SB")" | tr -d ' ' | sed 's/,"total":[0-9]*//')"

# ---- CSRF
tok="$(get_csrf)"
check "a state-changing request without the CSRF token is refused" 403 "$(code -X POST -H "Cookie: vs_session=$SA" -H 'content-type: application/json' -d '{}' "$API/me/password")"
check "a mismatched CSRF token is refused" 403 "$(code -X POST -H "Cookie: vs_session=$SA; csrf_token=$tok" -H "x-csrf-token: other" -H 'content-type: application/json' -d '{}' "$API/me/password")"

# ---- sign out everywhere
r="$(authreq POST /auth/login "" "{\"email\":\"$A\",\"password\":\"$PW\"}")"; check "A signs in a second time" 200 "$(status_of "$r")"; SA2="$(cookie_of)"
check "the second session works" 200 "$(status_of "$(req GET /me "$SA2")")"
check "sign out everywhere" 204 "$(status_of "$(req POST /auth/logout-all "$SA2")")"
check "the first session is refused afterwards" 401 "$(status_of "$(req GET /me "$SA")")"
check "and so is the second" 401 "$(status_of "$(req GET /me "$SA2")")"

# ---- lockout: five wrong passwords, then the right one is refused too
C="smoke-c-$RANDOM@example.com"; register "$C" >/dev/null
for _ in 1 2 3 4 5; do authreq POST /auth/login "" "{\"email\":\"$C\",\"password\":\"wrong wrong wrong wrong\"}" >/dev/null; done
sleep 13
last="$(req POST /auth/login "" "{\"email\":\"$C\",\"password\":\"$PW\"}")"
check "after five failures the account is locked, even for the right password" 429 "$(status_of "$last")"

# ---- the isolation the compose file promises
ag="$(dc exec -T agents python -c "
import socket
for host,port in (('postgres',5432),('pyhsm',1)):
    try:
        socket.create_connection((host,port),timeout=3); print(host,'REACHABLE')
    except OSError: print(host,'unreachable')
" 2>&1)"
grep -q 'postgres unreachable' <<<"$ag" && ok "agents cannot reach Postgres" || bad "agents reach Postgres" "$ag"
env_in_agents="$(dc exec -T agents sh -c 'env' 2>/dev/null | grep -E '^(MASTER_ENCRYPTION_KEY|PYHSM_IPC_SECRET|JWT_SECRET|DATABASE_URL|POSTGRES_PASSWORD)=' | wc -l | tr -d ' ')"
check "agents hold none of the backend-only secrets" 0 "$env_in_agents"
envb="$(dc exec -T backend sh -c 'env' 2>/dev/null | grep -c '^MASTER_ENCRYPTION_KEY=' | tr -d ' ')"
check "the backend holds no master key" 0 "$envb"
agk="$(dc exec -T agents python -c "
import os,urllib.request,urllib.error
u=os.environ['BACKEND_API_URL']+'/api/v1/internal/removal-requests'
try: urllib.request.urlopen(urllib.request.Request(u,headers={'X-Agent-Key':'wrong'}))
except urllib.error.HTTPError as e: print(e.code)
" 2>&1 | tail -1)"
check "the internal listener refuses a wrong agent key" 401 "$agk"

# ---- the database role and the audit trail
rt="$(dc exec -T -e PGPASSWORD="$DB_APP_PASSWORD" postgres psql -U vectorscrub_app -h 127.0.0.1 -d vectorscrub -Atc 'TRUNCATE audit_log' 2>&1 | head -1)"
grep -qi 'permission denied' <<<"$rt" && ok "the runtime role cannot truncate the audit log" || bad "runtime role truncate" "$rt"
rt="$(dc exec -T -e PGPASSWORD="$DB_APP_PASSWORD" postgres psql -U vectorscrub_app -h 127.0.0.1 -d vectorscrub -Atc 'DROP TRIGGER audit_log_no_update_delete ON audit_log' 2>&1 | head -1)"
grep -qi 'must be owner' <<<"$rt" && ok "the runtime role cannot drop the audit triggers" || bad "runtime role drop trigger" "$rt"
av="$(dc exec -T backend ./vectorscrub audit verify 2>&1 | tail -1)"
check "the audit chain holds" "the audit chain holds" "$av"

# ---- erasing an account
check "A erases their account (needs the password)" 204 "$(status_of "$(authreq POST /auth/login "" "{\"email\":\"$A\",\"password\":\"$PW\"}" >/dev/null; SA3="$(cookie_of)"; req DELETE /me "$SA3" "{\"password\":\"$PW\"}")")"
n="$(dc exec -T postgres psql -U vectorscrub -d vectorscrub -Atc "select count(*) from customers where lower(email)=lower('$A')")"
check "and the account is gone" 0 "$n"
left="$(dc exec -T postgres psql -U vectorscrub -d vectorscrub -Atc "select count(*) from audit_log where lower(user_email)=lower('$A')")"
check "and no audit row still carries the address" 0 "$left"
av="$(dc exec -T backend ./vectorscrub audit verify 2>&1 | tail -1)"
check "the audit chain still holds after the erasure" "the audit chain holds" "$av"

# ---- Redis down: the API keeps answering, and says so in a metric
dc stop redis >/dev/null 2>&1
sleep 2
check "with Redis stopped, the API still answers" 200 "$(status_of "$(req GET /me "$SB")")"
loginc="$(code -X POST -H 'content-type: application/json' -H "x-csrf-token: t" -H "Cookie: csrf_token=t" -d '{"email":"x@example.com","password":"nope nope nope nope"}' "$API/auth/login")"
[[ "$loginc" =~ ^(401|429)$ ]] && ok "with Redis stopped, login still answers and is still limited" || bad "login with Redis down" "$loginc"
m="$(curl -s -m 30 -H "Authorization: Bearer $METRICS_TOKEN" "$API/metrics" | grep -c '^rate_limit_backend_errors_total' | tr -d ' ')"
[[ "$m" -ge 1 ]] && ok "rate_limit_backend_errors_total appears" || bad "rate_limit_backend_errors_total missing"
dc start redis >/dev/null 2>&1; sleep 4
check "readiness returns once Redis is back" 200 "$(code "$API/health/ready")"
rs="$(dc exec -T redis sh -c 'redis-cli -a "$REDIS_PASSWORD" --no-auth-warning bgsave >/dev/null; sleep 2; redis-cli -a "$REDIS_PASSWORD" --no-auth-warning info persistence' 2>/dev/null | tr -d '\r' | sed -n 's/^rdb_last_bgsave_status://p')"
check "Redis can save to its volume" ok "$rs"
pw_in_cmdline="$(dc exec -T redis sh -c 'tr "\0" " " < /proc/1/cmdline' 2>/dev/null | grep -c "$REDIS_PASSWORD" | tr -d ' ')"
check "the Redis password is not on its command line" 0 "$pw_in_cmdline"

# ---- clean up what the test made
for e in "$B" "$C"; do
  dc exec -T backend ./vectorscrub erase-account "$e" >/dev/null 2>&1
done

echo
echo "== $pass passed, $fail failed"
[[ $fail -eq 0 ]]
