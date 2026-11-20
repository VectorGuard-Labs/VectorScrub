# Consumer-facing compliance checklist

Scope: six things a SaaS that collects personal data is commonly expected to get right. For each:
what the code does today, what has to be done outside the code, and what to re-check when
something changes. **This is an engineering checklist, not legal advice.** The legal pages are an
unreviewed pilot draft; have a lawyer review them (and the items marked "decision") before real
customers.

Last checked against the code: 2026-11-18.

## Decisions recorded (owner, 2026-11-18)

- **Age (18):** counsel will confirm it, together with the review of the legal pages and the refund wording.
- **Existing accounts:** nothing to do. The service is a prototype and no real customer has registered, so no account lacks the age confirmation. **Revisit this before the first real customer is let in**: any account created before that point should be deleted rather than carried forward.
- **Postal address:** `VectorGuard Labs, 915 Wilshire Blvd, 7th Floor, Los Angeles, CA 90017`, set as `POSTAL_ADDRESS` in the local `.env` and `frontend/.env.local`, and shown in the page footers. **Set the same value in the production environment** (it is passed through `infra/docker/docker-compose.yml`; the `.env` files are not committed). Confirm you can actually receive mail there: a suite address that is a mail drop counts only if it is a CMRA/PO-box-style address you have access to.
- **DMCA agent:** the owner will register it; until the four `DMCA_AGENT_*` values are set, the page sends notices to the contact email.
- **Marketing email:** none will be sent for now. Nothing in the code can send one.

## 1. Age gate

- **Done in code.** Registration needs a tick on "I am 18 years old or older". The API refuses a
  registration without `confirmed_adult: true` (so a client that skips the form is stopped too) and
  stores the time in `customers.adult_confirmed_at` (migration 0023). Terms section 3 says 18+ and
  that an account found to belong to a minor is closed and erased. Privacy section 10 says the
  service is not for under 18s and how to ask for a child's data to be erased. `TERMS_VERSION` is
  `2026-11-pilot-3`.
- **Limits.** It is a declaration. No date of birth is asked at signup (data minimisation). Accounts
  created before this change have no `adult_confirmed_at`.
- **Age:** 18 is used because a paid subscription is a contract and the service handles sensitive data; COPPA's line is 13 and US state laws put teen protections at 16, so 18 clears all of them. Counsel to confirm (see Decisions). Existing accounts: none are real (see Decisions).
- **Process:** if anyone reports a minor's account, run `vectorscrub erase-account <email>` and note the
  date.

## 2. Externally loaded fonts

- **Verified clean.** No `next/font/google`, Google Fonts, CDN fonts, or remote CSS anywhere in the
  source; the page uses the system font stack. The Content-Security-Policy allows fonts, scripts,
  styles, images and connections from `'self'` only, so the browser would refuse an external one even
  if one were added.
- **Kept clean by CI:** `npm run check:third-party` (in `ci.yml`) fails on external fonts, scripts,
  trackers, session replay, banned packages, or a CSP that opens to another origin. It was checked to
  fail on each.
- **If you want a custom font:** put the files in the repo and use `next/font/local`. A font loaded
  from a CDN sends every visitor's IP address to that company, which is a privacy-law problem in
  the EU (a German court fined a site for exactly this).

## 3. Session replay without consent

- **Verified none.** No Hotjar, FullStory, LogRocket, Clarity, PostHog, Mouseflow, Smartlook or similar,
  no analytics, no error tracker (the ErrorBoundary has only a commented example), and the CSP would
  block them. The Privacy Policy section 8 now says so. Same CI check as above.
- **If you ever want analytics or replay:** it needs a consent banner that blocks the script until the
  visitor accepts (opt-in, not a pre-ticked box), a way to withdraw, and masking of every field on
  the dashboard (identity forms, addresses, listing links). Do not record the dashboard at all:
  it shows personal data from broker sites. Prefer privacy-preserving, cookieless, first-party
  analytics hosted by you. Update the Privacy Policy and bump `TERMS_VERSION`.

## 4. Marketing email: unsubscribe and postal address

- **No email is sent to customers today.** The only email the system sends is opt-out and deletion
  requests to brokers and companies, on the customer's behalf (not marketing, not to customers).
  There is an `email_enabled` notification setting in the database but no code that sends an email
  to a customer. There is therefore nothing to fix, and no way to send a marketing email by accident.
- **Prepared:** `POSTAL_ADDRESS` (web server setting) is shown in the footers once you set it.
- **Before you send any email to customers** (alerts, receipts, newsletters, launch notes):
  - *Transactional* (password reset, security notice, a removal result the customer asked for):
    identify the sender, no promotional content, no unsubscribe needed.
  - *Marketing or promotional* (newsletter, upsell, pilot announcements, "tips"): opt-in at signup
    through a separate unticked box (the pilot's two optional boxes are not email consent); every
    message needs a working one-click unsubscribe (`List-Unsubscribe` and `List-Unsubscribe-Post`
    headers, plus a visible link), honored within 10 business days (do it immediately), the
    company's real postal address, honest subject lines and a clear identification as an ad where
    the law requires it (CAN-SPAM). Keep a suppression list that survives account deletion (store
    only a hash of the address). EU/UK/Canada need prior consent.
  - Send marketing from a different address and sending domain than security mail, with SPF, DKIM,
    DMARC.
- **Done:** `POSTAL_ADDRESS` is set locally (see Decisions); set it in production too.

## 5. Renewal terms beside the Subscribe button

- **Done in code.** On the Billing page, the renewal terms (plan, price, interval, "renews
  automatically until you cancel", when the card is charged, how to cancel, what cancelling does,
  refund position, link to Terms section 6) sit directly above the Subscribe button, and the button
  stays inactive until the customer ticks "I understand that this plan renews automatically at
  $X per month until I cancel." The home page shows the same notice on the paid plan's card. The
  figures come from the same table as the price, so they cannot disagree. Terms section 6 has an
  anchor (`/terms#plans`) the notice links to.
- **Not in the code, and where the purchase actually happens:** Subscribe sends the customer to a
  **Stripe Payment Link**. What Stripe's page shows is configured in Stripe, not here.
- **You (Stripe dashboard, for each Payment Link):**
  - Turn on "Require customers to accept your terms of service" and set the Terms URL.
  - Add "custom text" next to the Pay button with the same renewal sentence, in plain words.
  - Confirm the price and interval on the Stripe page match the page here.
  - Turn on Stripe's customer emails: successful-payment receipts, and upcoming-renewal reminders
    (needed for annual plans in several states, e.g. California requires a reminder 3 to 21 days
    before an annual renewal, and an acknowledgment email with the terms and how to cancel).
  - Keep "Manage subscription" (Customer Portal) enabled with cancellation allowed: cancelling must
    be as easy as signing up (the FTC and California both check this).
- **Decision for counsel:** the refund wording ("no refund for the current period unless the law
  requires") against California and other state rules; the free-trial or intro-price rules if you ever
  add one.

## 6. DMCA designated agent

- **What applies.** The DMCA safe harbor (17 U.S.C. 512(c)) protects a service that stores material
  *users post*. VectorScrub keeps customers' details private to their own account and publishes
  nothing, so it probably hosts no user content in that sense, and a registered agent may not be
  required. Registering is cheap ($6, valid for 3 years) and removes the question, so it is still
  worth doing if there is any chance of ever showing user material to others.
- **Done in code.** Terms section 9 ("Copyright") says what the service does and does not host and
  gives a route for notices: today the contact email. The page shows a designated agent **only when
  all of `DMCA_AGENT_NAME`, `DMCA_AGENT_ADDRESS`, `DMCA_AGENT_EMAIL` and `DMCA_AGENT_REGISTRATION`
  (the number `DMCA-1-...` the Copyright Office issues) are set and well formed.** It will not show
  an agent you have not registered, because the safe harbor depends on the registration being real
  and current.
- **You:** register at <https://dmca.copyright.gov/> (needs the service's legal name, all names it
  operates under, its website addresses, and an agent with a street address, phone and email; use a
  role mailbox such as `copyright@...`, not a personal one). Then set the four variables and redeploy.
  Renew every 3 years (the registration lapses without a renewal; put it in a calendar). Have a
  written process for handling a notice (who looks, 24-hour target, counter-notice handling).

## Setting reference (web server environment)

| Variable | Used for |
|---|---|
| `CONTACT_EMAIL` | Support, data requests, copyright notices when no agent is registered (already existed) |
| `POSTAL_ADDRESS` | Footer on every page and the legal pages; required in any commercial email |
| `DMCA_AGENT_NAME`, `DMCA_AGENT_ADDRESS`, `DMCA_AGENT_EMAIL`, `DMCA_AGENT_REGISTRATION` | Designated agent in Terms section 9. All four or none |
