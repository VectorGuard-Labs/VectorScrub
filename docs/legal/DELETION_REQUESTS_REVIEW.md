# Deletion requests: material for legal review

Status: **DRAFT, for review by counsel. The feature is switched off and no email has been sent to any company.**

> **Second update, 2026-11-24.** After counsel's second review: signed authorization with a downloadable PDF,
> withdrawal that also blocks the authorization, operator recovery of interrupted sends by Message-ID, delivery
> alerts, a DROP note, and the catalog re-checked. Authorization `2026-11-draft-3`, Terms `2026-11-pilot-6`.
> Section 10 is now the prioritized checklist.
>
> **Update, 2026-11-24.** A lawyer's recommendations were applied: authorization version `2026-11-draft-2`, a
> California-only pilot, separate choices for deletion and opt-out of sale or sharing, a withdrawal control, a
> retrievable authorization record, "no answer reported" wording after 45 days, and rewritten Terms and Privacy
> Policy (`TERMS_VERSION` `2026-11-pilot-5`). Sections 2 to 6 below describe the **first** draft and are kept for
> the record. The current text is in the code. Section 10 tracks the launch checklist.
Prepared from the code on 2026-11-24 (authorization version `2026-11-draft-1`). The text below is copied from
the code, not summarized. Nothing here is legal advice, and where this document says what a law requires,
that is a question for counsel, not a finding.

## 1. What the feature does

A paying customer picks companies from a short list. They tick a box that says they authorize VectorScrub to
act as their authorized agent. VectorScrub then emails each selected company a request to delete the
customer's personal information and, where the company's page offers it, to stop selling or sharing it.

- The customer is the one who approves, once per campaign, after reading the authorization text. The exact text
  and its SHA-256 hash are stored with the campaign.
- Each email is sent from VectorScrub's mail account. `Reply-To` is the customer's own address, so company
  answers go to the customer. VectorScrub does not read replies, and only records what the customer reports back.
- The email shares the customer's name and email address. It adds the street address only if the customer ticked
  a separate box (or the chosen company's page requires it). It never includes phone numbers or date of birth.
- The recipient address comes only from a catalog (section 5). Each entry carries a quote from the company's
  own published page that contains the address.

Switches: `DELETION_REQUESTS_ENABLED` (backend) and `DELETION_REQUESTS_SEND_ENABLED` (agents). Both default to off.

## 2. The authorization the customer accepts (verbatim)

Source: `backend/src/services/deletion.rs`, constant `AUTHORIZATION_TEXT`, version `AUTHORIZATION_VERSION`.
Shown on the page `frontend/src/app/dashboard/requests/page.tsx`, above the checkbox "I have read this and I
authorize VectorScrub to send these requests for me."

```
I authorize VectorScrub, a product of VectorGuard Labs, to act as my authorized agent in sending the privacy requests I select below to the companies I select. Each request asks the company to delete the personal information it holds about me and, where the company's page offers it, to stop selling or sharing it.

I agree that VectorScrub may share with each selected company my name and email address, and my address only if I tick the box to include it. It will not share my phone numbers or date of birth. Each company may contact me directly at my email address to verify my identity, and I understand a company may decline a request, or may ask me to confirm that I authorized this.

I understand that a request is not a guarantee of removal, that the law that gives me these rights depends on where I live, and that VectorScrub will tell me only what it sent and what I report back.

This authorization covers only the companies I select now. I can withdraw it for a company that has not yet been sent a request by contacting VectorScrub.
```

## 3. The email a company receives

Source: `agents/deletion/message.py`, function `render`. Below is the real output for a **fictional** person
(Jane Example, `jane@example.invalid`), for the Whitepages catalog entry, with the address included.
Plain text, one email per company.

```
Subject: Privacy request from Jane Example: deletion and opt-out of sale

To the privacy team at Whitepages, Inc,

I am writing as the authorized agent of Jane Example, through VectorScrub (a product of VectorGuard Labs). This request is made under the California Consumer Privacy Act (Cal. Civ. Code 1798.105 and 1798.120).

On their behalf, please:
  1. Delete the personal information you hold about them.
  2. Stop selling or sharing their personal information.

So that you can find their records:
  Name: Jane Example
  Email: jane@example.invalid
  Address: 123 Example Street, Sampletown, CA 90000

Jane Example authorized this request on November 24, 2026 at 19:00 UTC (authorization version 2026-11-draft-1). The authorization they accepted reads:

  > I authorize VectorScrub, a product of VectorGuard Labs, to act as my authorized agent in sending the privacy requests I select below to the companies I select. Each request asks the company to delete the personal information it holds about me and, where the company's page offers it, to stop selling or sharing it.
  >
  > I agree that VectorScrub may share with each selected company my name and email address, and my address only if I tick the box to include it. It will not share my phone numbers or date of birth. Each company may contact me directly at my email address to verify my identity, and I understand a company may decline a request, or may ask me to confirm that I authorized this.
  >
  > I understand that a request is not a guarantee of removal, that the law that gives me these rights depends on where I live, and that VectorScrub will tell me only what it sent and what I report back.
  >
  > This authorization covers only the companies I select now. I can withdraw it for a company that has not yet been sent a request by contacting VectorScrub.

You may contact Jane Example directly at jane@example.invalid to verify their identity or to confirm that they authorized this request. Please send your answer to that address.

Thank you,
VectorScrub, for Jane Example
```

Fixed by design: the email never contains phone numbers, date of birth or anything about other people. Every value
is flattened to one line, so a name cannot add a header or a line. If a company's page asks for a particular
subject line, that line is used as given.

## 4. Rules the system enforces

| Rule | Where |
|---|---|
| Paid plans only | `create_campaign` in `backend/src/services/deletion.rs` |
| The customer must tick the consent box and accept the **current** authorization version | same |
| A company is offered only to customers whose saved state is one the company's page offers it to (an entry with no state listed is offered to everyone) | `broker_channels::eligible` |
| 90-day cooldown per company, and no company twice in one campaign | `COOLDOWN_DAYS` |
| A request is claimed before sending and is never retried if interrupted (it is marked failed as "abandoned") | `agents/deletion/sender.py`, migration `0015_deletion_requests.sql` |
| If a company's page asks for the address, the customer cannot be sent to that company without agreeing to share it | `requires_address` |
| Recipient addresses come only from the catalog; it is checked at backend start and by `python -m deletion.verify_channels` | `broker_channels::validate` |
| Follow-up: after 45 days with no reply the dashboard says so (the period the CCPA allows a business to respond) | `FOLLOW_UP_DAYS` |
| The audit trail records each step; it cannot be edited | audit log |

No personal data is stored in the deletion tables. Name, email and address are decrypted from the customer's
encrypted profile at send time, and only the fields the customer agreed to share.

## 5. The company catalog (6 entries today)

Source: `backend/data/broker_channels.json`. Every entry also holds the company's own page URL, a short quote
from that page containing the address, and the date it was checked (all 2026-11-12). Only the summary is below.

| Company | Address | Offered to | Asks for | Legal basis the email cites | What the company's page says about agents |
|---|---|---|---|---|---|
| Whitepages, Inc | privacyrequest@whitepages.com | CA | delete, opt_out_sale | the California Consumer Privacy Act (Cal. Civ. Code 1798.105 and 1798.120) | Whitepages says an agent "may need to provide written, signed permission" and verification information, and that it may also verify the consumer directly. |
| Stirista, LLC | privacy@stirista.com | any state | delete | the opt-out process in Stirista's privacy policy, and any privacy law that applies to them | The page asks for the person's current address and email address, and any prior addresses they want opted out. It does not mention authorized agents. |
| RealSource, Inc | privacy@realsourcedata.com | any state | delete | the marketing-database opt-out in RealSource's privacy policy, and any privacy law that applies to them | The page does not mention authorized agents or identity verification. |
| DealMachine Operations Inc. | privacy@dealmachine.com | any state | delete, opt_out_sale | the privacy rights request process in DealMachine's privacy policy, and any privacy law that applies to them | The page asks for the subject line "Privacy Rights Request" and does not mention authorized agents. |
| RecruitBot | ccpa@recruitbot.com | CA | delete | the California Consumer Privacy Act (Cal. Civ. Code 1798.105) | The page says it will need to confirm the person's identity and California residency. It does not mention authorized agents. |
| Snovio Inc | help@snov.io | CA | delete, opt_out_sale | the California Consumer Privacy Act (Cal. Civ. Code 1798.105 and 1798.120) | The page directs authorized agents making a CCPA request for a California consumer to this address. |

Not used as a source: third-party guides, and the contact column of the California data broker registry.

## 6. Points I noticed that counsel should look at

These are observations from reading the code, not conclusions.

1. **Agent proof.** Whitepages' page says an agent "may need to provide written, signed permission". The email
   quotes the authorization text with its date and version, but attaches no signature or signed document. Whether
   that satisfies a company, or the California regulations on authorized agents, is open.
2. **Wording versus the screen.** The authorization says "the companies I select below", but on the page the
   companies are listed **above** the authorization. It also says the customer "can withdraw it ... by contacting
   VectorScrub", and there is no withdrawal button or API route. Withdrawal today would be a manual action.
3. **Three of six entries are offered to customers in any state** (no state restriction). For those the email
   cites the company's own policy "and any privacy law that applies to them", not a specific statute. Is that
   acceptable for a customer outside California?
4. **Opt-out of sale or sharing** is requested from companies whose page offers it, together with deletion.
   Counsel may want these as separate authorizations.
5. **Terms and Privacy Policy.** I found no text in `frontend/src/app/terms/page.tsx` or `privacy/page.tsx` that
   describes emailing companies as the customer's agent, or sharing name, email and address with them. Both pages
   cover opt-outs on broker sites. If this goes live, `TERMS_VERSION` (backend `models/customer.rs` and
   `frontend/src/lib/legal.ts`, now `2026-11-pilot-4`) must be bumped with them.
6. **Sender identity.** Mail goes out from VectorScrub's configured SMTP account. `docs/COMPLIANCE_CHECKLIST.md`
   lists the sender identity and postal address that must exist before mail is sent. Whether these emails need
   them is a question for counsel.
7. **Third-party data.** Companies may ask the customer directly to verify; VectorScrub reads no replies. The
   customer reports outcomes, and VectorScrub tells them only what it sent.
8. **Statements to avoid.** "A request is not a guarantee of removal" is in the authorization and on the page.
   Marketing must not say the feature removes data. It sends requests.
9. **Retention.** Authorization text, hash and request records are kept as long as the account exists and are
   removed on account erasure (`DELETE /me`). Check against `DATA_RETENTION.md` and the Privacy Policy.

## 7. Questions for counsel

1. Is the authorization text sufficient, in content and form, for VectorScrub to act as the customer's authorized
   agent under the CCPA and the laws of other states where the feature may be offered? Does a company require
   something signed?
2. Is it acceptable to cite only the company's own policy for entries with no state restriction?
3. Does sending these emails make VectorScrub a data broker, a service provider or a controller for anything it
   handles here, in any state?
4. What must the Terms and Privacy Policy add, and what must change in the authorization wording (point 2 above)?
5. Is a withdrawal mechanism required, and how fast?
6. Do the emails need sender or postal details, or any unsubscribe wording?
7. Should deletion and opt-out of sale or sharing be separate authorizations?
8. Are the retention periods acceptable?

## 8. After approval

Change the wording, bump `AUTHORIZATION_VERSION` (customers must accept the new version) and, if the Terms or
Privacy Policy change, `TERMS_VERSION` in both places. Run the backend tests and `python -m deletion.verify_channels`.
Only then set the two switches for a limited pilot. Update `docs/COMPLIANCE_CHECKLIST.md` and the steering file
(`.kiro/steering/vectorscrub.md`, "Deletion requests") so they stop saying the wording is a draft.

## 9. Files to hand over

- `backend/src/services/deletion.rs`, `backend/src/services/broker_channels.rs`, `backend/src/api/deletion.rs`
- `backend/data/broker_channels.json`
- `backend/migrations/0015_deletion_requests.sql`
- `agents/deletion/message.py`, `agents/deletion/sender.py`
- `frontend/src/app/dashboard/requests/page.tsx`
- `frontend/src/app/terms/page.tsx`, `frontend/src/app/privacy/page.tsx`
- `docs/COMPLIANCE_CHECKLIST.md`, `DATA_RETENTION.md`

## 10. Prioritized launch checklist (from counsel's two reviews, 2026-11-24)

**Both switches stay OFF** (`DELETION_REQUESTS_ENABLED`, `DELETION_REQUESTS_SEND_ENABLED`) until every Legal item
is ticked by counsel. A box is ticked only where code, a test or a recorded check shows it; the evidence is named.

### Legal (counsel decides; engineering cannot tick these)

| # | Priority | Item | State |
|---|---|---|---|
| L1 | Blocker | Is a typed-name electronic signature plus the stored, fingerprinted document "signed permission" under 11 CCR §7063, and does it meet ESIGN/UETA? If not, which mechanism? | Mechanism built (E1); **not approved** |
| L2 | Blocker | Approve the authorization text (`2026-11-draft-3`) and the signed-document layout (`services/deletion.rs`, `signed_document`) | Drafted; not approved |
| L3 | Blocker | Approve the Terms (sections 1 to 8) and Privacy Policy (sections 1 to 7, 9) at `TERMS_VERSION` `2026-11-pilot-6` | Drafted; not approved |
| L4 | Blocker | VectorGuard Labs (a Delaware corporation) as an authorized agent: California Secretary of State registration and any other agent requirement | Open |
| L5 | Blocker | Approve the California-only scope, and the legal basis each of the six emails cites. Three entries cite only the company's own policy "and any privacy law that applies"; for Californians, should they cite the CCPA? | Scope enforced (E5); citations open |
| L6 | Blocker | Approve the six-company catalog, per company: accepts agent requests? accepts email? signed permission required (Whitepages says it may be)? extra identification? which of delete / opt-out it supports? | Addresses re-checked (E8); the rest open |
| L7 | High | DROP: all six are in California's data broker registry (registry ids in the catalog), so all six must process DROP requests. Is the direct email additional coverage worth sending, and can VectorScrub act as a DROP agent (agent name, email and trade name disclosed through the consumer's DROP account)? | Page now says "use both"; legal question open |
| L8 | Medium | Statutory versus voluntary rights in Privacy section 9 | Reworded; not approved |

### Engineering

| # | Priority | Item | State and evidence |
|---|---|---|---|
| E1 | Blocker | Signed authorization: name, email, agent, each kind AUTHORIZED / NOT AUTHORIZED, companies and scope, time, version, e-sign consent, typed signature; downloadable PDF | [x] Migration 0027; test `each_campaign_is_signed_and_the_signed_document_is_kept_encrypted...`; refusals in `every_refusal_leaves_nothing_behind`. PDF is the browser's print-to-PDF of `/dashboard/requests/authorization/:id`, not a server-made file |
| E2 | Blocker | Withdrawal in the dashboard; cancels unsent requests; withdrawn authorization blocks future sends; timestamps; audit | [x] Tests `a_request_that_has_not_been_sent_can_be_withdrawn...`, `a_whole_campaign_can_be_withdrawn...`, `nothing_is_sent_under_a_withdrawn_authorization...` |
| E3 | Blocker | Separate permission for deletion and for opt-out of sale/sharing | [x] `request_kinds` (migration 0026), shown per kind in the signed document; tests `deleting_and_opting_out...`, agents `test_a_request_to_stop_selling_alone...` |
| E4 | Blocker | Interrupted-send recovery an operator can act on | [x] Message-ID recorded at claim; `vectorscrub deletion interrupted / resolve`; tests `an_interrupted_send_keeps_its_message_id...`, `...under_a_withdrawn_authorization_is_never_put_back`, agents `test_the_worker_claims_with_the_message_id...` |
| E5 | Blocker | California-only eligibility | [x] `DELETION_PILOT_STATES` default `CA`; tests `the_default_pilot_is_open_to_californians_only...`, config test |
| E6 | Blocker | End-to-end test with a real mail provider (send, bounce, interrupted send found in its logs) | [ ] Not done. Only unit and API tests with a fake SMTP |
| E7 | High | Delivery monitoring | [x] Counters `deletion_requests_sent_total`, `_failed_total`, `_interrupted_total`; alerts `DeletionRequestsNotSent`, `DeletionRequestSendInterrupted` (`promtool` passes). [ ] Not yet routed and tested end to end (`docs/runbooks/alerting.md`) |
| E8 | High | Catalog revalidated | [x] `python -m deletion.verify_channels`, 2026-11-24: all six `confirmed` (address still on the company's own page). `verified_on` dates left at 2026-11-12 for a person to update after reading each page (the tool never edits them). Stirista note now says previous addresses are not covered |
| E9 | High | Retention and erasure | [x] Test `deleting_the_account_erases_every_campaign_and_request...` (covers the signed copy, which is in the campaign row). Not covered: the mail provider's sent copies and 30-day backups. `vectorscrub verify-backup` does not yet trial-decrypt signed documents |
| E10 | Medium | Duplicate-send protection, SMTP failure handling, catalog validation, PII-free logs, outcome tracking, customer-visible history | [x] Existing tests (claim once, never retried, fixed-phrase errors, log tests, outcome tests) |
| E11 | Medium | Rate limiting | [x] API rate limits apply to these routes; 50 companies per campaign; 90-day cooldown per company; the worker spaces sends (`deletion_send_delay_seconds`). No per-day cap per customer |
| E12 | Medium | Emergency kill switch | [x] Off-by-default covered by tests. [ ] Not tried on a running deployment. Procedure: set `DELETION_REQUESTS_SEND_ENABLED=false` on the agents and restart them (nothing is claimed); for a full stop also set `DELETION_REQUESTS_ENABLED=false` on the backend (all routes 404, the queue is empty). Queued requests stay queued |
| E13 | High | Existing accounts accept updated Terms | [ ] No re-acceptance flow; only new registrations accept `pilot-6`. Every campaign does require the current authorization version |

### Launch (after Legal and Engineering)

| # | Item | State |
|---|---|---|
| R1 | Version numbers incremented | [x] `AUTHORIZATION_VERSION` `2026-11-draft-3`, `TERMS_VERSION` `2026-11-pilot-6` |
| R2 | California customers only, six approved companies only | Enforced for state (E5); catalog approval is L6 |
| R3 | Customer-facing limitations published | Drafted (requests page, Terms, Privacy); publish after L2 and L3 |
| R4 | Marketing wording: "automated privacy-request submission and tracking", never "guaranteed removal" | No marketing copy for this feature exists yet; the steering file forbids describing it as live |
| R5 | Pilot monitoring, manual review of exceptions, escalation to a named person | Alerts exist (E7); a named person and a review routine are open (`docs/runbooks/alerting.md`) |
