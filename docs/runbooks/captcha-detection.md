# Runbook: a CAPTCHA appears on a broker's opt-out page

## What the system does
A CAPTCHA is a hard stop. The web-form executor (`agents/removal/executor.py`) never solves one, never
disguises the browser, and never pauses a session for a person to take over. It fails the request with
"CAPTCHA challenge on the opt-out page; this source needs to be completed manually", and the customer
is pointed to the manual opt-out guide. Nothing is written to disk for these requests: no screenshot,
no cookies, no session files.

## Symptoms
- Removal requests for one broker fail with the CAPTCHA message above.
- The opt-out guide shows "Opt-out page opened; form has a CAPTCHA" for that broker. That status comes
  from the nightly check in `agents/removal/health.py`, which reports to `/internal/optout-health`.

## Impact
Customers of that broker must opt out by hand. They are told so in the dashboard. Other brokers are unaffected.

## Diagnosis
1. Open the broker's opt-out page in an ordinary browser and confirm a CAPTCHA widget is present.
2. Check the nightly result (`GET /api/v1/public/optout-health`): `captcha` means the widget markers were
   found in the plain-HTTP page. `reachable` does not mean the browser can load it or that removal works.
3. Look at the executor logs for `web_form_captcha` (the host is logged, never the URL).

## Resolution
- Do not add CAPTCHA solving, proxy rotation, header spoofing or a pretend-human browser. The product does not do this.
- If the broker publishes an opt-out email address on its own page, a procedure of type `email` can be
  added in `OPT_OUT_REGISTRY` (`agents/removal/agent.py`). Then run
  `python -m removal.frontend_export` from `agents/` and commit the regenerated `frontend/src/lib/brokers.generated.ts`.
- Otherwise leave the source as manual-only. Marketing copy says opt-outs are automated "where the site allows it".

## Escalation
None needed for a single broker. If every web-form broker starts failing at once, see the executor logs for
`web_form_page_refused` (the site refused the browser) and check the agents container.
