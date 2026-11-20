# Reverse proxy (nginx)

`vectorscrub.conf` and `vectorscrub-proxy.inc` are the reference proxy for a single-host
deployment: TLS in front of the Next.js container, with the doors that must stay closed shut.

## What was tested

`nginx -t` passes on `nginx:1.27-alpine`, and the behaviour below was checked against a running
container with a stand-in upstream:

| Request | Result |
|---|---|
| `/api/v1/internal/...`, `/api/v1/metrics`, `/docs`, `/api-docs/...`, `/swagger` | 404, including capital letters (`/API/V1/INTERNAL`), `%69nternal`, `//`, `/./`, `/x/../` and `%2F` variants |
| `/`, `/api/v1/me`, `/api/v1/health/live`, `/api/v1/internalfoo`, `/documents` | passed through (the block does not catch look-alike paths) |
| A client sending `X-Forwarded-For: 6.6.6.6, 7.7.7.7` and `X-Real-IP: 8.8.8.8` | the upstream saw only the real peer address |
| Body over 1 MB | 413 |
| 60 rapid `POST /api/v1/auth/login` | 11 passed, 49 got 429 (burst 10, then 30 per minute) |
| Plain HTTP | 301 to HTTPS |

It has not been run against your real hostname, certificate or load balancer. Do that before
launch (`curl -I https://app.example.com/api/v1/internal/x` must say 404).

## Using it

1. Replace `app.example.com` and the certificate paths in `vectorscrub.conf`.
2. Install: `vectorscrub.conf` into `/etc/nginx/conf.d/`, `vectorscrub-proxy.inc` into
   `/etc/nginx/vectorscrub-proxy.inc`. Then `nginx -t && nginx -s reload`.
3. In the backend's environment set `TRUST_PROXY_HEADERS=true` and leave `TRUSTED_PROXY_HOPS=1`.

## Why `TRUSTED_PROXY_HOPS=1` is right for this layout

The path is browser, nginx, the Next.js server, the backend. nginx **overwrites**
`X-Forwarded-For` with the real peer address. The Next.js server forwards that header unchanged
(checked: it neither appends to it nor adds one), so the backend sees exactly one entry, the
client, and one hop from the right is correct. If you put a load balancer in front of nginx, that
adds an entry: set `TRUSTED_PROXY_HOPS=2` and make nginx append (`$proxy_add_x_forwarded_for`)
**only** for that trusted balancer, never for the open internet.

If nginx is missing and the frontend is reachable directly, no `X-Forwarded-For` arrives and the
backend falls back to the peer address, so every client looks like the frontend container and
shares one rate-limit bucket. The compose file binds the frontend to loopback so that cannot
happen by accident.

## What this does not do

It does not terminate TLS for the database or Redis, and it does not protect the agent-only
API: that is on `INTERNAL_PORT` (8081 in compose) on a network the proxy is not part of.
