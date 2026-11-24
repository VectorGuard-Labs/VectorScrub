# Deletion requests: material for legal review

Status: **DRAFT, for review by counsel. The feature is switched off and no email has been sent to any company.**
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
