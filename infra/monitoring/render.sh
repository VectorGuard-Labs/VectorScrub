#!/usr/bin/env bash
# Render alertmanager.yml for a NAMED person, and check it. Refuses to render without one: an alert
# that goes to nobody is the failure this exists to prevent.
#
#   ONCALL_NAME="Ada Lovelace" ONCALL_EMAIL=ada@yourdomain \
#   ALERT_SMTP_SMARTHOST=smtp.yourprovider:587 ALERT_SMTP_FROM=alerts@yourdomain \
#   ALERT_SMTP_USERNAME=... ALERT_SMTP_PASSWORD_FILE=/path/to/file \
#   ALERT_DEADMAN_URL=https://hc-ping.com/<uuid> \
#   [ONCALL_PAGERDUTY_ROUTING_KEY_FILE=/path/to/file] \
#   infra/monitoring/render.sh
#
# Writes infra/monitoring/alertmanager.yml (no secrets in it) and infra/monitoring/secrets/ (mode 700,
# the SMTP password and the PagerDuty key copied in, mode 600). Both are git-ignored. Then it runs
# `amtool check-config` and prints which receiver each kind of alert reaches.
#
# ALERT_SMTP_REQUIRE_TLS defaults to true; set false ONLY for a local test mail sink.
set -euo pipefail
umask 077

die() { echo "render: $*" >&2; exit 1; }
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
AM_IMAGE="${AM_IMAGE:-prom/alertmanager@sha256:e638797febd034838eca8b98cdbd46f951e9bf7f178777aa8669167406a9946c}"

[[ -n "${ONCALL_NAME:-}" ]] || die "set ONCALL_NAME: alerts must go to a named person"
[[ "${ONCALL_EMAIL:-}" =~ ^[^@[:space:]\"]+@[^@[:space:]\"]+\.[^@[:space:]\"]+$ ]] || die "set ONCALL_EMAIL to that person's address"
[[ "$ONCALL_NAME" != *[\"\'@\\\{\}]* ]] || die "ONCALL_NAME may not contain quotes, @, backslashes or braces"
[[ -n "${ALERT_SMTP_SMARTHOST:-}" && "$ALERT_SMTP_SMARTHOST" =~ ^[A-Za-z0-9.-]+:[0-9]+$ ]] || die "set ALERT_SMTP_SMARTHOST to host:port"
[[ "${ALERT_SMTP_FROM:-}" =~ ^[^@[:space:]\"]+@[^@[:space:]\"]+$ ]] || die "set ALERT_SMTP_FROM to the sending address"
[[ -z "${ALERT_SMTP_USERNAME:-}" || "$ALERT_SMTP_USERNAME" != *\"* ]] || die "ALERT_SMTP_USERNAME may not contain quotes"
[[ -n "${ALERT_DEADMAN_URL:-}" && "$ALERT_DEADMAN_URL" =~ ^https?://[^[:space:]\"]+$ ]] || die "set ALERT_DEADMAN_URL: without it nobody learns that monitoring itself stopped"
tls="${ALERT_SMTP_REQUIRE_TLS:-true}"; [[ "$tls" == true || "$tls" == false ]] || die "ALERT_SMTP_REQUIRE_TLS must be true or false"
if [[ "$tls" == false ]]; then echo "render: WARNING: SMTP without TLS. Only for a local test mail sink." >&2; fi
if [[ "$ALERT_DEADMAN_URL" == http://* && "$tls" == true ]]; then die "ALERT_DEADMAN_URL must be https (http is accepted only together with ALERT_SMTP_REQUIRE_TLS=false, for local tests)"; fi

mkdir -p "$here/secrets"; chmod 700 "$here/secrets"
if [[ -n "${ALERT_SMTP_PASSWORD_FILE:-}" ]]; then
  [[ -f "$ALERT_SMTP_PASSWORD_FILE" ]] || die "no such file: $ALERT_SMTP_PASSWORD_FILE"
  install -m 600 "$ALERT_SMTP_PASSWORD_FILE" "$here/secrets/smtp_password"
else
  : > "$here/secrets/smtp_password"; chmod 600 "$here/secrets/smtp_password"
fi

pd_block=""
if [[ -n "${ONCALL_PAGERDUTY_ROUTING_KEY_FILE:-}" ]]; then
  [[ -f "$ONCALL_PAGERDUTY_ROUTING_KEY_FILE" ]] || die "no such file: $ONCALL_PAGERDUTY_ROUTING_KEY_FILE"
  install -m 600 "$ONCALL_PAGERDUTY_ROUTING_KEY_FILE" "$here/secrets/pagerduty_routing_key"
  pd_block="    pagerduty_configs:
      - routing_key_file: /etc/alertmanager/secrets/pagerduty_routing_key
        send_resolved: true"
else
  rm -f "$here/secrets/pagerduty_routing_key"
  echo "render: no PagerDuty key: pages go by email only. Email is a poor pager at 3 a.m.; consider a paging service." >&2
fi
deadman="    webhook_configs:
      - url: \"$ALERT_DEADMAN_URL\"
        send_resolved: false"

R_SMARTHOST="$ALERT_SMTP_SMARTHOST" R_FROM="$ALERT_SMTP_FROM" R_USER="${ALERT_SMTP_USERNAME:-}" R_TLS="$tls" \
R_EMAIL="$ONCALL_EMAIL" R_NAME="$ONCALL_NAME" R_PD="$pd_block" R_DEADMAN="$deadman" \
python3 - "$here/alertmanager.yml.tmpl" "$here/alertmanager.yml" <<'PY'
import os, sys
t = open(sys.argv[1]).read()
for marker, var in {
    "@SMTP_SMARTHOST@": "R_SMARTHOST", "@SMTP_FROM@": "R_FROM", "@SMTP_USERNAME@": "R_USER",
    "@SMTP_REQUIRE_TLS@": "R_TLS", "@ONCALL_EMAIL@": "R_EMAIL", "@ONCALL_NAME@": "R_NAME",
    "@PAGERDUTY_BLOCK@": "R_PD", "@DEADMAN_BLOCK@": "R_DEADMAN",
}.items():
    t = t.replace(marker, os.environ[var])
leftover = [m for m in ("@SMTP_", "@ONCALL_", "@PAGERDUTY_BLOCK@", "@DEADMAN_BLOCK@") if m in t]
if leftover:
    sys.exit(f"render: unreplaced markers: {leftover}")
open(sys.argv[2], "w").write(t)
PY
chmod 644 "$here/alertmanager.yml"
# Alertmanager runs as uid 65534 (nobody) and reads these files through a bind mount. On Linux the
# files keep their host owner, so they must belong to that uid. (Docker Desktop maps ownership itself.)
if [[ "$(uname -s)" == Linux ]]; then
  if [[ "$(id -u)" == 0 ]]; then
    chown -R 65534:65534 "$here/secrets"
  else
    echo "render: run as root, or: sudo chown -R 65534:65534 $here/secrets  (Alertmanager reads them as uid 65534)" >&2
  fi
fi

docker run --rm --entrypoint amtool -v "$here:/etc/alertmanager:ro" "$AM_IMAGE" check-config /etc/alertmanager/alertmanager.yml >/dev/null \
  || die "amtool rejected the rendered config"
echo "render: alertmanager.yml is valid. Alerts reach:"
for labels in "alertname=BackendDown severity=page" "alertname=SlowRequests severity=ticket" "alertname=Watchdog severity=none"; do
  printf '  %-40s -> %s\n' "$labels" "$(docker run --rm --entrypoint amtool -v "$here:/etc/alertmanager:ro" "$AM_IMAGE" config routes test --config.file=/etc/alertmanager/alertmanager.yml $labels)"
done
echo "render: on-call: $ONCALL_NAME <$ONCALL_EMAIL>"
